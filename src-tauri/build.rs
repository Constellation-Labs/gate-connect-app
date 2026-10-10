fn main() {
    tauri_build::build();
    embed_windows_manifest_for_examples();
}

/// Give the package's examples the application manifest tauri-build gives its
/// binaries.
///
/// tauri-build embeds a manifest declaring a dependency on Common Controls 6.0
/// through embed-resource, which links it with `rustc-link-arg-bins`: binaries
/// only. `examples/ui-harness.rs` is an example target, so on Windows it was
/// linked with no manifest, and a process without one is handed the 5.82
/// comctl32 from System32. The desktop library imports `TaskDialogIndirect`
/// (tauri-runtime-wry's dialog), which only the 6.0 assembly exports, so the
/// loader refused the exe before `main` with STATUS_ENTRYPOINT_NOT_FOUND - seen
/// from Git Bash as a bare exit 127, from Playwright as "Process from
/// config.webServer was not able to start". The main binary never hit it
/// because it has the manifest; the crate's tests never hit it because nothing
/// they call keeps the import alive.
///
/// MSVC only: `/MANIFEST:EMBED` is a link.exe flag. The GNU toolchain would
/// need the manifest compiled into a resource instead, and nothing builds this
/// crate that way.
fn embed_windows_manifest_for_examples() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" || target_env != "msvc" {
        return;
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("ui-harness.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg-examples=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-examples=/MANIFESTINPUT:{}",
        manifest.display()
    );
}
