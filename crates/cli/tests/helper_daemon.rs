//! Drives the real proxy helper daemon across a process boundary.
//!
//! The helper (`gate-connect --proxy-helper`, `gate_connect_core::proxy::helper`)
//! owns the engine on Linux so routing outlives the GUI, and every caller reaches
//! it through `HelperClient` over a Unix socket. Neither half had a test that ran
//! them as the separate processes they are in production, and regressions in the
//! seam between them shipped repeatedly: a read-only status probe killed the live
//! daemon (48fb096), the daemon served control connections one at a time
//! (5d244ee), and a second process could not see an engine it was not hosting
//! (1972651, on macOS and Windows; its Linux half is the lost-port warning
//! checked below). Each reached a user as a routing failure or a UI timeout, which is
//! the slowest place to find one. These tests put the failure here instead.
//!
//! What runs: the `gate-connect` binary Cargo builds for this package, spawned
//! as a child with `--proxy-helper`, exactly the argv `HelperClient` uses when it
//! spawns one. The test process plays the GUI and the CLI, through the real
//! `HelperClient`. No in-process daemon, so a daemon that wedges, exits or
//! serializes its clients is seen the way a user's app would see it.
//!
//! Hermetic, and it has to be: a developer running this may have Gate Connect
//! routing their own traffic. Every test gets a throwaway `GATE_CONNECT_TEST_HOME`,
//! which is where the control channel resolves (`control::runtime_dir`), so the
//! socket, token, pidfile and singleton lock are never the live ones.
//! `GATE_CONNECT_TEST_SECRETS` keeps any secret read off the OS store. The CA
//! is minted in memory and never trusted: no trust store is written, and
//! nothing here needs one, since no TLS is ever completed. The gateway is
//! `.invalid` and no request is ever routed, so nothing leaves loopback. The
//! only process ever signalled is the child this file spawned, by its PID.
//!
//! Linux only, and not by choice of the test: the helper and its client exist
//! only there (`#[cfg(target_os = "linux")]` on `proxy::helper`,
//! `proxy::helper_client` and `proxy::control`). macOS and Windows host the
//! engine inside whichever process enabled it, with no daemon to drive; the one
//! test compiled there pins that, so the day a helper appears on either it
//! arrives with a failing test rather than none.

#[cfg(target_os = "linux")]
mod linux {
    use std::fs::File;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::os::unix::process::ExitStatusExt;
    use std::path::Path;
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::sync::{Arc, Barrier, Mutex, MutexGuard};
    use std::time::{Duration, Instant};

    use gate_connect_core::account::BillingMode;
    use gate_connect_core::proxy::control::{self, Response};
    use gate_connect_core::proxy::helper_client::{self, HelperClient, Intercepting};
    use gate_connect_core::proxy::{default_domains, system_proxy, ProxyDomain};
    use tempfile::TempDir;

    /// Never resolves (RFC 2606), so an engine that did try to reach its
    /// gateway would fail rather than leave the machine.
    const GATEWAY_URL: &str = "https://gateway.invalid";
    const API_KEY: &str = "sk-gw-helper-test";

    /// How long a daemon gets to come up, or to go away. Generous next to the
    /// client's own 3s control timeout because CI runners stall; a healthy
    /// daemon takes milliseconds either way.
    const PROCESS_DEADLINE: Duration = Duration::from_secs(15);

    /// The control channel, the data dir and the secret store all resolve from
    /// process-global environment, and the test process is the client. So the
    /// tests in this binary run one at a time, each pointing that environment at
    /// its own home while it holds this lock.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// One test's isolated world: its home, and the environment that points the
    /// in-process client at it. Declared before any [`Daemon`] in a test so the
    /// daemon is killed before the home is deleted under it.
    struct Sandbox {
        home: TempDir,
        _serial: MutexGuard<'static, ()>,
    }

    impl Sandbox {
        fn new() -> Sandbox {
            // A failed test poisons the lock; the next one still gets a fresh
            // home, so the poison carries nothing worth refusing over.
            let serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
            let home = TempDir::new().expect("temp home");
            point_env_at(home.path());
            Sandbox {
                home,
                _serial: serial,
            }
        }

        fn path(&self) -> &Path {
            self.home.path()
        }

        fn spawn_daemon(&self) -> Daemon {
            Daemon::spawn(self.path())
        }
    }

