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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use http::uri::Authority;
use hudsucker::certificate_authority::CertificateAuthority;
use hudsucker::rcgen::{
    string::Ia5String, BasicConstraints, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    NameConstraints, SanType,
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

/// Subject CN of the local root CA - shared by the three platform CA
/// modules, which also use it as the lookup key for trust/untrust.
pub(crate) const CA_COMMON_NAME: &str = "Gate Connect Local CA";

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
///  2. `load_or_create` short-circuits whenever a key + cert already exist and
///     does NOT compare the on-disk constraints against the current catalog.
///     An install that enabled the proxy BEFORE a host was added therefore
///     keeps a root that cannot mint for it, and interception of that host
///     fails at the handshake with no obvious cause. Until that drift is
///     detected automatically, a newly added host needs `proxy untrust-ca`
///     followed by `proxy enable` to take effect on an existing install.
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
    cache: Mutex<HashMap<String, CachedLeaf>>,
}

/// A cached per-host leaf TLS config plus the leaf's expiry, so the cache can
/// regenerate entries before they go stale instead of serving them forever.
struct CachedLeaf {
    not_after: OffsetDateTime,
    config: Arc<ServerConfig>,
}

impl GateCa {
    pub fn new(issuer: Issuer<'static, KeyPair>, provider: CryptoProvider) -> Self {
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
