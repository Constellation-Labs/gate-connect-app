//! Hermetic test for `proxy::ca::load` on Linux: the read the session loop's
//! 30s token push makes, which must never mint a CA.
//!
//! It used to be `load_or_create`, which regenerates the pair whenever a half is
//! missing or the halves disagree. Only the enable and trust paths follow that
//! with `ensure_trusted`, so a root minted on the tick reached the daemon
//! untrusted, and every intercepted host failed its handshake while the rows
//! read Protected. `proxy untrust-ca` removing the material, or another process
//! caught between writing the two halves, was enough.
//!
//! Backed by the file seam (`GATE_CONNECT_TEST_SECRETS`) and a redirected home,
//! so neither the OS secret store nor the real CA material is touched. One test
//! function, because both seams are process-global env vars.

#![cfg(target_os = "linux")]

use gate_connect_core::proxy::ca;

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gate-connect-ca-load-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Point the material at `dir`, and return where its certificate lives.
fn use_home(dir: &std::path::Path) -> std::path::PathBuf {
    std::env::set_var("GATE_CONNECT_TEST_SECRETS", dir.join("secrets"));
    std::env::set_var("GATE_CONNECT_TEST_HOME", dir.join("home"));
    gate_connect_core::env::ca_material_dir()
        .expect("CA material dir")
        .join("ca-cert.pem")
}

#[test]
fn load_reads_the_existing_pair_and_never_mints_one() {
    let dir = temp_dir("main");
    let cert_path = use_home(&dir);

    // Nothing on disk: the tick's read fails rather than minting a root that
    // nothing would trust.
    assert!(ca::load().is_err(), "no material must be an error");
    assert!(
        !cert_path.exists(),
        "load must not write a certificate when there is none"
    );

    // The enable path mints, and the tick then re-sends exactly that pair.
    let minted = ca::load_or_create().expect("mint");
    let loaded = ca::load().expect("load the minted pair");
    assert_eq!(loaded.cert_pem(), minted.cert_pem());

    // A key from a different pair, as another process caught between its two
    // writes would leave it: an error, with the certificate left alone.
    let other = temp_dir("other");
    use_home(&other);
    let foreign_cert = ca::load_or_create()
        .expect("mint a second pair")
        .cert_pem()
        .to_string();
    use_home(&dir);
    std::fs::write(&cert_path, &foreign_cert).expect("swap in a foreign certificate");
    assert!(ca::load().is_err(), "a mismatched pair must be an error");
    assert_eq!(
        std::fs::read_to_string(&cert_path).expect("cert still there"),
        foreign_cert,
        "load must not regenerate over a mismatched pair"
    );

    // Removed material, as `proxy untrust-ca` leaves it: still no mint.
    std::fs::remove_file(&cert_path).expect("remove the certificate");
    assert!(
        ca::load().is_err(),
        "a missing certificate must be an error"
    );
    assert!(
        !cert_path.exists(),
        "load must not re-mint removed material"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&other);
}