    /// Point this process's control channel, data dir and secret store at
    /// `home`. `XDG_RUNTIME_DIR` is moved as well, though the seam already wins
    /// over it: if the seam ever stopped covering the control channel, the
    /// fallback would still be a throwaway dir rather than the session's.
    fn point_env_at(home: &Path) {
        std::env::set_var("GATE_CONNECT_TEST_HOME", home);
        std::env::set_var("GATE_CONNECT_TEST_SECRETS", home.join("secrets"));
        std::env::set_var("XDG_RUNTIME_DIR", home.join("xdg"));
    }

    /// A helper daemon this test spawned. Dropping it kills the child by PID,
    /// so a failing assertion never leaves one behind.
    struct Daemon {
        child: Child,
        log: std::path::PathBuf,
    }

    impl Daemon {
        /// Spawn `gate-connect --proxy-helper` against `home` and wait until it
        /// answers an authenticated `Hello`.
        fn spawn(home: &Path) -> Daemon {
            // One log per spawn: the restart tests run two daemons on one home,
            // and a failure should show the one that failed.
            let log = home.join(format!(
                "helper-{}.log",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default()
            ));
            let out = File::create(&log).expect("create helper log");
            let child = Command::new(env!("CARGO_BIN_EXE_gate-connect"))
                .arg("--proxy-helper")
                .env("GATE_CONNECT_TEST_HOME", home)
                .env("GATE_CONNECT_TEST_SECRETS", home.join("secrets"))
                .env("XDG_RUNTIME_DIR", home.join("xdg"))
                .env("HOME", home)
                .env_remove("GATE_PROXY_DEBUG")
                .stdin(Stdio::null())
                .stdout(out.try_clone().expect("clone log handle"))
                .stderr(out)
                .spawn()
                .expect("spawn gate-connect --proxy-helper");
            let mut daemon = Daemon { child, log };
            daemon.wait_until_serving();
            daemon
        }

        fn pid(&self) -> libc::pid_t {
            self.child.id() as libc::pid_t
        }

