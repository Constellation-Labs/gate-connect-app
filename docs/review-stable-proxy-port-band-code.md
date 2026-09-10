# Code quality review: fix/stable-proxy-port-band

- Branch: `fix/stable-proxy-port-band` (5 commits)
- Base: `main`
- Files: `crates/core/src/proxy/engine.rs` (+160/-5), `crates/core/src/proxy/relay.rs` (+7/-5), `crates/core/tests/proxy_e2e.rs` (+44/-20)
- Lens: code quality only (readability, naming, dead code, duplication, comment hygiene, test quality)

## Findings

### M - Stale "falls back to an ephemeral port" comments contradict the new fallback behavior

Multiple doc comments still describe the pre-branch behavior, where losing or lacking a
preferred port meant an OS-ephemeral `:0` bind. Since `bind_loopback` now falls back to
`bind_fresh` (the 47100..47200 band), these are wrong on exactly the axis this branch
changes, and this codebase leans heavily on rationale comments. Sites:

- `crates/core/src/proxy/engine.rs:64-65` - `EngineConfig::preferred_port` doc: "Falls back to an ephemeral port if `p` is taken (or `None`)."
- `crates/core/src/proxy/engine.rs:608` - `bind_loopback` doc: "falls back to an ephemeral port."
- `crates/core/src/proxy/engine.rs:722` and `crates/core/src/proxy/engine.rs:745` - `bind_preferred` doc and inline comment: "silently falls back to an ephemeral port" / "let the caller fall back to an ephemeral port."
- `crates/core/src/proxy/engine.rs:830` - `start` doc: "Start the engine on an ephemeral loopback port."
- `crates/core/src/proxy/relay.rs:55` - `port_path` doc: "only falls back to a fresh ephemeral port if it's taken."
- `crates/core/tests/proxy_e2e.rs:793` - the restart test's doc comment still says "fall back to an ephemeral one" even though the same commit updated that test's assertion message to "fall back to another one."

The new comments introduced by the branch (`STABLE_PORT_RANGE`, `bind_fresh`,
`candidate_ports`, `persisted_ports`, the relay `bind_relay` doc) are accurate against
the code; it is the pre-existing neighbors that were left behind.

### M - `seed_preferred_port` doc cites a concurrency mechanism that does not occur under the project's own test runner

`crates/core/tests/proxy_e2e.rs:864-867` justifies seeding from the ephemeral range with
"notably `relay_e2e`, which also brings engines up while cargo runs the two binaries
concurrently." Plain `cargo test` runs test binaries sequentially (tests are parallel
only within one binary), and CI uses exactly that (`.github/workflows/ci.yml:58`,
`cargo test --workspace`); there is no nextest configuration anywhere in the repo. So
the named race between the two binaries cannot happen under the repo's own runner. The
change itself is still worthwhile - a live Gate Connect install on the developer's
machine scans the same band, and a future switch to nextest would make the cited race
real - but the comment should name a mechanism that actually exists today.

### L - `bind_fresh` leaves the band silently

`crates/core/src/proxy/engine.rs:669-676`: every failed band bind (including
non-AddrInUse errors such as EACCES from a local firewall rule) is swallowed, and the
function drops to a `:0` ephemeral bind with no log line. That fallback recreates, for
the next restart, exactly the moving-port failure the branch fixes, and it is the only
quiet transition on this path - `bind_loopback` already eprintlns when the preferred
port is lost (`crates/core/src/proxy/engine.rs:630-637`). A one-line warning in the same
style would make the condition diagnosable. Rated L because reaching it needs the whole
100-port band unavailable, which is rare.

### L - Dangling empty doc line

`crates/core/src/proxy/engine.rs:668`: a bare `///` sits between the end of the
`bind_fresh` rationale and the `pub(super) fn` line, leaving a trailing blank paragraph
in the rendered doc.

### L - Persisted-port names hardcoded in a fourth place

`crates/core/src/proxy/engine.rs:710`: `persisted_ports` repeats the `"port"` and
`"pac-port"` string literals that the three platform `system_proxy*` modules already
hardcode (`crates/core/src/proxy/system_proxy.rs:65-84`,
`system_proxy_windows.rs:72-90`, `system_proxy_linux.rs:78-83`). A rename on the
platform side would compile fine and silently stop the self-steal deferral, since
`load` degrades missing files to `None`. The pattern predates the branch, but this is a
new coupling site; a shared constant next to `port_persist` would remove the hazard.

## Checked and fine

- `candidate_ports` arithmetic: `offset + i` peaks at 198, well inside `u16`; the rotation covers the band exactly once; `partition` defers rather than drops; all of this is pinned by the unit tests at `crates/core/src/proxy/engine.rs:1097-1123`.
- Unit test parallel-safety: the tests bind at most three live band ports at once against a 100-port band with a random start, so intra-binary parallelism and a concurrently running Gate install do not realistically exhaust it; `candidate_ports_starts_somewhere_different` (engine.rs:1126) has a false-failure probability of (1/100)^7.
- `bind_fresh` calling `persisted_ports` means the unit tests read the developer's real persisted port files via `app_support_dir` (engine.rs:709-716); this is read-only, best-effort, and no assertion depends on the content, so it is acceptable, just worth knowing.
- `seed_preferred_port` TOCTOU (proxy_e2e.rs:871-874): the bind-note-drop window is milliseconds, Linux and macOS rotate ephemeral allocation rather than reissuing the just-freed port, and the `assert_eq` at proxy_e2e.rs:814 names the race explicitly if it ever fires. Acceptable for a test helper.
- The reworked fallback blocks (proxy_e2e.rs:843-855 and the PAC twin) fix a real flake source: the blocker now holds its own `:0` port instead of racing to reclaim the port the first run just freed, and the assertion messages were updated consistently.
- `pub(super)` on `bind_fresh` correctly reaches `relay` as a sibling under `proxy`; the relay swap at `crates/core/src/proxy/relay.rs:143` is minimal and its updated doc (relay.rs:120-129) matches the code.
- `rand` was already a workspace dependency (`crates/core/Cargo.toml:37`); the new `use rand::Rng` is the only import change. No dead code introduced.
- The long rationale comments on `STABLE_PORT_RANGE`, `bind_fresh`, and `candidate_ports` (engine.rs:645-703) are accurate: Windows' default dynamic range is 49152-65535, 47001 is WinRM, the code does try the whole band before `:0`, and the deferral behavior matches the described three-listener bind order (engine.rs:851-862).
- `docs/oauth-authentication-impl-plan.md:118` mentions the old ephemeral bind pattern, but it is a historical planning document with an already stale line reference, not living guidance.
- Verified green: `cargo test -p gate-connect-core --lib proxy::engine` (9 pass), `cargo test -p gate-connect-core --test proxy_e2e restart` (1 pass on Linux; the PAC twin is cfg macos/windows), `cargo clippy -p gate-connect-core --all-targets` clean.

## Verdict

Solid, well-tested change with accurate new documentation; the must-do cleanup before merge is sweeping the six leftover "ephemeral fallback" comments and correcting the concurrent-binaries claim in `seed_preferred_port`.
