//! Whose process is on the other end of a loopback connection.
//!
//! The forwarder is the only hop that sees the real client. The engine behind
//! it sees every connection come from the forwarder, which runs as the owner,
//! so a gate on the engine alone would pass a non-owner peer that came this way
//! (`docs/security-notes-loopback.md`, "the ordering of any future UID work").
//! So the forwarder decides, and a peer it cannot prove is the owner's never
//! reaches the engine: its traffic goes straight to where it was going, as if
//! Gate were not installed.
//!
//! What this is for in practice: Claude Desktop's Cowork VM reaches the host
//! through `cowork-svc.exe`, a service running as LocalSystem, which applies
//! the host's PAC and so sends Gate's hosts here. Intercepting that traffic
//! only ever produced a certificate the VM does not trust, and it is not the
//! owner's to route in any case.
//!
//! Fails **closed**, like `engine::peer_uid_for` on Linux: a peer whose user
//! cannot be resolved is not the owner. For this hop "closed" means direct,
//! which costs routing and never exposes the owner's credential. Because that
//! cost is silent - the tool still works, only not through Gate - every peer
//! sent direct is recorded once in [`DIRECT_LOG`], with the reason, so "why is
//! this tool not on Gate" has an answer on disk.
//!
//! Windows only. macOS resolves no loopback peer (the accepted gap in the
//! security notes), and Linux does not run the forwarder, so both treat every
//! peer as the owner's, which is what they did before this existed. The
//! resolution itself is `gate_connect_paths::peer`, shared with the engine
//! and relay behind this hop, which gate on the same answer.

use gate_connect_paths::peer::{verdict, Verdict};

/// The file under the proxy directory that records peers sent direct.
#[cfg_attr(not(windows), allow(dead_code))]
pub const DIRECT_LOG: &str = "forwarder-direct.log";

/// Past this size the log starts over, so a machine full of other accounts'
/// traffic cannot grow it without bound.
#[cfg(windows)]
const DIRECT_LOG_LIMIT: u64 = 64 * 1024;

/// Whether an accepted connection is the owner's, resolved off the async
/// runtime. A peer that is not is recorded in [`DIRECT_LOG`].
pub async fn owner_connected(client: &tokio::net::TcpStream) -> bool {
    let (Ok(peer), Ok(local)) = (client.peer_addr(), client.local_addr()) else {
        return false;
    };
    tokio::task::spawn_blocking(move || match verdict(peer, local) {
        Verdict::Owner => true,
        Verdict::NotOwner { pid, why } => {
            record(pid, why, local.port());
            false
        }
    })
    .await
    .unwrap_or(false)
}

/// Append one line to [`DIRECT_LOG`] for a peer sent direct, once per process
/// for the life of this forwarder. Best effort: a log that cannot be written
/// never changes where the traffic goes.
#[cfg(windows)]
fn record(pid: Option<u32>, why: &str, port: u16) {
    use std::io::Write;
    use std::sync::Mutex;

    // Keyed on the process, not the connection: a VM makes a connection per
    // request, and one line says everything the next hundred would.
    static SEEN: Mutex<Vec<Option<u32>>> = Mutex::new(Vec::new());
    {
        let Ok(mut seen) = SEEN.lock() else { return };
        if seen.contains(&pid) || seen.len() >= 256 {
            return;
        }
        seen.push(pid);
    }
    let Ok(path) = gate_connect_paths::proxy_dir().map(|d| d.join(DIRECT_LOG)) else {
        return;
    };
    let restart = std::fs::metadata(&path).is_ok_and(|m| m.len() > DIRECT_LOG_LIMIT);
    let mut open = std::fs::OpenOptions::new();
    open.create(true);
    if restart {
        open.write(true).truncate(true);
    } else {
        open.append(true);
    }
    let Ok(mut file) = open.open(&path) else {
        return;
    };
    let when = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let name = pid.and_then(gate_connect_paths::peer::process_name);
    let _ = writeln!(
        file,
        "{when} port={port} pid={} process={} -> direct: {why}",
        pid.map_or_else(|| "none".to_string(), |p| p.to_string()),
        name.as_deref().unwrap_or("unknown"),
    );
}

#[cfg(not(windows))]
fn record(_pid: Option<u32>, _why: &str, _port: u16) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_connection_from_this_process_is_the_owners() {
        // The positive half of the gate: this test runs as the owner, so a
        // connection it opens must be let through to the engine.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        assert_eq!(verdict(server.peer_addr().unwrap(), addr), Verdict::Owner);
        assert!(owner_connected(&server).await);
    }
}