        fn wait_until_serving(&mut self) {
            let deadline = Instant::now() + PROCESS_DEADLINE;
            let mut last_err = String::new();
            while Instant::now() < deadline {
                if let Some(status) = self.child.try_wait().expect("poll helper") {
                    panic!(
                        "the helper exited ({status}) before serving\n{}",
                        self.log()
                    );
                }
                match HelperClient::connect_existing() {
                    Ok(_) => return,
                    Err(e) => last_err = format!("{e:#}"),
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            panic!(
                "the helper never answered a Hello (last error: {last_err})\n{}",
                self.log()
            );
        }

        fn is_alive(&mut self) -> bool {
            self.child.try_wait().expect("poll helper").is_none()
        }

        /// Wait for the child to exit, reaping it. `None` if it is still running
        /// at the deadline.
        fn wait_for_exit(&mut self) -> Option<ExitStatus> {
            let deadline = Instant::now() + PROCESS_DEADLINE;
            while Instant::now() < deadline {
                if let Some(status) = self.child.try_wait().expect("poll helper") {
                    return Some(status);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            None
        }

        fn log(&self) -> String {
            let text = std::fs::read_to_string(&self.log).unwrap_or_default();
            format!("--- helper output ---\n{text}--- end helper output ---")
        }
    }

    impl Drop for Daemon {
        fn drop(&mut self) {
            // Our own child, by PID; `kill` on one already reaped is a no-op.
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// Every catalog domain switched on, so a toggle has something to move and
    /// the intercept count is not the default's single entry.
    fn all_domains_on() -> Vec<ProxyDomain> {
        let mut domains = default_domains();
        for d in &mut domains {
            d.enabled = true;
        }
        assert!(domains.len() >= 2, "a toggle test needs two domains");
        domains
    }

    /// A throwaway CA as `(cert_pem, key_pem)`, minted once per test binary.
    ///
    /// Not `proxy::ca::load_or_create`: that keeps the key private to core, as
    /// it should. This one is never trusted anywhere and never completes a
    /// handshake; the engine only needs a root to build its signer from. One
    /// root for the whole binary, because the helper replaces an engine whose
    /// CA changes, and the tests that re-arm mean to reuse theirs.
    fn test_ca() -> &'static (String, String) {
        static CA: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
        CA.get_or_init(|| {
            let key = rcgen::KeyPair::generate().expect("CA key pair");
            let mut params =
                rcgen::CertificateParams::new(Vec::<String>::new()).expect("CA params");
            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
            params
                .distinguished_name
                .push(rcgen::DnType::CommonName, "Gate Connect helper test CA");
            params.key_usages = vec![
                rcgen::KeyUsagePurpose::KeyCertSign,
                rcgen::KeyUsagePurpose::CrlSign,
                rcgen::KeyUsagePurpose::DigitalSignature,
            ];
            let cert = params.self_signed(&key).expect("self-signed CA");
            (cert.pem(), key.serialize_pem())
        })
    }

    /// Arm the engine the way `manager_linux::enable` does: a CA, the domain
    /// list, and the preferred ports from the last run.
    fn arm(
        client: &mut HelperClient,
        domains: &[ProxyDomain],
        detached: bool,
        preferred: (Option<u16>, Option<u16>),
    ) -> Intercepting {
        let (cert_pem, key_pem) = test_ca();
        client
            .set_intercept(
                GATEWAY_URL,
                API_KEY,
                "",
                "",
                BillingMode::Byok,
                cert_pem,
                key_pem,
                domains,
                detached,
                preferred.0,
                preferred.1,
            )
            .expect("SetIntercept is accepted")
    }

    fn connect() -> HelperClient {
        HelperClient::connect_existing().expect("connect to the helper")
    }

    /// Whether something accepts on `127.0.0.1:port` right now.
    fn accepts(port: u16) -> bool {
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(2)).is_ok()
    }

    /// The relay's status line for its liveness path, over a raw socket so no
    /// ambient `HTTP_PROXY` can route the request anywhere but loopback.
    fn relay_liveness(relay_port: u16) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", relay_port)).expect("connect to relay");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("relay read timeout");
        write!(
            stream,
            "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{relay_port}\r\nConnection: close\r\n\r\n",
            gate_connect_core::proxy::RELAY_LIVENESS_PATH
        )
        .expect("write liveness request");
        let mut head = String::new();
        BufReader::new(stream)
            .read_line(&mut head)
            .expect("read relay status line");
        head.trim_end().to_string()
    }

    /// Enable: a `SetIntercept` starts the engine inside the helper process, on
    /// loopback ports the caller is told about, intercepting what it was given.
    #[test]
    fn enable_starts_the_engine_in_the_helper() {
        let sandbox = Sandbox::new();
        let _daemon = sandbox.spawn_daemon();
        let mut gui = connect();

        let (running, port, intercepting) = gui.status().expect("status before enable");
        assert!(!running, "a fresh helper has no engine yet");
        assert_eq!((port, intercepting), (None, 0));

        let domains = all_domains_on();
        let bound = arm(&mut gui, &domains, false, (None, None));
        assert_ne!(bound.port, 0);
        assert_ne!(bound.relay_port, 0);
        assert_ne!(
            bound.port, bound.relay_port,
            "MITM and relay are two listeners"
        );

        let (running, port, intercepting) = gui.status().expect("status after enable");
        assert!(running, "the engine must be running after SetIntercept");
        assert_eq!(port, Some(bound.port), "status names the port enable bound");
        assert_eq!(intercepting, domains.len(), "every enabled domain is armed");

        // Bound by the child, not merely reported: this process never started
        // an engine, so whatever answers is the helper's.
        assert!(accepts(bound.port), "the MITM port must accept");
        assert_eq!(
            relay_liveness(bound.relay_port),
            "HTTP/1.1 204 No Content",
            "the helper's relay must answer its liveness path"
        );
    }

    /// Disable: `SetPassthrough` disarms every domain but keeps the engine and
    /// its ports, so a session that froze the proxy pointer keeps flowing. The
    /// next enable reuses that engine rather than starting a second one.
    #[test]
    fn disable_drops_to_passthrough_and_keeps_the_ports() {
        let sandbox = Sandbox::new();
        let _daemon = sandbox.spawn_daemon();
        let mut gui = connect();
        let domains = all_domains_on();
        let bound = arm(&mut gui, &domains, false, (None, None));

        gui.set_passthrough().expect("SetPassthrough is accepted");
        let (running, port, intercepting) = gui.status().expect("status after disable");
        assert!(running, "disable must not stop the engine");
        assert_eq!(port, Some(bound.port), "disable must keep the port");
        assert_eq!(intercepting, 0, "disable must disarm every domain");
        assert!(accepts(bound.port), "the MITM port must stay bound");
        assert!(accepts(bound.relay_port), "the relay port must stay bound");

        // Re-enable with no preferred ports: if the old engine had been
        // stopped, a fresh bind would land on new ports.
        let again = arm(&mut gui, &domains, false, (None, None));
        assert_eq!(
            (again.port, again.relay_port),
            (bound.port, bound.relay_port),
            "a re-enable must reuse the parked engine"
        );
        let (_, _, intercepting) = gui.status().expect("status after re-enable");
        assert_eq!(intercepting, domains.len());
    }

    /// Domain toggle: the manager pushes the whole list again with one entry
    /// flipped (`manager_linux::set_domain`), and the live engine must follow it
    /// in place, on the same port.
    #[test]
    fn domain_toggle_updates_the_live_engine() {
        let sandbox = Sandbox::new();
        let _daemon = sandbox.spawn_daemon();
        let mut gui = connect();
        let mut domains = all_domains_on();
        let bound = arm(&mut gui, &domains, false, (None, None));
        let all = domains.len();

        let slug = domains[0].slug.clone();
        domains[0].enabled = false;
        let off = arm(&mut gui, &domains, false, (None, None));
        let (running, port, intercepting) = gui.status().expect("status after toggle off");
        assert!(running);
        assert_eq!(
            intercepting,
            all - 1,
            "turning {slug} off must disarm exactly it"
        );
        assert_eq!(port, Some(bound.port), "a toggle must not move the engine");
        assert_eq!(off.port, bound.port);

        domains[0].enabled = true;
        arm(&mut gui, &domains, false, (None, None));
        let (_, port, intercepting) = gui.status().expect("status after toggle on");
        assert_eq!(intercepting, all, "turning {slug} back on must re-arm it");
        assert_eq!(port, Some(bound.port));
    }

    /// Status from a second client: a caller that did not arm the engine must
    /// still see it. On Linux this is how a second process (the CLI beside the
    /// app, or a restarted GUI) finds the engine it is not hosting - by asking
    /// the helper, rather than by looking at its own handle, which is what left
    /// macOS and Windows blind in 1972651.
    #[test]
    fn status_from_a_second_client_sees_the_armed_engine() {
        let sandbox = Sandbox::new();
        let _daemon = sandbox.spawn_daemon();
        let domains = all_domains_on();

        // The GUI arms and holds its connection.
        let mut gui = connect();
        let bound = arm(&mut gui, &domains, false, (None, None));

        let mut other = connect();
        let (running, port, intercepting) = other.status().expect("second client status");
        assert!(
            running,
            "a second client must see the engine the first armed"
        );
        assert_eq!(port, Some(bound.port));
        assert_eq!(intercepting, domains.len());
        drop(other);
        drop(gui);

        // `gate-connect proxy enable` arms detached and exits. A later client
        // must still find the engine armed: reverting on that exit is what once
        // un-routed the machine while status read "on".
        let mut cli = connect();
        arm(&mut cli, &domains, true, (None, None));
        drop(cli);
        let mut later = connect();
        let (running, port, intercepting) = later.status().expect("status after CLI exit");
        assert!(running);
        assert_eq!(port, Some(bound.port));
        assert_eq!(
            intercepting,
            domains.len(),
            "a detached enable must outlive the connection that made it"
        );
    }

    /// SIGTERM, which is what a session teardown or `systemctl stop` sends: the
    /// helper must actually go, and leave nothing that wedges the next one - not
    /// its singleton lock, not its stale socket, not its ports.
    #[test]
    fn sigterm_ends_the_helper_and_frees_what_it_held() {
        let sandbox = Sandbox::new();
        let mut daemon = sandbox.spawn_daemon();
        let mut gui = connect();
        let bound = arm(&mut gui, &all_domains_on(), false, (None, None));

        // Our own child, by the PID `spawn` handed back.
        // SAFETY: `kill` with a valid signal is safe for any pid.
        let rc = unsafe { libc::kill(daemon.pid(), libc::SIGTERM) };
        assert_eq!(rc, 0, "signalling our own helper child");
        let status = daemon
            .wait_for_exit()
            .unwrap_or_else(|| panic!("the helper ignored SIGTERM\n{}", daemon.log()));
        // No handler is installed, so the default disposition ends it. Either a
        // signal death or a clean exit is a helper that honoured the request.
        assert!(
            status.signal() == Some(libc::SIGTERM) || status.success(),
            "unexpected exit after SIGTERM: {status}\n{}",
            daemon.log()
        );

        // The held connection now reads as gone, not as a hang.
        assert!(gui.status().is_err(), "a dead helper must not answer");
        assert!(
            HelperClient::connect_existing().is_err(),
            "nothing may answer on the control socket after SIGTERM"
        );
        assert!(
            !accepts(bound.port),
            "the MITM port must close with the helper"
        );

        // A successor on the same home comes up (the flock died with the old
        // process, and it replaces the stale socket) and reclaims both ports.
        let _successor = sandbox.spawn_daemon();
        let mut gui = connect();
        let again = arm(
            &mut gui,
            &all_domains_on(),
            false,
            (Some(bound.port), Some(bound.relay_port)),
        );
        assert_eq!(
            (again.port, again.relay_port),
            (bound.port, bound.relay_port),
            "the ports a SIGTERMed helper held must be free to reclaim"
        );
    }

    /// One frame on the control socket, by direction, named by its variant.
    /// Names only: a `Hello` carries the control token, which has no business in
    /// a test failure message.
    #[derive(Debug, Clone, PartialEq)]
    enum Frame {
        ToDaemon(String),
        ToClient(String),
    }

    fn variant_name(line: &str) -> String {
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(serde_json::Value::String(unit)) => unit,
            Ok(serde_json::Value::Object(map)) => map
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "<empty>".into()),
            _ => "<unparseable>".into(),
        }
    }

