//! Custom hudsucker [`CertificateAuthority`] that mints per-host leaf certs
//! **with the `serverAuth` extended-key-usage extension** (and key usages).
//!
//! hudsucker's built-in `RcgenAuthority` omits EKU on its leaves. Lenient TLS
//! stacks (OpenSSL/LibreSSL, i.e. curl) accept that, but macOS's
//! Network.framework - which Claude Desktop / Cowork use - rejects a leaf
//! without `serverAuth` EKU and aborts the handshake with a `BadCertificate`
//! alert. That asymmetry is exactly what blocked Cowork: the request never
//! survived the MITM handshake. Adding the EKU makes Apple's stack accept the
//! cert. Otherwise this mirrors `RcgenAuthority`: per-host leaf, ~1-year
//! validity, `h2` + `http/1.1` ALPN, cached per host.
//!
//! The same asymmetry bit again from the other side on Windows, and the second
//! fix lives here too. schannel asks CryptoAPI to check revocation for the whole
//! chain, and the clients that do not opt out of that (notably the `curl.exe` in
//! System32) reject a leaf carrying no CRL distribution point outright, with
//! `CRYPT_E_NO_REVOCATION_CHECK`. OpenSSL and rustls never look. So on Windows
//! the leaves advertise a distribution point on the engine's own loopback
//! listener and [`sign_empty_crl`] answers it. Only the leaf changed: the root
//! needs no revocation data, which is what keeps existing installs from having
//! to re-trust a new one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use http::uri::Authority;
use hudsucker::certificate_authority::CertificateAuthority;
use hudsucker::rcgen::{
    string::Ia5String, BasicConstraints, CertificateParams, CrlDistributionPoint,
    DistinguishedName, DnType, ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, NameConstraints, SanType,
};
use hudsucker::rustls::{
    crypto::CryptoProvider,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    ServerConfig,
};
use time::{Duration, OffsetDateTime};

const LEAF_TTL_SECS: i64 = 365 * 24 * 60 * 60;
const NOT_BEFORE_OFFSET_SECS: i64 = 60;
/// Regenerate a cached leaf once it's within this window of expiry. The cache
/// is keyed by host and otherwise never evicts, so without this a long-running
/// engine would keep serving a leaf past its `not_after` and break handshakes.
const LEAF_RENEW_MARGIN_SECS: i64 = 7 * 24 * 60 * 60;

/// Path the CRL is served from, on the same loopback listener that serves the
/// PAC script. Shared so the URL baked into a leaf's CRL distribution point and
/// the route the listener answers on cannot drift apart.
///
/// Gated to the platforms that have that listener, plus `test` so the CRL is
/// still exercised on a Linux CI box where none of this is compiled otherwise.
#[cfg(any(target_os = "windows", target_os = "macos", test))]
pub(crate) const CRL_PATH: &str = "/gate-ca.crl";

/// How far ahead a freshly signed CRL declares its `nextUpdate`.
///
/// Windows caches a fetched CRL and will not re-fetch until this passes, so it
/// trades request volume against how long a stale answer could be believed.
/// Neither side of that trade matters much here: this CA revokes nothing, and
/// the CRL is regenerated per request rather than cached by us, so a week is
/// simply long enough that the endpoint is hit rarely.
#[cfg(any(target_os = "windows", target_os = "macos", test))]
const CRL_NEXT_UPDATE_SECS: i64 = 7 * 24 * 60 * 60;

/// How far back a freshly signed CRL declares its `thisUpdate`, absorbing clock
/// skew between us and the validating client for the same reason
/// [`NOT_BEFORE_OFFSET_SECS`] does on a leaf.
#[cfg(any(target_os = "windows", target_os = "macos", test))]
const CRL_THIS_UPDATE_BACKDATE_SECS: i64 = 60 * 60;

