# Correctness review: fix/stable-proxy-port-band

Scope: `git diff main...HEAD` on branch `fix/stable-proxy-port-band` (base `main`).
Changed files: `crates/core/src/proxy/engine.rs`, `crates/core/src/proxy/relay.rs`,
`crates/core/tests/proxy_e2e.rs`. Lens: correctness only. Supporting reads:
`port_persist.rs`, `system_proxy*.rs`, `manager.rs` / `manager_linux.rs` /
`manager_windows.rs`, `relay.rs` (`serve`, `bind_relay`), `tests/relay_e2e.rs`.
Verified locally: `cargo test -p gate-connect-core --lib` (173 pass) and the
Linux-visible restart e2e test pass.

## Trace results for the numbered checks

1. candidate_ports math (engine.rs:697-703). `span = STABLE_PORT_RANGE.len() as u16`
   is 100; `offset` in 0..100; `offset + i` maxes at 198, no u16 overflow; max
   emitted port is 47100 + 99 = 47199, inside the band. The modular rotation is a
   bijection over 0..span, so each band port appears exactly once (unit test
   `candidate_ports_covers_the_band_exactly_once` confirms). `Iterator::partition`
   pushes items into each collection in iteration order, so both the front half and
   the deferred tail keep the rotation's relative order. Correct.

2. persisted_ports (engine.rs:709-716). Names match the writers exactly: `"port"`
   is written by `system_proxy::save_port` (system_proxy.rs:70,
   system_proxy_windows.rs:77, system_proxy_linux.rs:83), `"pac-port"` by
   `save_pac_port` (system_proxy.rs:84, system_proxy_windows.rs:90), and
   `relay::load_persisted_port` reads the same `proxy/relay-port` file that
   `save_persisted_port` writes (relay.rs:57-79). `load` returns Ok(None) for a
   missing file and Err for other IO failures; `.ok().flatten()` degrades both to
   "not deferred", which only weakens deferral, never breaks binding. Upgrade case
   (old ephemeral persisted ports like 63854): those ports are outside the
   rotation, so `contains` is false and they are simply absent from the candidate
   list, which is fine because a band pick can never collide with an out-of-band
   port. On the transition run each listener either reclaims its old ephemeral
   port via `bind_preferred` (deferral moot) or falls into the band; a listener
   that falls into the band cannot steal another listener's out-of-band persisted
   port, and in-band persisted ports (from prior runs of this code) are deferred.
   Nothing is stolen in any mix.

3. Sequential binds in one startup (engine.rs:851-861). Order is MITM, relay,
   then PAC (mac/win only). All three `std::net::TcpListener`s stay bound in
   local scope until moved into the runtime, so a later `bind_fresh` fails its
   plain bind on any port an earlier listener holds and skips it. On a true first
   run (nothing persisted, nothing to defer) collision is prevented by the held
   listeners, not by deferral. Correct.

4. Concurrent app + standalone `proxy relay`. Both processes resolve the same
   persist files via `app_support_dir` (port_persist.rs:14-16, relay.rs:57-61),
   so the CLI's `bind_fresh` defers the app's persisted MITM/PAC/relay ports.
   When the app is live its ports are held, so the CLI's plain bind fails and
   skips them; `bind_fresh` binds directly rather than probe-then-bind, so the
   only cross-process race is the atomic bind itself, where the loser just moves
   to the next candidate. First-run app ports that were never persisted are
   protected while live and do not exist while the app is stopped. Additionally
   `relay::serve` refuses to start at all when the engine looks live
   (relay.rs:325-335). No theft path found.

5. Band exhaustion (engine.rs:669-675). The loop tries all 100 candidates, then
   falls back to `bind ("127.0.0.1", 0)` returning io::Result; callers wrap with
   context and propagate. No panic, no infinite loop.

6. Persistence vs served port. Persistence happens only after `engine::start`
   returns, from the running handle's actual bound ports: manager.rs:262-266,
   manager_windows.rs:267-271, manager_linux.rs:199-200 (from the daemon's
   reported bound ports), and relay.rs:344-345 for the standalone host. A
   preferred-taken fallback into the band therefore persists the new band port,
   not the old one. Within one startup, `persisted_ports()` still returns the
   stale pre-fallback values for the later binds, which is harmless: the newly
   bound port is protected by its held listener, and deferring the stale value
   is at worst over-cautious.