    /// Stand a recording relay between one client and the real helper.
    ///
    /// The relay listens on a socket inside `probe_home`'s control dir (with a
    /// copy of the real token beside it), so a client pointed at `probe_home`
    /// connects to the relay and every frame it exchanges with the daemon is
    /// written down. With `skew`, the daemon's `Hello` reply has its build
    /// fingerprint replaced, which turns the real daemon into the build-skewed
    /// one the probe in 48fb096 used to kill. Returns the recorded frames once
    /// the client hangs up.
    fn recording_relay(
        daemon_sock: std::path::PathBuf,
        probe_sock: std::path::PathBuf,
        skew: bool,
    ) -> std::thread::JoinHandle<Vec<Frame>> {
        let _ = std::fs::remove_file(&probe_sock);
        let listener = UnixListener::bind(&probe_sock).expect("bind recording relay");
        std::thread::spawn(move || {
            let mut frames = Vec::new();
            let (client, _) = listener.accept().expect("probe connects to the relay");
            let daemon = UnixStream::connect(&daemon_sock).expect("relay reaches the helper");
            for s in [&client, &daemon] {
                s.set_read_timeout(Some(Duration::from_secs(10)))
                    .expect("relay read timeout");
            }
            let mut client_w = client.try_clone().expect("clone client side");
            let mut daemon_w = daemon.try_clone().expect("clone daemon side");
            let mut client_r = BufReader::new(client);
            let mut daemon_r = BufReader::new(daemon);
            // The protocol is strictly request then reply, so one loop pumps it.
            loop {
                let mut req = String::new();
                if client_r.read_line(&mut req).unwrap_or(0) == 0 {
                    break;
                }
                frames.push(Frame::ToDaemon(variant_name(req.trim_end())));
                if daemon_w.write_all(req.as_bytes()).is_err() {
                    break;
                }
                let mut resp = String::new();
                if daemon_r.read_line(&mut resp).unwrap_or(0) == 0 {
                    break;
                }
                frames.push(Frame::ToClient(variant_name(resp.trim_end())));
                if skew {
                    if let Ok(Response::Hello { ok, version, .. }) =
                        serde_json::from_str::<Response>(resp.trim_end())
                    {
                        let skewed = Response::Hello {
                            ok,
                            version,
                            fingerprint: "0000000000000000".into(),
                        };
                        resp = serde_json::to_string(&skewed).expect("encode skewed Hello") + "\n";
                    }
                }
                if client_w.write_all(resp.as_bytes()).is_err() {
                    break;
                }
            }
            frames
        })
    }

