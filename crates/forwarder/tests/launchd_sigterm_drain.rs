//! Does a socket-activated forwarder, booted out with a tunnel open, finish
//! briefly and exit itself - rather than die on the spot, or hold
//! `launchctl bootout` until launchd kills it?
//!
//! The app replaces or stops a launchd-managed forwarder with `launchctl
//! bootout`. launchd owns that listening socket, so bootout is the only way the
//! port is handed on: it sends SIGTERM, and kills the job once its exit timeout
//! (20s, since the plist sets none) runs out. The forwarder answers SIGTERM
//! with a drain capped at `SIGTERM_DRAIN_LIMIT` (5s). Three claims, each
//! about launchd as much as about our code:
//!
//! 1. SIGTERM reaches the handler: the open tunnel still carries bytes after
//!    bootout has begun. Without the handler, SIGTERM's default action would
//!    end the process at once.
//! 2. The drain is capped: the tunnel closes well inside launchd's exit
//!    timeout, so the forwarder exits itself and is not killed.
//! 3. bootout does not hold its caller for the whole timeout. The app calls it
//!    from `retire_stale` under `ENSURE_LOCK`.
//!
//! The unit tests in `src/main.rs` drive the same drain with SIGTERM stood in
//! by a `Notify`; this is the only place the real signal and the real launchd
//! meet it.
//!
//! **Ignored by default**, run by `.github/workflows/launchd-restart.yml`:
//!
//! ```text
//! cargo test -p gate-connect-forwarder --test launchd_sigterm_drain -- --ignored --nocapture
//! ```
//!
//! It touches nothing of the user's: a label unique to this process, a plist
//! and a `GATE_CONNECT_TEST_HOME` in a scratch directory (a debug build honours
//! the seam, and `cargo test` builds debug), and a bootout on the way past,
//! including when an assertion fails.
#![cfg(target_os = "macos")]

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// launchd's default `ExitTimeOut`, which the production plist leaves alone.
const LAUNCHD_EXIT_TIMEOUT: Duration = Duration::from_secs(20);

struct Job {
    label: String,
    uid: String,
}

impl Drop for Job {
    fn drop(&mut self) {
        let _ = Command::new("/bin/launchctl")
            .arg("bootout")
            .arg(format!("gui/{}/{}", self.uid, self.label))
            .output();
    }
}

/// The production plist's shape - a `Forwarder` socket and nothing else of
/// note - plus the test seam, so the forwarder reads its token and marker
/// from the scratch directory.
fn plist(label: &str, binary: &Path, home: &Path, port: u16, log: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array><string>{binary}</string></array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>GATE_CONNECT_TEST_HOME</key><string>{home}</string>
  </dict>
  <key>StandardErrorPath</key><string>{log}</string>
  <key>Sockets</key>
  <dict>
    <key>Forwarder</key>
    <dict>
      <key>SockNodeName</key><string>127.0.0.1</string>
      <key>SockServiceName</key><string>{port}</string>
      <key>SockType</key><string>stream</string>
      <key>SockFamily</key><string>IPv4</string>
    </dict>
  </dict>
  <key>ProcessType</key><string>Background</string>
</dict>
</plist>
"#,
        binary = binary.display(),
        home = home.display(),
        log = log.display(),
    )
}

fn current_uid() -> String {
    let out = Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .expect("running id -u");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A loopback port nothing holds, released for launchd to bind.
fn free_port() -> u16 {
    let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    l.local_addr().unwrap().port()
}

/// An origin that echoes every byte back until the other side closes.
fn echo_origin() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let Ok((mut sock, _)) = listener.accept() else {
            return;
        };
        let mut buf = [0u8; 256];
        loop {
            match sock.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    if sock.write_all(&buf[..n]).is_err() {
                        return;
                    }
                }
            }
        }
    });
    port
}

/// Send `msg` down the tunnel and expect it back.
fn echo(client: &mut TcpStream, msg: &[u8]) -> Result<(), String> {
    client
        .write_all(msg)
        .map_err(|e| format!("write failed: {e}"))?;
    let mut got = vec![0u8; msg.len()];
    client
        .read_exact(&mut got)
        .map_err(|e| format!("read failed: {e}"))?;
    if got != msg {
        return Err(format!("echoed {got:?}, sent {msg:?}"));
    }
    Ok(())
}

