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
//! peer as the owner's, which is what they did before this existed.

use std::net::SocketAddr;

/// The file under the proxy directory that records peers sent direct.
pub const DIRECT_LOG: &str = "forwarder-direct.log";

/// Past this size the log starts over, so a machine full of other accounts'
/// traffic cannot grow it without bound.
#[cfg(windows)]
const DIRECT_LOG_LIMIT: u64 = 64 * 1024;

/// What [`verdict`] found out about a peer.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Runs as the same user as this process.
    Owner,
    /// Not proven to be the owner's. `pid` is the process holding the client
    /// end, where one was found.
    NotOwner { pid: Option<u32>, why: &'static str },
}

/// Who holds the client end of `client` -> `listener`.
///
/// Blocking: it reads the TCP table and may enumerate processes. Call it from
/// `spawn_blocking`, as [`owner_connected`] does.
#[cfg(windows)]
pub fn verdict(client: SocketAddr, listener: SocketAddr) -> Verdict {
    let not = |pid, why| Verdict::NotOwner { pid, why };
    // Asked first: without it nothing can be proven, and the reason should
    // say so rather than blame the peer.
    let Some(own) = windows::own_sid() else {
        return not(None, "this process's own user could not be read");
    };
    let Some(pid) = windows::owning_pid(client, listener) else {
        return not(None, "no process holds the client end of the connection");
    };
    match windows::user_sid(pid) {
        Some(sid) if sid == own => Verdict::Owner,
        Some(_) => not(Some(pid), "runs as a different user"),
        None => not(
            Some(pid),
            "its user could not be read (another account or a service)",
        ),
    }
}

#[cfg(not(windows))]
pub fn verdict(_client: SocketAddr, _listener: SocketAddr) -> Verdict {
    Verdict::Owner
}

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
    let name = pid.and_then(windows::process_name);
    let _ = writeln!(
        file,
        "{when} port={port} pid={} process={} -> direct: {why}",
        pid.map_or_else(|| "none".to_string(), |p| p.to_string()),
        name.as_deref().unwrap_or("unknown"),
    );
}

#[cfg(not(windows))]
fn record(_pid: Option<u32>, _why: &str, _port: u16) {}

