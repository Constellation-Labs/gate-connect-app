//! A synthesized CA bundle: the platform's own trust roots with Gate's local
//! CA appended.
//!
//! For tools that accept a single CA *file* rather than an extra CA. Node's
//! `NODE_EXTRA_CA_CERTS` appends to its built-in roots, so it only ever needs
//! [`super::ca_cert_path`]. Python's is the opposite:
//! `ssl.create_default_context(cafile=…)` skips `load_default_certs()`
//! entirely, so whatever file is named becomes the *complete* trust store.
//! Pointing such a tool at our one-cert PEM would make every unrelated HTTPS
//! call it makes fail, so it has to be handed a full bundle instead.
//!
//! Why this is needed at all when the CA is already in the OS trust store:
//! stdlib Python does read that store, but a `requests` / `httpx` client backed
//! by a **pip-installed** certifi uses certifi's vendored bundle instead.
//! Measured: Debian's apt `python3-certifi` is patched to return
//! `/etc/ssl/certs/ca-certificates.crt`, but a venv - which is how Hermes
//! installs (`setup-hermes.sh` runs `uv venv` then pip) - returns its own
//! `site-packages/certifi/cacert.pem`, which knows nothing about the CA.
//!
//! The bundle is regenerated on each connect rather than cached indefinitely:
//! the platform roots underneath it move (a distro `ca-certificates` upgrade, an
//! OS update), and a snapshot that silently went stale would keep trusting roots
//! the platform has since distrusted.

use anyhow::{Context, Result};
use std::path::PathBuf;

/// Where the synthesized bundle lives - beside the CA it is built from.
pub fn path() -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?
        .join("proxy")
        .join("ca-bundle.pem"))
}

/// Platform trust roots in PEM form, as the base to append our CA to.
///
/// Linux and macOS both ship a concatenated PEM at a well-known path. On Linux
/// that file already contains our CA (`update-ca-certificates` regenerates it
/// from `/usr/local/share/ca-certificates`), so the append is a harmless
/// duplicate there rather than the load-bearing part - keeping one code path
/// costs nothing and means we don't depend on the refresh having run.
#[cfg(not(target_os = "windows"))]
fn system_roots_pem() -> Result<String> {
    // Debian/Ubuntu/Arch first, then RHEL/Fedora/SUSE, then macOS. Ordered by
    // which platforms actually ship each path so the common case is one stat.
    const CANDIDATES: &[&str] = &[
        "/etc/ssl/certs/ca-certificates.crt",
        "/etc/pki/tls/certs/ca-bundle.crt",
        "/etc/ssl/ca-bundle.pem",
        "/etc/ssl/cert.pem",
    ];
    for candidate in CANDIDATES {
        let p = std::path::Path::new(candidate);
        if p.is_file() {
            return std::fs::read_to_string(p)
                .with_context(|| format!("reading system CA bundle {candidate}"));
        }
    }
    anyhow::bail!(
        "no system CA bundle found (looked in {}) -- Gate cannot build a trust bundle for tools \
         that need one",
        CANDIDATES.join(", ")
    )
}

#[cfg(target_os = "windows")]
fn system_roots_pem() -> Result<String> {
    use std::process::Command;

    // Windows keeps its roots in the certificate store rather than a PEM file,
    // so export them. `Root` is the machine-wide trusted-root store, which is
    // also where our own CA is installed - a duplicate in the bundle is
    // harmless, so no attempt is made to filter it out.
    //
    // Opened through .NET rather than the `Cert:` PSDrive, which looks like the
    // obvious spelling and is a trap. That drive is provided by the
    // `Microsoft.PowerShell.Security` module, so using it depends on module
    // AUTOLOADING, which follows `PSModulePath` - and this process does not
    // control that variable. Spawning from a PowerShell 7 parent is enough to
    // break it: the child `powershell` (Windows PowerShell 5.1) inherits pwsh's
    // `PSModulePath`, which names the 7.x module tree rather than 5.1's, the
    // module never loads, and the export dies with "Cannot find drive. A drive
    // with the name 'Cert' does not exist". Measured on CI, where the test
    // process is a child of the runner's pwsh.
    //
    // Nothing below needs a module: `X509Store` and `[Convert]` are .NET types
    // reached through the language itself, `::new()` is language syntax (not
    // `New-Object`, which lives in Microsoft.PowerShell.Utility), and `foreach`
    // is a keyword (not `ForEach-Object`). So there is no autoload to fail.
    let script = "$store = [System.Security.Cryptography.X509Certificates.X509Store]::new('Root','LocalMachine'); \
         $store.Open('ReadOnly'); \
         foreach ($c in $store.Certificates) { \
           '-----BEGIN CERTIFICATE-----'; \
           [Convert]::ToBase64String($c.RawData, 'InsertLineBreaks'); \
           '-----END CERTIFICATE-----' \
         }; \
         $store.Close()";
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .context("running powershell to export the Windows root store")?;
    if !out.status.success() {
        anyhow::bail!(
            "exporting the Windows root store failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let pem = String::from_utf8_lossy(&out.stdout).into_owned();
    if !pem.contains("BEGIN CERTIFICATE") {
        anyhow::bail!("the Windows root store export produced no certificates");
    }
    Ok(pem)
}

/// Write (or refresh) the bundle and return its path.
///
/// Fails rather than falling back to a CA-only file: a tool handed a one-cert
/// bundle would fail TLS everywhere *except* Gate, which is a far more
/// confusing outcome than refusing to connect.
pub fn ensure() -> Result<PathBuf> {
    let ca_pem = std::fs::read_to_string(super::ca_cert_path()?)
        .context("reading Gate's CA certificate -- enable the proxy first")?;
    let mut body = system_roots_pem()?;
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(&ca_pem);
    if !body.ends_with('\n') {
        body.push('\n');
    }

    let path = path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    // 0644: public certificates, and the tool that reads it may run as a
    // different service user than the one that wrote it.
    crate::primitives::write_file(&path, body.as_bytes(), 0o644)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

// Gated as a whole rather than per-test: the single test below does not run on
// Windows, so leaving the module open there makes `use super::*` an unused
// import and `-D warnings` fails the build. Drop this line if a Windows test
// is ever added.
#[cfg(test)]
#[cfg(not(target_os = "windows"))]
mod tests {
    #[test]
    fn system_roots_are_a_real_bundle_not_a_single_cert() {
        // The whole point of this module: a tool that takes a `cafile` replaces
        // its trust store with that file, so the base must carry the platform's
        // full root set. One cert would break every non-Gate TLS call.
        let roots =
            super::system_roots_pem().expect("a system CA bundle must exist on this platform");
        let count = roots.matches("BEGIN CERTIFICATE").count();
        assert!(
            count > 10,
            "expected a full root bundle, found only {count} certificate(s)"
        );
    }
}
