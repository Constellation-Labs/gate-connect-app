//! Custom hudsucker [`CertificateAuthority`] that mints per-host leaf certs
//! **with the `serverAuth` extended-key-usage extension** (and key usages).
//!
//! hudsucker's built-in `RcgenAuthority` omits EKU on its leaves. Lenient TLS
//! stacks (OpenSSL/LibreSSL, i.e. curl) accept that, but macOS's
//! Network.framework — which Claude Desktop / Cowork use — rejects a leaf
//! without `serverAuth` EKU and aborts the handshake with a `BadCertificate`
//! alert. That asymmetry is exactly what blocked Cowork: the request never
//! survived the MITM handshake. Adding the EKU makes Apple's stack accept the
//! cert. Otherwise this mirrors `RcgenAuthority`: per-host leaf, ~1-year
//! validity, `h2` + `http/1.1` ALPN, cached per host.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use hudsucker::certificate_authority::CertificateAuthority;
use hudsucker::rcgen::{
    string::Ia5String, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    IsCa, Issuer, KeyPair, KeyUsagePurpose, SanType,
};
use hudsucker::rustls::{
    crypto::CryptoProvider,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    ServerConfig,
};
use http::uri::Authority;
use time::{Duration, OffsetDateTime};

const LEAF_TTL_SECS: i64 = 365 * 24 * 60 * 60;
const NOT_BEFORE_OFFSET_SECS: i64 = 60;

pub struct GateCa {
    issuer: Issuer<'static, KeyPair>,
    /// The key pair all minted leaves certify — distinct from the CA key.
    leaf_key: KeyPair,
    private_key: PrivateKeyDer<'static>,
    provider: Arc<CryptoProvider>,
    cache: Mutex<HashMap<String, Arc<ServerConfig>>>,
}

impl GateCa {
    pub fn new(issuer: Issuer<'static, KeyPair>, provider: CryptoProvider) -> Self {
        // Leaves get their OWN key pair, distinct from the CA. hudsucker's
        // RcgenAuthority reuses the CA key for every leaf, which makes a
        // leaf's SubjectKeyIdentifier identical to its issuer's — Apple's TLS
        // stack rejects that with CertificateUnknown. Giving leaves a separate
        // key (what real CAs do) is the actual fix; the EKU / AKI / basic
        // constraints above were necessary but not sufficient.
        let leaf_key = KeyPair::generate().expect("failed to generate leaf key pair");
        let private_key =
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        Self {
            issuer,
            leaf_key,
            private_key,
            provider: Arc::new(provider),
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn gen_cert(&self, host: &str) -> CertificateDer<'static> {
        let mut params = CertificateParams::default();

        let not_before = OffsetDateTime::now_utc() - Duration::seconds(NOT_BEFORE_OFFSET_SECS);
        params.not_before = not_before;
        params.not_after = not_before + Duration::seconds(LEAF_TTL_SECS);

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

        params
            .signed_by(&self.leaf_key, &self.issuer)
            .expect("failed to sign leaf certificate")
            .into()
    }

    fn build_server_config(&self, host: &str) -> Arc<ServerConfig> {
        let certs = vec![self.gen_cert(host)];
        let mut cfg = ServerConfig::builder_with_provider(Arc::clone(&self.provider))
            .with_safe_default_protocol_versions()
            .expect("failed to set protocol versions")
            .with_no_client_auth()
            .with_single_cert(certs, self.private_key.clone_key())
            .expect("failed to build ServerConfig");
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Arc::new(cfg)
    }
}

impl CertificateAuthority for GateCa {
    async fn gen_server_config(&self, authority: &Authority) -> Arc<ServerConfig> {
        let host = authority.host().to_owned();
        if let Some(cfg) = self.cache.lock().expect("cert cache mutex poisoned").get(&host) {
            return Arc::clone(cfg);
        }
        let cfg = self.build_server_config(&host);
        self.cache
            .lock()
            .expect("cert cache mutex poisoned")
            .insert(host, Arc::clone(&cfg));
        cfg
    }
}