    /// Run the read-only probe `proxy status` performs (`connect_existing`,
    /// then `Status` if it got a client) through a recording relay, and return
    /// whether it reached a usable daemon plus the frames it exchanged.
    fn probe_through_relay(sandbox: &Sandbox, probe_home: &Path, skew: bool) -> (bool, Vec<Frame>) {
        let daemon_sock = control::socket_path().expect("daemon socket");
        let token = std::fs::read_to_string(control::token_path().expect("token path"))
            .expect("read helper token");

        // Point this process at the probe home for the probe alone.
        point_env_at(probe_home);
        std::fs::write(control::token_path().expect("probe token path"), token)
            .expect("copy token beside the relay");
        let relay = recording_relay(
            daemon_sock,
            control::socket_path().expect("probe socket"),
            skew,
        );
        let reached = match HelperClient::connect_existing() {
            Ok(mut client) => {
                client.status().expect("a reached helper answers Status");
                true
            }
            Err(_) => false,
        };
        let frames = relay.join().expect("relay thread");
        point_env_at(sandbox.path());
        (reached, frames)
    }

    /// Read-only probe: asking a helper how it is doing must leave its engine
    /// armed, including when the helper is from another build. In 48fb096 the
    /// probe sent `Shutdown` to a build-skewed daemon, and every test binary or
    /// skewed CLI that asked for status ended the user's routing. Whatever
    /// breaks this, the failure names the frames the probe actually sent.
    #[test]
    fn read_only_probe_leaves_the_engine_armed() {
        let sandbox = Sandbox::new();
        let probe_home = TempDir::new().expect("probe home");
        let mut daemon = sandbox.spawn_daemon();
        let domains = all_domains_on();

        // The GUI arms and keeps its connection: the probe's disconnect must not
        // count as the last client leaving.
        let mut gui = connect();
        let bound = arm(&mut gui, &domains, false, (None, None));

        for skew in [false, true] {
            let what = if skew {
                "a build-skewed helper"
            } else {
                "a same-build helper"
            };
            let (reached, frames) = probe_through_relay(&sandbox, probe_home.path(), skew);
            assert_eq!(
                reached, !skew,
                "probing {what}: a same-build helper is usable, a skewed one is reported stale \
                 (frames: {frames:?})"
            );

            let mutating: Vec<&Frame> = frames
                .iter()
                .filter(|f| {
                    matches!(f, Frame::ToDaemon(name)
                        if !matches!(name.as_str(), "Hello" | "Status"))
                })
                .collect();
            assert!(
                mutating.is_empty(),
                "probing {what} must only say Hello and ask Status, but it sent {mutating:?} \
                 (all frames: {frames:?})"
            );

            assert!(
                daemon.is_alive(),
                "probing {what} ended the helper; the probe's frames were {frames:?}\n{}",
                daemon.log()
            );
            let (running, port, intercepting) = gui.status().unwrap_or_else(|e| {
                panic!("probing {what} cut the GUI off ({e:#}); the probe's frames were {frames:?}")
            });
            assert!(
                running && port == Some(bound.port) && intercepting == domains.len(),
                "probing {what} disarmed the engine: status ({running}, {port:?}, \
                 {intercepting}), the probe's frames were {frames:?}"
            );
        }
    }