/// DER of a freshly signed, empty CRL for the local CA.
///
/// Empty is not a placeholder: this CA mints only short-lived per-host leaves
/// held in memory by the engine that signed them, so there is nothing to revoke
/// and no mechanism by which there could be. The CRL exists to be *fetchable*,
/// not to carry entries.
///
/// That sounds pointless until you look at what Windows does without it.
/// schannel asks CryptoAPI for `CERT_CHAIN_REVOCATION_CHECK_CHAIN` and, for
/// clients that do not pass `SCH_CRED_IGNORE_NO_REVOCATION_CHECK` (curl's
/// default, i.e. the `curl.exe` in System32), a leaf carrying no CRL
/// distribution point fails the handshake outright with
/// `CRYPT_E_NO_REVOCATION_CHECK (0x80092012)` - "the revocation function was
/// unable to check revocation". Not a warning; the connection dies. OpenSSL,
/// rustls and BoringSSL clients never notice, which is why this only ever
/// showed up as "curl stopped working while the proxy is on".
///
/// So the leaves advertise a distribution point pointing back at our own
/// loopback listener ([`CRL_PATH`]) and this function answers it. Measured on
/// Windows 11 with curl 8.21.0 (schannel): a leaf with no CDP fails as above, a
/// leaf whose CDP resolves to this CRL completes the handshake, and CryptoAPI
/// really does issue a plain-HTTP GET to `127.0.0.1` mid-handshake to get it.
///
/// The root deliberately carries no CDP of its own, and does not need one -
/// tested on the same box: a chain whose leaf has a distribution point and
/// whose self-signed root has none validates fine, because CryptoAPI does not
/// demand revocation data for a trust anchor. That is what keeps this fix
/// leaf-only, and therefore invisible to already-trusted installs: changing the
/// root would mean regenerating it and walking every existing user back through
/// the trust dialog.
#[cfg(any(target_os = "windows", target_os = "macos", test))]
pub(crate) fn sign_empty_crl(issuer: &Issuer<'_, KeyPair>) -> Result<Vec<u8>> {
    use hudsucker::rcgen::{CertificateRevocationListParams, KeyIdMethod, SerialNumber};

    let now = OffsetDateTime::now_utc();
    let crl = CertificateRevocationListParams {
        this_update: now - Duration::seconds(CRL_THIS_UPDATE_BACKDATE_SECS),
        next_update: now + Duration::seconds(CRL_NEXT_UPDATE_SECS),
        // Fixed rather than counted: a monotonic sequence would have to survive
        // engine restarts to mean anything, and nothing consuming this CRL
        // compares numbers across fetches. One CRL, one number.
        crl_number: SerialNumber::from(1u64),
        issuing_distribution_point: None,
        revoked_certs: Vec::new(),
        key_identifier_method: KeyIdMethod::Sha256,
    }
    .signed_by(issuer)
    .context("signing the CA's CRL")?;
    Ok(crl.der().as_ref().to_vec())
}

/// Subject CN of the local root CA - shared by the three platform CA
/// modules, which also use it as the lookup key for trust/untrust.
pub(crate) const CA_COMMON_NAME: &str = "Gate Connect Local CA";

/// Fingerprint of the host set the root CA was minted for.
///
/// The CA's X.509 name constraints are built from the WHOLE domain catalog at
/// generation time, so a root minted before a host was added cannot issue for it
/// and interception of that host fails at the handshake with nothing naming the
/// cause. `load_or_create` is presence-based — it returns whatever key + cert are
/// on disk without examining them — so the drift is invisible to it.
///
/// Rather than parse the stored certificate's constraints back out (an X.509
/// dependency, for a value we already know), each platform persists this
/// fingerprint beside its cert and compares on load. A mismatch, or a missing
/// file on a pre-existing install, means regenerate.
///
/// Hashed rather than stored verbatim so the file stays fixed-size and carries no
/// meaning worth editing by hand. Sorted + deduped + lowercased so the value
/// tracks the host SET and not the catalog's declaration order — reordering
/// entries must not force a re-trust on every user.
pub(crate) fn catalog_host_fingerprint() -> String {
    let hosts: Vec<String> = crate::proxy::default_domains()
        .iter()
        .flat_map(|d| d.hosts.iter())
        .cloned()
        .collect();
    fingerprint_hosts(&hosts)
}