/// Open a direct tunnel through the forwarder on `port`. The first connection
/// is what makes launchd start it.
fn open_tunnel(port: u16, origin: u16) -> TcpStream {
    let mut client =
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(5))
            .expect("connecting to the launchd socket");
    client
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    client
        .write_all(format!("CONNECT 127.0.0.1:{origin} HTTP/1.1\r\n\r\n").as_bytes())
        .unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        let n = client.read(&mut byte).expect("reading the CONNECT answer");
        assert!(n > 0, "the forwarder closed before answering the CONNECT");
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head);
    assert!(head.starts_with("HTTP/1.1 200"), "CONNECT answer: {head}");
    echo(&mut client, b"ping").expect("the tunnel carries bytes before the bootout");
    client
}

#[test]
#[ignore = "drives the real launchd; needs a GUI session"]
fn a_booted_out_forwarder_drains_briefly_and_exits_itself() {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("gate-launchd-sigterm-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let home = dir.join("home");
    let proxy = home.join("app-support").join("Gate Connect").join("proxy");
    fs::create_dir_all(&proxy).expect("creating the scratch proxy directory");
    // What the app writes before starting a forwarder: the token it proves
    // itself with, and the marker saying it is wanted. No engine port file, so
    // every tunnel goes direct.
    fs::write(proxy.join("forwarder.token"), "launchd-sigterm-test-token").unwrap();
    fs::write(proxy.join("forwarder-wanted"), b"").unwrap();

    let uid = current_uid();
    let label = format!(
        "ai.constellation.gate-connect.launchd-test.{}.sigterm-drain",
        std::process::id()
    );
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_gate-connect-forwarder"));
    let log = dir.join("forwarder.log");
    let port = free_port();
    let plist_path = dir.join(format!("{label}.plist"));
    fs::write(&plist_path, plist(&label, &binary, &home, port, &log)).unwrap();

    let out = Command::new("/bin/launchctl")
        .arg("bootstrap")
        .arg(format!("gui/{uid}"))
        .arg(&plist_path)
        .output()
        .expect("running launchctl bootstrap");
    assert!(
        out.status.success(),
        "launchctl bootstrap failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    let job = Job {
        label: label.clone(),
        uid: uid.clone(),
    };
    let stderr = || fs::read_to_string(&log).unwrap_or_default();

    let mut client = open_tunnel(port, echo_origin());

    // Bootout on its own thread, so the tunnel can be exercised while it runs
    // and its own duration measured.
    let started = Instant::now();
    let target = format!("gui/{uid}/{label}");
    let bootout = std::thread::spawn(move || {
        let out = Command::new("/bin/launchctl")
            .arg("bootout")
            .arg(&target)
            .output();
        (Instant::now(), out)
    });

    // Claim 1: SIGTERM was handled, not fatal. Long enough after the bootout
    // began for the signal to have landed, short of the 5s cap.
    std::thread::sleep(Duration::from_millis(1500));
    echo(&mut client, b"still-here").unwrap_or_else(|e| {
        panic!(
            "the tunnel died as soon as bootout began ({e}); SIGTERM ended the \
             forwarder instead of starting its drain. stderr: {}",
            stderr()
        )
    });

    // Claim 2: the drain is capped, so the tunnel closes well inside the exit
    // timeout - by the forwarder, not by launchd's kill.
    let mut rest = [0u8; 16];
    let closed = loop {
        match client.read(&mut rest) {
            Ok(0) | Err(_) => break Instant::now(),
            Ok(_) => continue,
        }
    };
    let closed_after = closed - started;
    println!("tunnel closed {closed_after:?} after bootout began");
    assert!(
        closed_after < LAUNCHD_EXIT_TIMEOUT - Duration::from_secs(5),
        "the tunnel stayed open {closed_after:?}: the drain ran to launchd's exit \
         timeout and was killed, instead of stopping at its own cap. stderr: {}",
        stderr()
    );

    // Claim 3: bootout came back without waiting out the timeout.
    let (returned, out) = bootout.join().expect("the bootout thread");
    let out = out.expect("running launchctl bootout");
    let took = returned - started;
    println!("launchctl bootout returned after {took:?}");
    assert!(
        out.status.success(),
        "launchctl bootout failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    assert!(
        took < LAUNCHD_EXIT_TIMEOUT - Duration::from_secs(5),
        "launchctl bootout held its caller {took:?}, close to launchd's exit timeout"
    );

    drop(job);
    let _ = fs::remove_dir_all(&dir);
}