7. Tests. `seed_preferred_port` (proxy_e2e.rs:871-874) drops the probe before
   the engine binds; see finding L1. The probe never accepts a connection, so no
   TIME_WAIT remnant blocks the engine's plain bind. The blocker change (bind :0
   instead of the just-freed port, proxy_e2e.rs:846-848, 928-930) removes a real
   race and the e2e now asserts only "fell back to a different port"; the
   band-landing property is covered by the unit test
   `bind_loopback_falls_into_the_band_when_the_preferred_port_is_taken`
   (engine.rs:1138-1148), so coverage is preserved, just relocated. The new unit
   tests bind real band ports but take the first free one, so they cannot
   collide with each other or with concurrently running e2e engines; see finding
   L2 for the residual exhaustion-flake margin.

8. Windows cfg structure (engine.rs:737-763). `#[cfg(unix)] bind_preferred` does
   plain bind, then a connect probe, then the SO_REUSEADDR rebind;
   `#[cfg(not(unix))]` is a plain bind only. That matches the doc comment's
   claims (Windows defaults allow the rebind; SO_REUSEADDR on Windows would
   permit hijack). Consistent.

## Findings

No high-severity findings.

### L1: seed_preferred_port has a TOCTOU window (accepted trade-off)
proxy_e2e.rs:871-874. The probe listener is dropped when the function returns,
so between the drop and `engine::start` binding it, any other local process can
take the port. The risk is low (OS ephemeral assignment cycles rather than
reissuing immediately, and the suite's other binaries now bind :0 or the band)
and the failure mode is the explicit `assert_eq!(engine.port(), port)` message
at proxy_e2e.rs:817, not a silent wrong-test. There is no better option short of
handing over a live listener, which `EngineConfig` does not support. Acceptable.

### L2: band-membership asserts share machine-global state with the e2e suite
engine.rs:1071-1080, 1083-1094, 1138-1148 assert the bound port lands inside
47100..47200. The band is machine-global, `bind_fresh` uses a plain bind (no
SO_REUSEADDR), and every e2e engine that served traffic can leave its 2-3 band
ports in TIME_WAIT for about a minute after stop. proxy_e2e starts 13 engines
and relay_e2e 2 more; on mac/win that is roughly 45 band ports per full run, so
two full-suite runs inside one TIME_WAIT window plus an unrelated squatter could
in principle exhaust the band and flake these asserts. The margin today is
about 2x and the band was chosen to be quiet, so this is a note, not a fix
request.

### L3: bind_fresh swallows non-AddrInUse bind errors
engine.rs:670-674. `if let Ok(listener)` treats every bind failure as "port
busy". Under fd exhaustion (EMFILE) it makes 100 doomed attempts and the error
the caller finally sees is the `:0` bind's, with the fallback's context string.
Behavior is still correct (error propagates, no panic); only the diagnosis is
slightly indirect. Nit.

### L4 (pre-existing, unchanged by branch): embedded relay fallback still repoints the persisted relay port
engine.rs:855-856 falls back to `bind_fresh` when the preferred relay port is
taken, and manager.rs:266 / manager_windows.rs:271 / manager_linux.rs:200 then
persist the new port, stranding tool configs that baked the old URL. The
standalone host refuses exactly this (relay.rs:119-129, 134-143). The branch
only changes where the fallback lands (in-band, which is strictly better);
noting for completeness, not against this branch.

### L5 (pre-existing doc nit): stale claim that only Linux persists the MITM port
relay.rs:55-56 says the MITM port is persisted "only Linux", but the macOS and
Windows managers persist it too (manager.rs:262, manager_windows.rs:267).
Harmless to `persisted_ports()`, which reads it on all platforms anyway.

## Verdict

Correct: the rotation, deferral, bind sequencing, persistence ordering, and
cross-process behavior all check out, with only low-severity notes and two
pre-existing observations.
