# Security review: fix/stable-proxy-port-band

Base: main. Branch: fix/stable-proxy-port-band. Lens: security only.

Changed files reviewed:
- crates/core/src/proxy/engine.rs (bind_loopback, bind_fresh, candidate_ports, persisted_ports, bind_preferred, STABLE_PORT_RANGE)
- crates/core/src/proxy/relay.rs (bind_relay, serve, peer_allowed, credential injection path)
- crates/core/tests/proxy_e2e.rs (restart tests, seed_preferred_port)

Context read for the threat model: crates/core/src/proxy/port_persist.rs, crates/core/src/proxy/mod.rs (inject_gate_credential), crates/core/src/proxy/engine.rs (peer_uid_for, GateHandler::peer_allowed), the EngineConfig owner_uid construction sites (helper.rs:319 Linux, manager.rs:251 macOS None, manager_windows.rs:256 None), and how clients bake the port (integrations/claude_code.rs:8-15, integrations/hermes.rs:7-9).

## What the change does

Fresh (first-run) and fallback loopback binds move from OS-ephemeral `bind(("127.0.0.1", 0))` to picking the first free port in a fixed 100-port band, `STABLE_PORT_RANGE = 47100..47200` (engine.rs:650), with a random rotation offset and this install's own persisted ports deferred to the tail (engine.rs:697-702). This applies to the MITM, PAC, and relay listeners via bind_loopback (engine.rs:613-630, called at engine.rs:851/856/860) and to the standalone relay's first-run path (relay.rs:144). The engine-hosted relay falls back into the band when the persisted port is taken (bind_loopback); the standalone `proxy relay` still errors rather than falling back (bind_relay, relay.rs:136-143).

## Findings

### High
None.

### Medium
None.

### Low

L1. Predictable band makes a same-machine denial-of-service trivial where the old ephemeral pick did not. A non-privileged local user can bind all 100 ports of 47100..47199 before Gate starts. Effects: the engine-hosted listeners fall through bind_fresh's loop (engine.rs:663-670) to an OS-ephemeral port, which then is not stable across runs and forces a config rebake; the standalone `proxy relay` bind_relay returns an error and refuses to start when its persisted port (now itself a band port) is held (relay.rs:136-143). On main the fresh/fallback pick was an unpredictable ephemeral port, so blanket pre-binding was not feasible; a squatter could only target the one known persisted port. The band widens that to the whole first-run and fallback space. This is a local-user DoS only: it does not capture credentials (see "checked and clear"), needs an already-hostile co-resident user, and grants no privilege escalation. Rated L because loopback DoS by a co-resident user is largely an accepted property of same-machine listeners, and the impact is a failed start or an unstable port, not disclosure.

## Checked and clear

- No credential capture is newly enabled by the band. What a client sends the relay is the client's own upstream bearer (e.g. Claude Code's `ANTHROPIC_BASE_URL` points at `http://127.0.0.1:<port>/anthropic` and it sends its own Authorization, integrations/claude_code.rs:8-15). The Gate credential is injected outbound by the relay toward the gateway (mod.rs:418-451, relay.rs:539-547), never sent by the client, so a squatter impersonating the relay would receive the tool's own upstream token, not the Gate credential. This exposure exists on main too and is unchanged.

- No co-binding / shadowing via SO_REUSE on the new path. bind_fresh uses a plain exclusive `TcpListener::bind` with no reuse flag (engine.rs:663-670), so a squatter cannot share Gate's live port. bind_preferred sets SO_REUSEADDR only after a connect probe confirms the port is not live (engine.rs:738-758); a live squatter is detected and returns AddrInUse, sending the caller to fallback rather than binding over it. The narrow macOS wildcard probe-to-bind shadow race is documented at engine.rs:729-731 and is pre-existing, not introduced here.

- Gate always persists and bakes exactly the port it actually bound (engine.rs:851-861 then persist; relay.rs:344-345). Tool configs therefore point at the live Gate listener, so an attacker holding other band ports cannot be the target of a fresh connect. Capture still requires the pre-existing window (Gate stops, squatter grabs the freed persisted port, tool connects while Gate is down); the band does not change that window for persisted ports, which were already stable known targets on main.

- Fallback-into-band (item 2 of the brief) does not worsen the moved-port story in a security-relevant way. The new port is held live by Gate; the freed old port is the same known persisted value it was on main. relay.rs:116-133 keeps the "taken preferred port is an error, not a silent move" contract for the standalone host, which is the safer choice.

- rand usage (item 3) is a cosmetic rotation offset only (engine.rs:698-700). Predictability of the band is inherent (100 ports); the offset does not add or remove any security property, and `thread_rng` is a CSPRNG regardless.

- Peer authorization is unchanged. Linux still gates on owner UID and fails closed (relay.rs:249-254, engine.rs peer_uid_for/peer_allowed; helper.rs:319 sets owner_uid). macOS and Windows still pass owner_uid: None (manager.rs:251, manager_windows.rs:256), meaning any local user reaching the loopback listener is served. That is pre-existing accepted design (loopback is same-machine-reachable by all local users), not a product of this branch.

- Test changes (item 4) do not weaken isolation or leak credentials. seed_preferred_port binds `:0`, reads the port, and releases it (proxy_e2e.rs:291-294); no credential is handled. The blocker now holds a `:0` port of its own instead of racing to reclaim the just-freed one (proxy_e2e.rs:266-267, 328-329), which reduces flakiness. Seeding preferred ports from the ephemeral range deliberately keeps the restart tests out of the shared band that relay_e2e also draws from, improving isolation rather than reducing it. Tests still route through the mock gateway via the GATE_CONNECT_TEST_* seams (relay.rs:89-114).

## Verdict

No must-fix or should-fix security issues; the only new effect is a low-severity, local-user denial of service from the now-predictable port band, and the dominant loopback exposure (macOS/Windows serve any local peer) is pre-existing and unchanged.