#[cfg(windows)]
mod windows {
    use std::net::SocketAddr;
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, HANDLE};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
    };
    use windows_sys::Win32::Networking::WinSock::AF_INET;
    use windows_sys::Win32::Security::{
        GetLengthSid, GetTokenInformation, TokenUser, PSID, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::RemoteDesktop::{
        WTSEnumerateProcessesExW, WTSFreeMemoryExW, WTSTypeProcessInfoLevel1,
        WTS_CURRENT_SERVER_HANDLE, WTS_PROCESS_INFO_EXW,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// `WTS_ANY_SESSION`, which windows-sys does not export.
    const WTS_ANY_SESSION: u32 = 0xFFFF_FFFE;

    /// This process's user SID, as bytes.
    ///
    /// Kept once read, since it cannot change for the life of the process, but
    /// a failed read is not kept: it is retried on the next connection. Caching
    /// the failure would send every connection direct until the forwarder next
    /// restarted, which could be the end of the login session.
    pub fn own_sid() -> Option<&'static [u8]> {
        static OWN: OnceLock<Vec<u8>> = OnceLock::new();
        if let Some(sid) = OWN.get() {
            return Some(sid);
        }
        // SAFETY: the pseudo-handle from GetCurrentProcess needs no closing.
        let sid = token_sid(unsafe { GetCurrentProcess() })?;
        Some(OWN.get_or_init(|| sid))
    }

    /// The PID owning the client end of a connection to `listener`: the TCP
    /// row whose local end is `client` and whose remote end is `listener`.
    ///
    /// The row's addresses are in network byte order, and its ports sit in the
    /// low 16 bits of a `u32`, also in network order.
    pub fn owning_pid(client: SocketAddr, listener: SocketAddr) -> Option<u32> {
        let (SocketAddr::V4(client), SocketAddr::V4(listener)) = (client, listener) else {
            // The listeners bind 127.0.0.1, so a v6 peer is not one of ours.
            return None;
        };
        let addr = |ip: &std::net::Ipv4Addr| u32::from_ne_bytes(ip.octets());
        let port = |raw: u32| u16::from_be(raw as u16);
        let table = tcp_table()?;
        let rows = rows(&table)?;
        rows.iter()
            .find(|r| {
                r.dwLocalAddr == addr(client.ip())
                    && port(r.dwLocalPort) == client.port()
                    && r.dwRemoteAddr == addr(listener.ip())
                    && port(r.dwRemotePort) == listener.port()
            })
            .map(|r| r.dwOwningPid)
            // 0 and 4 are the idle and System pseudo-processes: a socket with
            // no process behind it any more, never the owner's.
            .filter(|pid| *pid > 4)
    }

    /// The IPv4 TCP table with owning PIDs, as `u32` words so the rows inside
    /// are aligned. Retried because the table can grow between the size query
    /// and the read.
    fn tcp_table() -> Option<Vec<u32>> {
        let mut size: u32 = 0;
        for _ in 0..4 {
            let mut buf = vec![0u32; (size as usize).div_ceil(4).max(1)];
            let mut len = (buf.len() * 4) as u32;
            // SAFETY: `buf` is writable for `len` bytes; the call writes at
            // most that many and reports the size it needed in `len`.
            let rc = unsafe {
                GetExtendedTcpTable(
                    buf.as_mut_ptr().cast(),
                    &mut len,
                    0,
                    AF_INET as u32,
                    TCP_TABLE_OWNER_PID_ALL,
                    0,
                )
            };
            match rc {
                0 => return Some(buf),
                ERROR_INSUFFICIENT_BUFFER => size = len + 1024,
                _ => return None,
            }
        }
        None
    }

    /// The rows of a table [`tcp_table`] returned: a `u32` count, then that
    /// many rows.
    fn rows(table: &[u32]) -> Option<&[MIB_TCPROW_OWNER_PID]> {
        let count = *table.first()? as usize;
        let words = std::mem::size_of::<MIB_TCPROW_OWNER_PID>() / 4;
        if table.len() < 1 + count * words {
            return None;
        }
        // SAFETY: the rows start one word in, `u32`-aligned like the struct,
        // and the length check above keeps `count` of them inside `table`.
        Some(unsafe { std::slice::from_raw_parts(table.as_ptr().add(1).cast(), count) })
    }

    /// The user SID `pid` runs as.
    ///
    /// From its token, which this process can read for every process in its
    /// own account, elevated ones included (checked on Windows 11 against an
    /// elevated shell and the Store-packaged Claude and ChatGPT apps). Where
    /// the token cannot be read, from the terminal services process list, a
    /// fallback for anything in the owner's account that refuses a token
    /// query. For another account's process, a service's included, both come
    /// back empty, and that is the answer: not the owner.
    pub fn user_sid(pid: u32) -> Option<Vec<u8>> {
        // SAFETY: a null return is checked; a real handle is closed below.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if !process.is_null() {
            let sid = token_sid(process);
            // SAFETY: `process` is a handle this function opened.
            unsafe { CloseHandle(process) };
            if sid.is_some() {
                return sid;
            }
        }
        wts_find(|p| p.ProcessId == pid, |p| sid_bytes(p.pUserSid))
    }

    /// `pid`'s image name, for the log. From the terminal services list
    /// because it names every process, services included, where opening the
    /// process to ask it would be refused for exactly the ones worth logging.
    pub fn process_name(pid: u32) -> Option<String> {
        wts_find(|p| p.ProcessId == pid, name)
    }

    /// The PID of a running process by image name.
    #[cfg(test)]
    pub fn pid_named(image: &str) -> Option<u32> {
        wts_find(
            |p| name(p).is_some_and(|n| n.eq_ignore_ascii_case(image)),
            |p| Some(p.ProcessId),
        )
    }

    /// An entry's image name.
    fn name(p: &WTS_PROCESS_INFO_EXW) -> Option<String> {
        if p.pProcessName.is_null() {
            return None;
        }
        // SAFETY: a non-null name is a NUL-terminated wide string that lives
        // as long as the list, which `wts_find` frees only after reading.
        let len = (0..)
            .take_while(|&i| unsafe { *p.pProcessName.add(i) } != 0)
            .count();
        let wide = unsafe { std::slice::from_raw_parts(p.pProcessName, len) };
        Some(String::from_utf16_lossy(wide))
    }

    /// The user SID on `process`'s token.
    fn token_sid(process: HANDLE) -> Option<Vec<u8>> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: `token` is an out-parameter, closed below when set.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        let sid = (|| {
            let mut len: u32 = 0;
            // SAFETY: a size query; it writes only `len`.
            unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len) };
            if len == 0 {
                return None;
            }
            // `u64` words: TOKEN_USER holds pointers.
            let mut buf = vec![0u64; (len as usize).div_ceil(8)];
            // SAFETY: `buf` is writable for at least `len` bytes.
            if unsafe {
                GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len)
            } == 0
            {
                return None;
            }
            // SAFETY: the call succeeded, so `buf` starts with a TOKEN_USER
            // whose SID points inside `buf`.
            let user = unsafe { &*buf.as_ptr().cast::<TOKEN_USER>() };
            sid_bytes(user.User.Sid)
        })();
        // SAFETY: `token` is a handle this function opened.
        unsafe { CloseHandle(token) };
        sid
    }

    /// The first entry in the terminal services process list that `matches`,
    /// read by `read` while the list is still allocated.
    fn wts_find<T>(
        matches: impl Fn(&WTS_PROCESS_INFO_EXW) -> bool,
        read: impl Fn(&WTS_PROCESS_INFO_EXW) -> Option<T>,
    ) -> Option<T> {
        let mut level: u32 = 1;
        let mut info: *mut WTS_PROCESS_INFO_EXW = std::ptr::null_mut();
        let mut count: u32 = 0;
        // SAFETY: out-parameters; on success the list is freed below.
        let ok = unsafe {
            WTSEnumerateProcessesExW(
                WTS_CURRENT_SERVER_HANDLE,
                &mut level,
                WTS_ANY_SESSION,
                (&mut info as *mut *mut WTS_PROCESS_INFO_EXW).cast(),
                &mut count,
            )
        };
        if ok == 0 || info.is_null() {
            return None;
        }
        // SAFETY: on success `info` points at `count` level-1 entries.
        let found = unsafe { std::slice::from_raw_parts(info, count as usize) }
            .iter()
            .find(|p| matches(p))
            .and_then(read);
        // SAFETY: `info` and `count` are what the enumeration returned.
        unsafe { WTSFreeMemoryExW(WTSTypeProcessInfoLevel1, info.cast(), count) };
        found
    }

    /// A SID's bytes. Two SIDs are equal exactly when these are, which is what
    /// `EqualSid` compares.
    fn sid_bytes(sid: PSID) -> Option<Vec<u8>> {
        if sid.is_null() {
            return None;
        }
        // SAFETY: `sid` is a valid SID for the life of its owner, which
        // outlives this call; GetLengthSid reports its size.
        let len = unsafe { GetLengthSid(sid) } as usize;
        (len > 0).then(|| unsafe { std::slice::from_raw_parts(sid.cast::<u8>(), len) }.to_vec())
    }
}

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

    #[cfg(windows)]
    #[test]
    fn a_connection_that_does_not_exist_has_no_owner() {
        // Fails closed: no row, no owner, and the reason says which.
        let nowhere: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let also: SocketAddr = "127.0.0.1:2".parse().unwrap();
        assert!(matches!(
            verdict(nowhere, also),
            Verdict::NotOwner { pid: None, .. }
        ));
    }

    #[cfg(windows)]
    #[test]
    fn a_service_is_not_the_owner() {
        // The negative half against a real process in another account: lsass
        // runs as LocalSystem on every Windows install, as `cowork-svc` does.
        let lsass = windows::pid_named("lsass.exe").expect("lsass is always running");
        assert_eq!(windows::user_sid(lsass), None);
        assert_eq!(windows::process_name(lsass).as_deref(), Some("lsass.exe"));
    }
}
