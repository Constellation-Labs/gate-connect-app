//! Emits `GATE_CORE_FINGERPRINT`: a digest of this crate's own sources (plus
//! its version), compiled into the binary. The Linux proxy helper daemon and
//! its client exchange it in the control `Hello` handshake so a daemon left
//! over from a different build of the core crate is detected and replaced -
//! for *any* core change (catalog, relay routing, engine behavior), not just
//! wire-incompatible ones, which is all `PROTOCOL_VERSION` covers.
//!
//! Deterministic over source contents (not a timestamp): the CLI and the
//! desktop app link the same core crate, so binaries built from the same
//! sources always agree and never ping-pong-replace each other's daemon.
//! This is a change detector, not a security boundary - the control socket's
//! UID and token checks remain the access control - so a hand-rolled FNV-1a
//! keeps the build script dependency-free.

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");

    let mut files = Vec::new();
    collect_rs_files(Path::new("src"), &mut files);
    files.sort();

    let mut hash = Fnv1a::new();
    hash.update(env!("CARGO_PKG_VERSION").as_bytes());
    for path in files {
        hash.update(path.to_string_lossy().as_bytes());
        hash.update(&fs::read(&path).unwrap_or_default());
    }
    println!("cargo:rustc-env=GATE_CORE_FINGERPRINT={:016x}", hash.0);
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// 64-bit FNV-1a.
struct Fnv1a(u64);

impl Fnv1a {
    fn new() -> Self {
        Fnv1a(0xcbf2_9ce4_8422_2325)
    }

    fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}