/// The hash itself, over an explicit host list so the normalisation is testable
/// without reaching into the catalog.
fn fingerprint_hosts(hosts: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut normalised: Vec<String> = hosts
        .iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .collect();
    normalised.sort();
    normalised.dedup();
    let mut hasher = Sha256::new();
    for h in &normalised {
        hasher.update(h.as_bytes());
        // NUL-delimited so ["ab","c"] and ["a","bc"] cannot hash alike.
        hasher.update([0u8]);
    }
    format!("{:x}", hasher.finalize())
}

/// Path of the fingerprint sidecar for a given CA cert path.
pub(crate) fn host_fingerprint_path(cert_path: &std::path::Path) -> std::path::PathBuf {
    cert_path.with_extension("hosts")
}

/// True when the on-disk fingerprint matches the current catalog — i.e. the
/// stored CA can still mint for every host we route.
///
/// A missing or unreadable sidecar reads as STALE, which is the safe direction:
/// every install predating this check has no sidecar and needs the regeneration
/// exactly once. A false "stale" costs one regeneration + re-trust; a false
/// "fresh" leaves a CA that cannot serve the catalog.
pub(crate) fn host_fingerprint_is_current(cert_path: &std::path::Path) -> bool {
    match std::fs::read_to_string(host_fingerprint_path(cert_path)) {
        Ok(stored) => stored.trim() == catalog_host_fingerprint(),
        Err(_) => false,
    }
}

