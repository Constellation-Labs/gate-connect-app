//! Whose process is on the other end of a loopback connection.
//!
//! Shared by the forwarder, which decides where a peer's traffic goes
//! (`gate-connect-forwarder`'s `peer`), and the engine and relay in core, which
//! refuse to spend the owner's credential for anyone else. Both have to agree
//! on who the owner is, and the forwarder cannot link core, so it lives here.
//!
//! Fails **closed**: a peer whose user cannot be resolved is not the owner.
//!
//! Windows resolves the peer's process from the TCP table and its user from
//! the process token. macOS reads the kernel's TCP socket list, which carries
//! each socket's creating UID. Linux resolves UIDs from `/proc/net/tcp` in core
//! (`engine::peer_uid_for`) and runs no forwarder, so there [`verdict`] treats
//! every peer as the owner's.

use std::net::SocketAddr;

/// What [`verdict`] found out about a peer.
///
/// Only Windows and macOS ever find a peer that is not the owner's.
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
/// `spawn_blocking`.
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

/// Who holds the client end of `client` -> `listener`.
///
/// Blocking: it reads the kernel's TCP socket list. Call it from
/// `spawn_blocking`.
#[cfg(target_os = "macos")]
pub fn verdict(client: SocketAddr, listener: SocketAddr) -> Verdict {
    // SAFETY: geteuid never fails.
    let own = unsafe { libc::geteuid() };
    match macos::peer(client, listener) {
        Some(peer) if peer.uid == own => Verdict::Owner,
        Some(peer) => Verdict::NotOwner {
            pid: peer.pid,
            why: "runs as a different user",
        },
        None => Verdict::NotOwner {
            pid: None,
            why: "no single user holds the client end of the connection",
        },
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn verdict(_client: SocketAddr, _listener: SocketAddr) -> Verdict {
    Verdict::Owner
}

/// The UID of the socket holding the client end of `client` -> `listener`,
/// for core's engine and relay, which compare it to an owner UID. `None` when
/// no such socket is found, its holders disagree, or the list can't be read.
///
/// Blocking, like [`verdict`].
#[cfg(target_os = "macos")]
pub fn peer_uid(client: SocketAddr, listener: SocketAddr) -> Option<u32> {
    macos::peer(client, listener).map(|peer| peer.uid)
}

#[cfg(target_os = "macos")]
pub use macos::process_name;
/// `pid`'s image name, for a log line about a peer turned away.
#[cfg(windows)]
pub use windows::process_name;

/// The macOS lookup: one `net.inet.tcp.pcblist_n` read, the list `netstat`
/// prints, which any user may read and which names each socket's UID.
#[cfg(target_os = "macos")]
mod macos {
    use std::net::SocketAddr;

    pub use super::pcblist::Peer;

    /// The peer holding the client end of `client` -> `listener`.
    pub fn peer(client: SocketAddr, listener: SocketAddr) -> Option<Peer> {
        super::pcblist::find(&pcblist()?, client.port(), listener.port())
    }

    /// `pid`'s process name, where this user may read it.
    pub fn process_name(pid: u32) -> Option<String> {
        let mut buf = [0u8; 256];
        // SAFETY: `buf` is writable for its length, which is what we pass.
        let n = unsafe {
            libc::proc_name(
                pid as libc::c_int,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
            )
        };
        let n = usize::try_from(n)
            .ok()
            .filter(|n| *n > 0 && *n <= buf.len())?;
        Some(String::from_utf8_lossy(&buf[..n]).into_owned())
    }

    /// The raw socket list. Retried because sockets can open between the size
    /// query and the read; the slack covers most of those on the first try.
    fn pcblist() -> Option<Vec<u8>> {
        let name = c"net.inet.tcp.pcblist_n";
        for _ in 0..4 {
            let mut len: libc::size_t = 0;
            // SAFETY: a size query; it writes only `len`.
            let rc = unsafe {
                libc::sysctlbyname(
                    name.as_ptr(),
                    std::ptr::null_mut(),
                    &mut len,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if rc != 0 {
                return None;
            }
            len += len / 4 + 4096;
            let mut buf = vec![0u8; len];
            // SAFETY: `buf` is writable for `len` bytes; the call writes at
            // most that many and reports how many it wrote in `len`.
            let rc = unsafe {
                libc::sysctlbyname(
                    name.as_ptr(),
                    buf.as_mut_ptr().cast(),
                    &mut len,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if rc == 0 {
                buf.truncate(len);
                return Some(buf);
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOMEM) {
                return None;
            }
        }
        None
    }
}

/// Parsing `net.inet.tcp.pcblist_n`, kept free of the sysctl so it builds and
/// is tested on every platform.
///
/// The list is a 24-byte `struct xinpgen` header, then for each TCP socket six
/// self-describing records (`xinpcb_n`, `xsocket_n`, two `xsockbuf_n`,
/// `xsockstat_n`, `xtcpcb_n`), each opening with a `u32` length and a `u32`
/// kind and padded to 8 bytes, then a closing `xinpgen`. This walks records by
/// their own length and groups them by kind, the way `netstat` does
/// (`protopr` in `inet.c`), so record sizes and order are not assumed.
///
/// Two fields are read at fixed offsets, and both are checked:
///
/// - the ports, at 16 and 18 of `xinpcb_n`, which precede its first 8-byte
///   member, so they sit there whatever the struct's packing;
/// - the UID in `xsocket_n`, whose offset does depend on packing (the SDK
///   declares the externalized socket structs under `#pragma pack(4)`). Both
///   layouts are known, and each is accepted only if the same record's
///   protocol and family read as TCP and IPv4/IPv6 at that layout's offsets.
///
/// Anything that does not fit - a record running past the buffer, a kind seen
/// twice in one socket, a socket record in neither layout - returns `None`,
/// which every caller treats as "not the owner".
#[cfg(any(test, target_os = "macos"))]
mod pcblist {
    const XSO_SOCKET: u32 = 0x001;
    const XSO_RCVBUF: u32 = 0x002;
    const XSO_SNDBUF: u32 = 0x004;
    const XSO_STATS: u32 = 0x008;
    const XSO_INPCB: u32 = 0x010;
    const XSO_TCPCB: u32 = 0x020;
    /// Every record a TCP socket carries; a socket is complete once all are in.
    const ALL_TCP: u32 = XSO_SOCKET | XSO_RCVBUF | XSO_SNDBUF | XSO_STATS | XSO_INPCB | XSO_TCPCB;
    /// `sizeof(struct xinpgen)`. A record no longer than this is the closing
    /// header, as `netstat` reads it.
    const XINPGEN_LEN: usize = 24;
    const IPPROTO_TCP: u32 = 6;
    const AF_INET: u32 = 2;
    const AF_INET6: u32 = 30;

    /// Where `xsocket_n` keeps the fields read here, per packing.
    struct SocketLayout {
        protocol: usize,
        family: usize,
        uid: usize,
        last_pid: usize,
    }

    /// `#pragma pack(4)`: `so_pcb` (a `u64`) follows `so_state` at 28.
    const PACKED4: SocketLayout = SocketLayout {
        protocol: 36,
        family: 40,
        uid: 64,
        last_pid: 68,
    };
    /// Natural alignment: `so_pcb` is padded out to 32.
    const NATURAL: SocketLayout = SocketLayout {
        protocol: 40,
        family: 44,
        uid: 68,
        last_pid: 72,
    };

    /// Who holds a socket: its UID, and the last process to use it.
    #[derive(Debug, PartialEq, Eq)]
    pub struct Peer {
        pub uid: u32,
        pub pid: Option<u32>,
    }

    fn u32_at(b: &[u8], at: usize) -> Option<u32> {
        Some(u32::from_ne_bytes(b.get(at..at + 4)?.try_into().ok()?))
    }

    /// A port, stored in network order.
    fn port_at(b: &[u8], at: usize) -> Option<u16> {
        Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
    }

    /// `ROUNDUP64` from the SDK: records are padded to 8 bytes.
    fn roundup64(n: usize) -> usize {
        if n == 0 {
            8
        } else {
            ((n - 1) | 7) + 1
        }
    }

    /// The UID and last PID from an `xsocket_n`, in whichever layout reads as
    /// a TCP socket.
    fn socket_owner(rec: &[u8]) -> Option<Peer> {
        [PACKED4, NATURAL].iter().find_map(|l| {
            let tcp = u32_at(rec, l.protocol)? == IPPROTO_TCP
                && matches!(u32_at(rec, l.family)?, AF_INET | AF_INET6);
            tcp.then_some(())?;
            Some(Peer {
                uid: u32_at(rec, l.uid)?,
                pid: Some(u32_at(rec, l.last_pid)?).filter(|p| *p > 0),
            })
        })
    }

    /// The holder of the socket whose local port is `client_port` and foreign
    /// port `listener_port`: the client end of a loopback connection, seen
    /// from the client's side.
    ///
    /// Matched on ports alone, since the addresses sit after `xinpcb_n`'s
    /// first packing-dependent member. A second socket on the same pair (one
    /// lingering in TIME_WAIT, or one bound to another local address on
    /// purpose) is allowed only if it has the same UID, so a collision can
    /// refuse a connection but never pass one.
    pub fn find(buf: &[u8], client_port: u16, listener_port: u16) -> Option<Peer> {
        if (u32_at(buf, 0)? as usize) < XINPGEN_LEN {
            return None;
        }
        let mut at = roundup64(u32_at(buf, 0)? as usize);
        let mut seen = 0u32;
        let (mut inp, mut so): (Option<&[u8]>, Option<&[u8]>) = (None, None);
        let mut found: Option<Peer> = None;
        while at + 8 <= buf.len() {
            let len = u32_at(buf, at)? as usize;
            let kind = u32_at(buf, at + 4)?;
            if len <= XINPGEN_LEN {
                break;
            }
            let rec = buf.get(at..at + len)?;
            if kind & ALL_TCP == kind && kind.count_ones() == 1 {
                if seen & kind != 0 {
                    return None;
                }
                seen |= kind;
                match kind {
                    XSO_INPCB => inp = Some(rec),
                    XSO_SOCKET => so = Some(rec),
                    _ => {}
                }
            }
            if seen == ALL_TCP {
                seen = 0;
                let (inp, so) = (inp.take()?, so.take()?);
                if port_at(inp, 18)? == client_port && port_at(inp, 16)? == listener_port {
                    let peer = socket_owner(so)?;
                    match &found {
                        Some(prev) if prev.uid != peer.uid => return None,
                        Some(_) => {}
                        None => found = Some(peer),
                    }
                }
            }
            at += roundup64(len);
        }
        found
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const CLIENT: u16 = 51234;
        const LISTENER: u16 = 8787;

        /// A record of `kind`, `len` bytes long before padding.
        fn record(kind: u32, len: usize) -> Vec<u8> {
            let mut r = vec![0u8; roundup64(len)];
            r[0..4].copy_from_slice(&(len as u32).to_ne_bytes());
            r[4..8].copy_from_slice(&kind.to_ne_bytes());
            r
        }

        fn put(r: &mut [u8], at: usize, v: u32) {
            r[at..at + 4].copy_from_slice(&v.to_ne_bytes());
        }

        /// An `xinpcb_n` (112 bytes under natural alignment) for `lport` -> `fport`.
        fn inpcb(lport: u16, fport: u16) -> Vec<u8> {
            let mut r = record(XSO_INPCB, 112);
            r[16..18].copy_from_slice(&fport.to_be_bytes());
            r[18..20].copy_from_slice(&lport.to_be_bytes());
            r
        }

        /// An `xsocket_n` in `layout`: 104 bytes packed, 112 natural.
        fn socket(layout: &SocketLayout, uid: u32, protocol: u32) -> Vec<u8> {
            let len = if layout.uid == PACKED4.uid { 104 } else { 112 };
            let mut r = record(XSO_SOCKET, len);
            put(&mut r, layout.protocol, protocol);
            put(&mut r, layout.family, AF_INET);
            put(&mut r, layout.uid, uid);
            put(&mut r, layout.last_pid, 4242);
            r
        }

        /// One socket's six records, the socket record first or after the pcb
        /// (the kernel's order).
        fn tcp(inp: Vec<u8>, so: Vec<u8>, so_first: bool) -> Vec<u8> {
            let mut out = Vec::new();
            if so_first {
                out.extend(so);
                out.extend(inp);
            } else {
                out.extend(inp);
                out.extend(so);
            }
            for kind in [XSO_RCVBUF, XSO_SNDBUF, XSO_STATS, XSO_TCPCB] {
                out.extend(record(kind, 48));
            }
            out
        }

        /// The full list: header, sockets, closing header.
        fn list(sockets: &[Vec<u8>]) -> Vec<u8> {
            let header = || {
                let mut h = vec![0u8; XINPGEN_LEN];
                put(&mut h, 0, XINPGEN_LEN as u32);
                h
            };
            let mut out = header();
            for s in sockets {
                out.extend(s);
            }
            out.extend(header());
            out
        }

        #[test]
        fn finds_the_client_end_in_either_packing() {
            for layout in [&PACKED4, &NATURAL] {
                let buf = list(&[
                    tcp(inpcb(LISTENER, CLIENT), socket(layout, 501, 6), false),
                    tcp(inpcb(CLIENT, LISTENER), socket(layout, 502, 6), false),
                ]);
                // The listener's accepted end (lport LISTENER) is not the client's.
                assert_eq!(
                    find(&buf, CLIENT, LISTENER),
                    Some(Peer {
                        uid: 502,
                        pid: Some(4242)
                    })
                );
            }
        }

        #[test]
        fn record_order_is_not_assumed() {
            let buf = list(&[tcp(inpcb(CLIENT, LISTENER), socket(&NATURAL, 7, 6), true)]);
            assert_eq!(find(&buf, CLIENT, LISTENER).map(|p| p.uid), Some(7));
        }

        #[test]
        fn an_unknown_socket_has_no_holder() {
            let buf = list(&[tcp(inpcb(CLIENT, LISTENER), socket(&NATURAL, 7, 6), false)]);
            assert_eq!(find(&buf, CLIENT + 1, LISTENER), None);
            assert_eq!(find(&buf, CLIENT, LISTENER + 1), None);
        }

        #[test]
        fn holders_that_disagree_are_nobody() {
            // Same ports, different users: a collision may refuse a connection
            // but must never pass one.
            let buf = list(&[
                tcp(inpcb(CLIENT, LISTENER), socket(&NATURAL, 501, 6), false),
                tcp(inpcb(CLIENT, LISTENER), socket(&NATURAL, 502, 6), false),
            ]);
            assert_eq!(find(&buf, CLIENT, LISTENER), None);
            // The same user twice (a TIME_WAIT leftover) is still that user.
            let buf = list(&[
                tcp(inpcb(CLIENT, LISTENER), socket(&NATURAL, 501, 6), false),
                tcp(inpcb(CLIENT, LISTENER), socket(&NATURAL, 501, 6), false),
            ]);
            assert_eq!(find(&buf, CLIENT, LISTENER).map(|p| p.uid), Some(501));
        }

        #[test]
        fn a_layout_it_does_not_know_is_nobody() {
            // Protocol reads as UDP at both layouts' offsets: no layout fits,
            // so no UID is read from it.
            let buf = list(&[tcp(inpcb(CLIENT, LISTENER), socket(&NATURAL, 7, 17), false)]);
            assert_eq!(find(&buf, CLIENT, LISTENER), None);
        }

        #[test]
        fn a_malformed_list_is_nobody() {
            let good = list(&[tcp(inpcb(CLIENT, LISTENER), socket(&NATURAL, 7, 6), false)]);
            // A record running past the end of the buffer.
            let cut = good.len() - XINPGEN_LEN - 8;
            assert_eq!(find(&good[..cut], CLIENT, LISTENER), None);
            // A kind seen twice within one socket.
            let mut twice = tcp(inpcb(CLIENT, LISTENER), socket(&NATURAL, 7, 6), false);
            twice.splice(0..0, inpcb(CLIENT, LISTENER));
            assert_eq!(find(&list(&[twice]), CLIENT, LISTENER), None);
            // No header at all.
            assert_eq!(find(&[], CLIENT, LISTENER), None);
        }
    }
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;

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

    #[test]
    fn a_service_is_not_the_owner() {
        // The negative half against a real process in another account: lsass
        // runs as LocalSystem on every Windows install, as `cowork-svc` does.
        // Whether its user can be *read* depends on who asks: a standard user
        // cannot, an administrator (a CI runner) reads SYSTEM. Either way it is
        // not the owner, which is the only thing the gate relies on.
        let lsass = windows::pid_named("lsass.exe").expect("lsass is always running");
        let own = windows::own_sid().expect("own user");
        assert_ne!(windows::user_sid(lsass).as_deref(), Some(own));
        assert_eq!(windows::process_name(lsass).as_deref(), Some("lsass.exe"));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;

    #[test]
    fn a_real_loopback_connection_resolves_to_this_user() {
        // Against the live socket list: a wrong struct layout reads some other
        // field as the UID, or fails to read one, and either fails here.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = std::net::TcpStream::connect(addr).unwrap();
        let _accepted = listener.accept().unwrap();
        let peer = client.local_addr().unwrap();
        // SAFETY: geteuid never fails.
        let me = unsafe { libc::geteuid() };
        assert_eq!(peer_uid(peer, addr), Some(me));
        assert_eq!(verdict(peer, addr), Verdict::Owner);
        let nowhere: SocketAddr = "127.0.0.1:1".parse().unwrap();
        assert_eq!(peer_uid(nowhere, addr), None);
        assert!(matches!(
            verdict(nowhere, addr),
            Verdict::NotOwner { pid: None, .. }
        ));
    }
}