    /// Port persistence: the engine port is saved where `manager_linux::enable`
    /// saves it, and a helper restarted through the real retire path
    /// (`shutdown_daemon`) rebinds the same MITM and relay ports when handed
    /// them back. Tools that captured `HTTPS_PROXY` or baked the relay URL at
    /// launch keep working only if this holds, so when it cannot hold the
    /// helper has to announce it.
    #[test]
    fn ports_persist_across_a_helper_restart() {
        let sandbox = Sandbox::new();
        let mut first = sandbox.spawn_daemon();
        let domains = all_domains_on();
        let mut gui = connect();
        let bound = arm(
            &mut gui,
            &domains,
            false,
            (
                system_proxy::load_port().expect("read persisted port"),
                None,
            ),
        );
        system_proxy::save_port(bound.port).expect("persist the engine port");
        drop(gui);

        helper_client::shutdown_daemon();
        assert!(
            first.wait_for_exit().is_some(),
            "shutdown_daemon must end the helper\n{}",
            first.log()
        );

        let mut second = sandbox.spawn_daemon();
        let mut gui = connect();
        let persisted = system_proxy::load_port().expect("read persisted port");
        assert_eq!(persisted, Some(bound.port), "the engine port was persisted");
        let again = arm(
            &mut gui,
            &domains,
            false,
            (persisted, Some(bound.relay_port)),
        );
        assert_eq!(
            again.port, bound.port,
            "the restarted helper must rebind the persisted MITM port"
        );
        assert_eq!(
            again.relay_port, bound.relay_port,
            "the restarted helper must rebind the relay port"
        );
        drop(gui);

        // When the persisted port is lost to another listener, the helper has to
        // bind elsewhere, and it must say so: every tool that read
        // `HTTPS_PROXY` at launch is now dialling a port Gate does not own, and
        // the helper's log is the only place that can name why (1972651).
        helper_client::shutdown_daemon();
        assert!(second.wait_for_exit().is_some(), "{}", second.log());
        let squatter = std::net::TcpListener::bind(("127.0.0.1", bound.port))
            .expect("take the persisted port as a stranger would");
        let third = sandbox.spawn_daemon();
        let mut gui = connect();
        let moved = arm(
            &mut gui,
            &domains,
            false,
            (persisted, Some(bound.relay_port)),
        );
        assert_ne!(
            moved.port, bound.port,
            "a port held by another listener must not be shared"
        );
        let wanted = format!("wanted the previous loopback port {}", bound.port);
        assert!(
            third.log().contains(&wanted),
            "the helper moved off the persisted port without saying so\n{}",
            third.log()
        );
        drop(squatter);
    }

