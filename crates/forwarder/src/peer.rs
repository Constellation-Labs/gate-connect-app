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
//! which costs routing and never exposes the owner's credential.
//!
//! Windows only. macOS resolves no loopback peer (the accepted gap in the
//! security notes), and Linux does not run the forwarder, so both treat every
//! peer as the owner's, which is what they did before this existed.

use std::net::SocketAddr;

/// Whether the process holding the client end of `client` -> `listener` runs
/// as the same user as this process.
///
/// Blocking: it reads the TCP table and may enumerate processes. Call it from
/// `spawn_blocking`, as [`owner_connected`] does.
#[cfg(windows)]
pub fn is_owner(client: SocketAddr, listener: SocketAddr) -> bool {
    let Some(own) = windows::own_sid() else {
        return false;
    };
    windows::owning_pid(client, listener)
        .and_then(windows::user_sid)
        .is_some_and(|sid| sid == *own)
}

#[cfg(not(windows))]
pub fn is_owner(_client: SocketAddr, _listener: SocketAddr) -> bool {
    true
}

/// [`is_owner`] for an accepted connection, off the async runtime.
pub async fn owner_connected(client: &tokio::net::TcpStream) -> bool {
    let (Ok(peer), Ok(local)) = (client.peer_addr(), client.local_addr()) else {
        return false;
    };
    tokio::task::spawn_blocking(move || is_owner(peer, local))
        .await
        .unwrap_or(false)
}

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

    /// This process's user SID, as bytes. Resolved once: it cannot change for
    /// the life of the process.
    pub fn own_sid() -> Option<&'static Vec<u8>> {
        static OWN: OnceLock<Option<Vec<u8>>> = OnceLock::new();
        // SAFETY: the pseudo-handle from GetCurrentProcess needs no closing.
        OWN.get_or_init(|| token_sid(unsafe { GetCurrentProcess() }))
            .as_ref()
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
    /// From its token where this process may open it. Where it may not - a
    /// same-user process running elevated, whose token a medium-integrity
    /// process cannot always query - from the terminal services process list,
    /// which reports the user of every process in the caller's own account.
    /// For another account's process, a service's included, both come back
    /// empty, and that is the answer: not the owner.
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
        wts_sid(pid)
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

    /// `pid`'s user SID from the terminal services process list.
    fn wts_sid(pid: u32) -> Option<Vec<u8>> {
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
        let sid = unsafe { std::slice::from_raw_parts(info, count as usize) }
            .iter()
            .find(|p| p.ProcessId == pid)
            .and_then(|p| sid_bytes(p.pUserSid));
        // SAFETY: `info` and `count` are what the enumeration returned.
        unsafe { WTSFreeMemoryExW(WTSTypeProcessInfoLevel1, info.cast(), count) };
        sid
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
        // connection it opens must be let through to the engine. The negative
        // half needs a second account and is covered by hand.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        assert!(owner_connected(&server).await);
    }

    #[cfg(windows)]
    #[test]
    fn a_connection_that_does_not_exist_has_no_owner() {
        // Fails closed: no row, no owner.
        let nowhere: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let also: SocketAddr = "127.0.0.1:2".parse().unwrap();
        assert!(!is_owner(nowhere, also));
    }
}
