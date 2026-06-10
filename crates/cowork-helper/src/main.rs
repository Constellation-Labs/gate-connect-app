//! Gate Connect credential helper for Cowork on Windows.
//!
//! Claude Desktop spawns this executable at request time (it is named by
//! the `inferenceCredentialHelper` registry policy value that the Cowork
//! integration writes). It reads the upstream provider credential and the
//! Gate API key from the Windows Credential Manager and prints the
//! `{"token","headers"}` JSON Claude expects on stdout.
//!
//! Built with the Windows GUI subsystem so no console window flashes when
//! Claude (a GUI process) spawns it; stdout still flows to the pipe Claude
//! sets up. Exit code 0 = success; non-zero with a stderr message = failure.

#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
  #[cfg(target_os = "windows")]
  {
    use std::io::Write;
    match gate_connect_core::integrations::cowork::windows_helper_emit() {
      Ok(json) => {
        // Ignore write errors: when Claude spawns us with a pipe this
        // succeeds; with no console attached there is nothing to flush to.
        let _ = std::io::stdout().write_all(json.as_bytes());
      }
      Err(e) => {
        eprintln!("gate-connect-cowork-helper: {e:#}");
        std::process::exit(1);
      }
    }
  }
  #[cfg(not(target_os = "windows"))]
  {
    eprintln!("gate-connect-cowork-helper is Windows-only");
    std::process::exit(1);
  }
}