    /// Concurrent control connections: while the GUI holds its connection, other
    /// callers must be served at once rather than queued behind it. In 5d244ee's
    /// parent the daemon served one connection to completion before accepting
    /// the next, so every `proxy status` waited out the client's 3s timeout and
    /// reported routing off while it was on.
    #[test]
    fn concurrent_control_connections_are_served() {
        let sandbox = Sandbox::new();
        let _daemon = sandbox.spawn_daemon();
        let domains = all_domains_on();
        let mut gui = connect();
        let bound = arm(&mut gui, &domains, false, (None, None));

        const CALLERS: usize = 8;
        let start = Arc::new(Barrier::new(CALLERS));
        let callers: Vec<_> = (0..CALLERS)
            .map(|i| {
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    let began = Instant::now();
                    let mut client = HelperClient::connect_existing()
                        .map_err(|e| format!("caller {i} could not connect: {e:#}"))?;
                    let status = client
                        .status()
                        .map_err(|e| format!("caller {i} got no status: {e:#}"))?;
                    // Held until every caller has its answer, so the
                    // connections genuinely overlap.
                    Ok::<_, String>((status, began.elapsed(), client))
                })
            })
            .collect();
        let results: Vec<_> = callers
            .into_iter()
            .map(|t| t.join().expect("caller thread"))
            .collect();

        for result in &results {
            let ((running, port, intercepting), took, _) = match result {
                Ok(r) => r,
                Err(e) => panic!("{e} while the GUI held its connection"),
            };
            assert!(*running && *port == Some(bound.port) && *intercepting == domains.len());
            // Far under the client's 3s timeout: queued behind the GUI, each
            // caller would have taken exactly that long and then failed.
            assert!(
                *took < Duration::from_secs(2),
                "a caller took {took:?}, as if queued behind another connection"
            );
        }

        // Callers leaving is not the last client leaving: the GUI is still here.
        drop(results);
        let (running, _, intercepting) = gui.status().expect("the GUI is still served");
        assert!(running);
        assert_eq!(
            intercepting,
            domains.len(),
            "other clients leaving must not disarm the GUI's engine"
        );