/// Persist the current catalog fingerprint beside a freshly generated cert.
///
/// Written AFTER the cert so an interrupted sequence leaves the sidecar absent
/// or stale, never newer than the cert it describes — the same self-healing
/// ordering the platform modules already use for key-then-cert.
pub(crate) fn write_host_fingerprint(cert_path: &std::path::Path) -> Result<()> {
    let path = host_fingerprint_path(cert_path);
    let tmp = path.with_extension("hosts.tmp");
    std::fs::write(&tmp, catalog_host_fingerprint())
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Certificate parameters for the local root CA, shared by the three
/// platform CA modules so the security-critical extensions cannot drift.
///
/// The CA carries X.509 Name Constraints permitting only the hosts in the
/// built-in domain catalog: even as a fully trusted root it cannot mint
/// acceptable certs for any other domain, shrinking the blast radius of a
/// CA-key compromise to the providers the proxy actually intercepts.
/// Adding a host to the catalog later requires regenerating the CA
/// (`load_or_create` re-trusts a regenerated pair).
///
/// Two consequences of that, both easy to miss:
///
///  1. The subtree list is built from the WHOLE catalog, not from the enabled
///     entries, so shipping a new domain widens what this root may mint for
///     every user — including those who never turn the domain on. Weigh that
///     when adding a host: `claude-web` puts `claude.ai` in here, i.e. the
///     surface holding the user's Claude session cookie.
///  2. `load_or_create` short-circuits whenever a key + cert already exist, and
///     the stored certificate's constraints are never parsed back out — so the
///     drift is invisible to it. That is what [`catalog_host_fingerprint`]
///     and its sidecar exist for: the host set is fingerprinted at generation
///     time and compared on every load, so adding a host regenerates the root
///     automatically on next launch instead of failing the handshake with no
///     obvious cause. No manual `proxy untrust-ca` is required.
pub(crate) fn ca_certificate_params() -> Result<CertificateParams> {
    let mut params =
        CertificateParams::new(Vec::<String>::new()).context("building CA certificate params")?;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(DnType::CommonName, CA_COMMON_NAME);
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Constellation Gate");
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    params.name_constraints = Some(NameConstraints {
        permitted_subtrees: crate::proxy::default_domains()
            .iter()
            .flat_map(|d| d.hosts.iter())
            .map(|h| GeneralSubtree::DnsName(h.clone()))
            .collect(),
        excluded_subtrees: Vec::new(),
    });
    Ok(params)
}

pub struct GateCa {
    issuer: Issuer<'static, KeyPair>,
    /// The key pair all minted leaves certify - distinct from the CA key.
    leaf_key: KeyPair,
    private_key: PrivateKeyDer<'static>,
    provider: Arc<CryptoProvider>,
    /// URL to advertise as the leaves' CRL distribution point, or `None` to emit
    /// no such extension. Set only where a client actually hard-fails without
    /// one (Windows; see [`sign_empty_crl`]), because the URL names a loopback
    /// port that has to be serving [`CRL_PATH`] for the whole run - an
    /// advertised CDP that cannot be fetched is *worse* than none, turning
    /// `CRYPT_E_NO_REVOCATION_CHECK` into `CRYPT_E_REVOCATION_OFFLINE`, which
    /// the same clients also refuse.
    crl_url: Option<String>,
    cache: Mutex<HashMap<String, CachedLeaf>>,
}

/// A cached per-host leaf TLS config plus the leaf's expiry, so the cache can
/// regenerate entries before they go stale instead of serving them forever.
struct CachedLeaf {
    not_after: OffsetDateTime,
    config: Arc<ServerConfig>,
}

impl GateCa {
    pub fn new(
        issuer: Issuer<'static, KeyPair>,
        provider: CryptoProvider,
        crl_url: Option<String>,
    ) -> Self {
        // Leaves get their OWN key pair, distinct from the CA. hudsucker's
        // RcgenAuthority reuses the CA key for every leaf, which makes a
        // leaf's SubjectKeyIdentifier identical to its issuer's - Apple's TLS
        // stack rejects that with CertificateUnknown. Giving leaves a separate
        // key (what real CAs do) is the actual fix; the EKU / AKI / basic
        // constraints above were necessary but not sufficient.
        let leaf_key = KeyPair::generate().expect("failed to generate leaf key pair");
        let private_key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        Self {
            issuer,
            leaf_key,
            private_key,
            provider: Arc::new(provider),
            crl_url,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn gen_cert(&self, host: &str) -> (CertificateDer<'static>, OffsetDateTime) {
        let mut params = CertificateParams::default();

        let not_before = OffsetDateTime::now_utc() - Duration::seconds(NOT_BEFORE_OFFSET_SECS);
        params.not_before = not_before;
        let not_after = not_before + Duration::seconds(LEAF_TTL_SECS);
        params.not_after = not_after;

        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, host);
        params.distinguished_name = dn;

        params.subject_alt_names.push(SanType::DnsName(
            Ia5String::try_from(host).expect("host is not a valid DNS name"),
        ));

        // The fix: macOS's TLS stack requires the leaf to declare serverAuth.
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyEncipherment,
        ];
        // Explicit Basic Constraints (CA:FALSE) + an Authority Key Identifier
        // linking the leaf to the CA's Subject Key Identifier. Apple's chain
        // builder needs the AKI to tie the leaf to the trusted root; without
        // these it rejects the leaf with a CertificateUnknown alert even
        // though serverAuth is present.
        params.is_ca = IsCa::ExplicitNoCa;
        params.use_authority_key_identifier_extension = true;
        // Windows needs somewhere to go looking for revocation status or it
        // refuses the handshake outright; see `sign_empty_crl` for the whole
        // story. Absent on platforms whose clients soft-fail, so they gain no
        // in-handshake HTTP fetch they never needed.
        if let Some(url) = &self.crl_url {
            params.crl_distribution_points = vec![CrlDistributionPoint {
                uris: vec![url.clone()],
            }];
        }

        let der: CertificateDer<'static> = params
            .signed_by(&self.leaf_key, &self.issuer)
            .expect("failed to sign leaf certificate")
            .into();
        (der, not_after)
    }

    fn build_server_config(&self, host: &str) -> (Arc<ServerConfig>, OffsetDateTime) {
        let (cert, not_after) = self.gen_cert(host);
        let certs = vec![cert];
        let mut cfg = ServerConfig::builder_with_provider(Arc::clone(&self.provider))
            .with_safe_default_protocol_versions()
            .expect("failed to set protocol versions")
            .with_no_client_auth()
            .with_single_cert(certs, self.private_key.clone_key())
            .expect("failed to build ServerConfig");
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        (Arc::new(cfg), not_after)
    }
}

impl CertificateAuthority for GateCa {
    async fn gen_server_config(&self, authority: &Authority) -> Arc<ServerConfig> {
        let host = authority.host().to_owned();
        let renew_margin = Duration::seconds(LEAF_RENEW_MARGIN_SECS);
        {
            let cache = self.cache.lock().expect("cert cache mutex poisoned");
            if let Some(entry) = cache.get(&host) {
                // Serve the cached leaf unless it's within the renew margin of
                // expiry - past that we drop through and mint a fresh one so a
                // long-lived engine never hands out an expired cert.
                if entry.not_after - OffsetDateTime::now_utc() > renew_margin {
                    return Arc::clone(&entry.config);
                }
            }
        }
        let (cfg, not_after) = self.build_server_config(&host);
        self.cache
            .lock()
            .expect("cert cache mutex poisoned")
            .insert(
                host,
                CachedLeaf {
                    not_after,
                    config: Arc::clone(&cfg),
                },
            );
        cfg
    }
}

#[cfg(test)]
mod fingerprint_tests {
    use super::*;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn fingerprint_tracks_the_host_set_not_the_declaration_order() {
        // Reordering catalog entries must NOT look like drift — otherwise every
        // cosmetic reshuffle forces a CA regeneration and re-trust on every user.
        assert_eq!(
            fingerprint_hosts(&v(&["claude.ai", "chatgpt.com"])),
            fingerprint_hosts(&v(&["chatgpt.com", "claude.ai"]))
        );
    }

    #[test]
    fn fingerprint_ignores_case_and_duplicates() {
        // Two entries legitimately share a host (the chatgpt.com relay + MITM
        // pair), so a duplicate must not read as a different set.
        assert_eq!(
            fingerprint_hosts(&v(&["chatgpt.com"])),
            fingerprint_hosts(&v(&["ChatGPT.com", "chatgpt.com"]))
        );
    }

    #[test]
    fn fingerprint_changes_when_a_host_is_added() {
        // The whole point: adding a host must invalidate an existing CA.
        assert_ne!(
            fingerprint_hosts(&v(&["api.anthropic.com"])),
            fingerprint_hosts(&v(&["api.anthropic.com", "claude.ai"]))
        );
    }

    #[test]
    fn fingerprint_is_delimited_so_concatenations_cannot_collide() {
        assert_ne!(
            fingerprint_hosts(&v(&["ab", "c"])),
            fingerprint_hosts(&v(&["a", "bc"]))
        );
    }

    #[test]
    fn current_catalog_fingerprint_is_stable_and_covers_the_catalog() {
        assert_eq!(catalog_host_fingerprint(), catalog_host_fingerprint());
        let hosts: Vec<String> = crate::proxy::default_domains()
            .iter()
            .flat_map(|d| d.hosts.iter())
            .cloned()
            .collect();
        assert_eq!(catalog_host_fingerprint(), fingerprint_hosts(&hosts));
    }

    #[test]
    fn a_missing_sidecar_reads_as_stale() {
        // Every install predating this check has no sidecar, and must regenerate
        // exactly once. Erring toward stale costs a regeneration; erring toward
        // fresh leaves a CA that cannot serve the catalog.
        let dir = std::env::temp_dir().join(format!("gate-ca-fp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cert = dir.join("ca-cert.pem");
        assert!(!host_fingerprint_is_current(&cert));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn writing_the_sidecar_makes_it_current_and_a_wrong_value_does_not() {
        let dir = std::env::temp_dir().join(format!("gate-ca-fp-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cert = dir.join("ca-cert.pem");

        write_host_fingerprint(&cert).unwrap();
        assert!(host_fingerprint_is_current(&cert));
        // Sidecar sits beside the cert, not on top of it.
        assert_eq!(host_fingerprint_path(&cert), dir.join("ca-cert.hosts"));

        std::fs::write(host_fingerprint_path(&cert), "not-the-fingerprint").unwrap();
        assert!(!host_fingerprint_is_current(&cert));

        std::fs::remove_dir_all(&dir).ok();
    }
}

/// Tests for the Windows revocation fix: the leaves' CRL distribution point and
/// the CRL that has to be fetchable from it.
///
/// The bug these guard against is silent on every platform CI runs on. A leaf
/// with no CDP is accepted by OpenSSL, rustls and BoringSSL without comment and
/// rejected by schannel with `CRYPT_E_NO_REVOCATION_CHECK`, so nothing short of
/// asserting on the emitted extension will catch a regression here.
#[cfg(test)]
mod crl_tests {
    use super::*;

    /// A CA built exactly the way the platform modules build it, so what these
    /// tests sign with is what ships.
    fn test_ca(crl_url: Option<String>) -> (GateCa, String) {
        let params = ca_certificate_params().expect("CA params");
        let key = KeyPair::generate().expect("CA key");
        let cert_pem = params.self_signed(&key).expect("self-signed CA").pem();
        let issuer = Issuer::from_ca_cert_pem(
            &cert_pem,
            KeyPair::from_pem(&key.serialize_pem()).expect("reparse CA key"),
        )
        .expect("issuer");
        let ca = GateCa::new(
            issuer,
            hudsucker::rustls::crypto::aws_lc_rs::default_provider(),
            crl_url,
        );
        (ca, cert_pem)
    }

    /// The CA's own key usages must permit CRL signing, or `sign_empty_crl`
    /// fails at runtime on Windows and every handshake there dies with an
    /// unfetchable distribution point. rcgen enforces this, so assert the
    /// shipping params satisfy it rather than discovering it in the field.
    #[test]
    fn ca_params_permit_crl_signing() {
        let params = ca_certificate_params().unwrap();
        assert!(
            params.key_usages.contains(&KeyUsagePurpose::CrlSign),
            "the CA must keep CrlSign or it cannot sign the CRL its leaves point at"
        );
    }

    /// A successful sign is itself the assertion on the validity window: rcgen
    /// rejects `next_update <= this_update` with `InvalidCrlNextUpdate`, so this
    /// passing means the window is ordered and forward-looking.
    #[test]
    fn signed_crl_is_parseable_and_not_expired() {
        let (_ca, cert_pem) = test_ca(None);
        let key = KeyPair::generate().unwrap();
        // Sign with an issuer over the same params so the CRL is well-formed;
        // the assertions below are about the CRL body, not the chain.
        let issuer = Issuer::new(ca_certificate_params().unwrap(), key);
        let der = sign_empty_crl(&issuer).expect("signing an empty CRL");
        assert!(!der.is_empty(), "CRL DER should not be empty");
        // A DER SEQUENCE, i.e. it is at least shaped like a CRL rather than an
        // error string that happened to serialise.
        assert_eq!(der[0], 0x30, "CRL should be a DER SEQUENCE");
        assert!(
            cert_pem.contains("BEGIN CERTIFICATE"),
            "sanity: the CA cert should be PEM"
        );
    }

    #[test]
    fn leaf_carries_the_distribution_point_when_a_url_is_configured() {
        let url = format!("http://127.0.0.1:47123{CRL_PATH}");
        let (ca, _) = test_ca(Some(url.clone()));
        let (der, _) = ca.gen_cert("api.anthropic.com");
        // The URL is an IA5String in the DER, so a byte search is enough to
        // prove the extension carries it - and it cannot appear by accident.
        let needle = url.as_bytes();
        assert!(
            der.as_ref().windows(needle.len()).any(|w| w == needle),
            "leaf should carry the configured CRL distribution point URL"
        );
    }

    /// The other half of the switch: platforms whose clients soft-fail must not
    /// gain an in-handshake HTTP fetch, so no URL means no extension.
    #[test]
    fn leaf_omits_the_distribution_point_when_no_url_is_configured() {
        let (ca, _) = test_ca(None);
        let (der, _) = ca.gen_cert("api.anthropic.com");
        let needle = CRL_PATH.as_bytes();
        assert!(
            !der.as_ref().windows(needle.len()).any(|w| w == needle),
            "leaf must not advertise a CRL distribution point when none is set"
        );
    }
}
