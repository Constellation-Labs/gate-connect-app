//! Does a socket-activated launch agent keep answering after its plist is
//! deleted, as long as nobody boots it out?
//!
//! A disconnect that drains the forwarder instead of stopping it would rest on
//! that: delete `~/Library/LaunchAgents/<label>.plist` so the next login loads
//! nothing, and leave the loaded job alone so every tool still holding the
//! forwarder's address keeps reaching it until this session ends. Two claims,
//! both properties of launchd rather than of our code:
//!
//! 1. Removing the file does not unload the job: `launchctl print` still finds
//!    it.
//! 2. launchd still holds the socket and still starts the job on a connection,
//!    so it runs from its in-memory copy of the plist and does not re-read the
//!    file on demand.
//!
//! The third claim - that nothing loads the job at the next login - is not
//! something a runner can log out to see. It follows from where login looks:
//! launchd loads agents from `~/Library/LaunchAgents`, and the file is gone.
//! This test never puts one there at all.
//!
//! The job mirrors the forwarder's shape rather than using inetd
//! compatibility: a plain `Sockets` entry, collected by the program with
//! `launch_activate_socket`, here through Python's `ctypes` so the test needs
//! no second binary. Each run accepts one connection, writes one line, and
//! exits cleanly, which is what a forwarder does once its marker is gone.
//!
//! **Ignored by default**, run by `.github/workflows/launchd-restart.yml`:
//!
//! ```text
//! cargo test -p gate-connect-core --test launchd_plist_removed -- --ignored --nocapture
//! ```
//!
//! It touches nothing of the user's: a label unique to this process, a plist
//! in a scratch directory, and a bootout on the way past, including when an
//! assertion fails.
#![cfg(target_os = "macos")]

use std::fs;
use std::io::Read;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const SOCKET_NAME: &str = "Listener";

/// The program launchd starts: collect the socket, answer one connection,
/// append one line to the tally, exit 0.
const PROGRAM: &str = r#"#!/usr/bin/env python3
import ctypes, socket, sys
libc = ctypes.CDLL(None)
fds = ctypes.POINTER(ctypes.c_int)()
cnt = ctypes.c_size_t(0)
rc = libc.launch_activate_socket(b"Listener", ctypes.byref(fds), ctypes.byref(cnt))
if rc != 0 or cnt.value == 0:
    sys.exit(3)
s = socket.socket(fileno=fds[0])
conn, _ = s.accept()
with open(sys.argv[1], "a") as f:
    f.write("run\n")
conn.sendall(b"ok\n")
conn.close()
"#;

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

fn plist(label: &str, program: &Path, tally: &Path, port: u16) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array><string>{program}</string><string>{tally}</string></array>
  <key>Sockets</key>
  <dict>
    <key>{SOCKET_NAME}</key>
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
        program = program.display(),
        tally = tally.display(),
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
    let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    l.local_addr().unwrap().port()
}

fn runs(tally: &Path) -> usize {
    fs::read_to_string(tally)
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

/// Connect, read the answer, and wait for the run to be tallied.
fn poke(port: u16, tally: &Path, want: usize) -> Result<(), String> {
    let mut stream =
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(5))
            .map_err(|e| format!("connect to 127.0.0.1:{port} failed: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && runs(tally) < want {
        std::thread::sleep(Duration::from_millis(100));
    }
    if runs(tally) < want {
        return Err(format!(
            "no run tallied (have {}, want {want}); answer was {answer:?}",
            runs(tally)
        ));
    }
    Ok(())
}

fn loaded(job: &Job) -> bool {
    Command::new("/bin/launchctl")
        .arg("print")
        .arg(format!("gui/{}/{}", job.uid, job.label))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
#[ignore = "drives the real launchd; needs a GUI session"]
fn a_loaded_socket_agent_survives_its_plist_being_deleted() {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("gate-launchd-plist-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("creating the scratch directory");
    let uid = current_uid();
    let label = format!(
        "ai.constellation.gate-connect.launchd-test.{}.plist-removed",
        std::process::id()
    );
    let program = dir.join("listener.py");
    let tally = dir.join("runs");
    fs::write(&program, PROGRAM).expect("writing the program");
    Command::new("/bin/chmod")
        .arg("+x")
        .arg(&program)
        .status()
        .expect("chmod");

    let port = free_port();
    let plist_path = dir.join(format!("{label}.plist"));
    fs::write(&plist_path, plist(&label, &program, &tally, port)).expect("writing the plist");

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
        label,
        uid: uid.clone(),
    };

    // Baseline: with the plist in place, a connection starts the job. Without
    // this, a failure below could be a job that never worked at all.
    poke(port, &tally, 1).expect("the job did not answer even with its plist in place");

    fs::remove_file(&plist_path).expect("deleting the plist");
    assert!(!plist_path.exists());

    // Claim 1: still loaded.
    assert!(
        loaded(&job),
        "deleting the plist unloaded the job; a drain cannot leave it running until logout"
    );

    // Claim 2: still socket-activated, from the in-memory copy. Twice, so the
    // second start is one launchd made entirely after the file was gone and
    // after a clean exit.
    for want in [2, 3] {
        poke(port, &tally, want).unwrap_or_else(|e| {
            panic!(
                "after the plist was deleted, launchd did not start the job on a connection: {e}"
            )
        });
    }
    assert!(
        loaded(&job),
        "the job was unloaded after running without its plist"
    );

    // And bootout by label still works with no file behind it, which is what
    // an uninstall or an explicit stop would do.
    drop(job);
    let gone = Command::new("/bin/launchctl")
        .arg("print")
        .arg(format!(
            "gui/{uid}/ai.constellation.gate-connect.launchd-test.{}.plist-removed",
            std::process::id()
        ))
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true);
    assert!(
        gone,
        "bootout by label did not remove a job whose plist is gone"
    );
    assert!(
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(2)).is_err(),
        "the port still answers after bootout"
    );

    let _ = fs::remove_dir_all(&dir);
}