        // The flip side of the same rule: when the GUI, the last non-detached
        // client, leaves, the engine drops to pass-through.
        drop(gui);
        let deadline = Instant::now() + PROCESS_DEADLINE;
        loop {
            let (_, _, intercepting) = connect().status().expect("status after GUI left");
            // This probe is itself a client; the count covers it too, so the
            // drop already happened when the GUI's task ended.
            if intercepting == 0 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the engine stayed armed after its last client left"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Which manager call [`fresh_process_call`] makes, passed to the child in
    /// this variable.
    const FRESH_CALL_ENV: &str = "GATE_HELPER_TEST_FRESH_CALL";

    /// Run one `proxy::manager()` call in a fresh process, the way every CLI
    /// invocation runs it: a manager that has never held a control connection.
    ///
    /// The manager is a process-global singleton, so calling it from this
    /// process would carry a handle from one test into the next. The child is
    /// this same test binary, running only [`fresh_process_call`], against the
    /// sandbox this process points at. Returns the child's report line.
    fn in_fresh_process(sandbox: &Sandbox, call: &str) -> String {
        let out = Command::new(std::env::current_exe().expect("test binary path"))
            .args([
                "--exact",
                "linux::fresh_process_call",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(FRESH_CALL_ENV, call)
            .env("HOME", sandbox.path())
            // The drop-in `disable` removes lives under the config dir, which
            // the test home does not move. Never the developer's own.
            .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
            .stdin(Stdio::null())
            .output()
            .expect("run the fresh-process call");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "the fresh-process {call} failed ({})\nstdout: {stdout}\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        stdout
            .lines()
            // libtest prints the test's name on the same line first.
            .find_map(|l| l.split_once("FRESH: ").map(|(_, report)| report))
            .unwrap_or_else(|| panic!("the fresh-process {call} reported nothing\n{stdout}"))
            .to_string()
    }

    /// The child half of [`in_fresh_process`]. Ignored, so it runs only when
    /// asked for by name, with [`FRESH_CALL_ENV`] set.
    #[test]
    #[ignore = "run in a child process by in_fresh_process"]
    fn fresh_process_call() {
        let Ok(call) = std::env::var(FRESH_CALL_ENV) else {
            return;
        };
        let mgr = gate_connect_core::proxy::manager();
        // What the CLI does before any proxy subcommand, so a toggle it pushes
        // does not end with this process.
        mgr.set_detached(true);
        match call.as_str() {
            "status" => {
                let state = mgr.status().expect("status");
                println!("FRESH: running={} port={:?}", state.running, state.port);
            }
            "disable" => {
                mgr.disable().expect("disable");
                println!("FRESH: disabled");
            }
            other => {
                let slug = other
                    .strip_prefix("domain-on:")
                    .unwrap_or_else(|| panic!("unknown fresh-process call {other:?}"));
                mgr.set_domain(slug, true).expect("set_domain");
                println!("FRESH: enabled {slug}");
            }
        }
    }

    /// The state an enabled proxy leaves behind for the next process: an engine
    /// armed in the helper by a client that has gone (`proxy enable` arms
    /// detached and exits), and the routing snapshot on disk.
    fn armed_and_left(domains: &[ProxyDomain]) -> Intercepting {
        let mut cli = connect();
        let bound = arm(&mut cli, domains, true, (None, None));
        drop(cli);
        // Built rather than taken: `system_proxy::snapshot` would look at this
        // session's real config dir.
        system_proxy::save_snapshot(&system_proxy::ProxySnapshot {
            block_present: false,
            gnome_proxy: None,
        })
        .expect("record the routing snapshot");
        bound
    }

    /// Fix 53e6757, adoption half: a process with no handle of its own (every
    /// CLI invocation) must ask the helper rather than report its own `None`.
    /// Before it, `proxy status` printed "stopped" beside a live daemon.
    #[test]
    fn a_fresh_process_status_adopts_the_running_helper() {
        let sandbox = Sandbox::new();
        let _daemon = sandbox.spawn_daemon();
        let bound = armed_and_left(&all_domains_on());

        assert_eq!(
            in_fresh_process(&sandbox, "status"),
            format!("running=true port=Some({})", bound.port),
            "a fresh process must see the engine the helper is running"
        );
    }

    /// Fix 53e6757, adoption half: `proxy disable` from a fresh process must
    /// reach the helper and disarm it. Before it, disable removed the drop-in
    /// and the snapshot and left the engine intercepting.
    #[test]
    fn a_fresh_process_disable_disarms_the_running_helper() {
        let sandbox = Sandbox::new();
        let _daemon = sandbox.spawn_daemon();
        let domains = all_domains_on();
        let bound = armed_and_left(&domains);

        in_fresh_process(&sandbox, "disable");

        let (running, port, intercepting) = connect().status().expect("status after disable");
        assert_eq!(
            intercepting, 0,
            "a fresh-process disable must disarm the helper's engine"
        );
        assert!(running, "disable parks the engine, it does not stop it");
        assert_eq!(port, Some(bound.port));
    }

    /// Fix a390693: `proxy domain <slug> on` from a fresh process must reach the
    /// running engine, not only the domains file. Before it, the toggle wrote
    /// config and the engine kept its old rules until routing was cycled.
    #[test]
    fn a_fresh_process_domain_toggle_reaches_the_running_helper() {
        let sandbox = Sandbox::new();
        gate_connect_core::account::save(GATEWAY_URL, Some(API_KEY)).expect("save the account");
        let _daemon = sandbox.spawn_daemon();
        // The catalog's defaults, which is what the domains file says too.
        let defaults = default_domains();
        let armed = defaults.iter().filter(|d| d.enabled).count();
        let off = defaults
            .iter()
            .find(|d| !d.enabled && d.supported)
            .expect("a supported domain that is off by default")
            .slug
            .clone();
        armed_and_left(&defaults);

        in_fresh_process(&sandbox, &format!("domain-on:{off}"));

        let (running, _, intercepting) = connect().status().expect("status after toggle");
        assert!(running);
        assert_eq!(
            intercepting,
            armed + 1,
            "turning {off} on from a fresh process must arm it in the running engine"
        );
    }
}

/// macOS and Windows have no helper daemon: the engine lives in the process
/// that enabled it. Pinned so that assumption is checked rather than silently
/// skipped - `--proxy-helper` must be refused outright there, not start
/// anything. If a helper ever ships on these platforms, this fails and the
/// Linux tests above should be widened to cover it.
#[cfg(not(target_os = "linux"))]
#[test]
fn no_helper_daemon_outside_linux() {
    let home = tempfile::TempDir::new().expect("temp home");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_gate-connect"))
        .arg("--proxy-helper")
        .env("GATE_CONNECT_TEST_HOME", home.path())
        .env("GATE_CONNECT_TEST_SECRETS", home.path().join("secrets"))
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run gate-connect --proxy-helper");
    assert!(
        !out.status.success(),
        "--proxy-helper must be rejected where no helper exists, got {}\nstdout: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--proxy-helper"),
        "expected an argument error naming the flag, got: {stderr}"
    );
}
