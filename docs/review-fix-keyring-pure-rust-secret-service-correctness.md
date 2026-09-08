# Review: `fix/keyring-pure-rust-secret-service` - correctness lens

**Commit under review:** `b4636c6` "Stop the app aborting on launch: drop the C libdbus Secret Service client"
**Base:** `origin/feat/new-app-ui` (the diff vs `main` is inherited integration work and is out of scope)
**Scope:** `Cargo.toml` (keyring feature list + comment) and `Cargo.lock` (381 lines of resulting churn). No Rust source changed.

Lens: is the fix correct and complete, and does it introduce a new failure mode.

**Verdict: the fix is correct and it works. No H findings.** Every load-bearing technical claim in the
Cargo.toml comment checks out against the crate sources, the tree builds, the vendored C is
demonstrably gone from the shipped binary, and the root cause is identifiable in the vendored build
script. The findings below are about the *explanation* being wrong in ways that will mislead the next
person, plus two genuine behaviour changes the commit does not mention.

---

## What I verified as correct

**1. The API really is still synchronous.** `keyring-3.6.3/src/secret_service.rs:94-98`:

```rust
#[cfg(not(feature = "async-secret-service"))]
use dbus_secret_service::{Collection, EncryptionType, Error, Item, SecretService};
#[cfg(feature = "async-secret-service")]
use secret_service::{
    blocking::{Collection, Item, SecretService},
    EncryptionType, Error,
};
```

The two clients are swapped behind one import block, and every `CredentialApi` method below it is a
plain `fn` (`set_password` :137, `get_password` :176, `delete_credential` :211). `Entry` stays sync,
`crates/core/src/keychain.rs` needs no change, and `cargo check` confirms it (below). The comment's
"'async' names the transport, not our call sites" is accurate.

**2. `async-io` over `tokio` is the right choice, and for the stated reason.**
`zbus-4.4.0/src/utils.rs:31-49` is the whole argument:

```rust
#[cfg(not(feature = "tokio"))]
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    async_io::block_on(future)
}

#[cfg(feature = "tokio")]
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    static TOKIO_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let runtime = TOKIO_RT.get_or_init(|| ... new_current_thread() ... );
    runtime.block_on(future)
}
```

With `tokio`, a keychain read becomes `Runtime::block_on` on a private current-thread runtime, which
panics ("Cannot start a runtime from within a runtime") when the calling thread is inside a tokio
runtime. With `async-io` it is `async_io::block_on`, which parks the calling thread against
async-io's own reactor: no panic, no deadlock, just a blocked thread. Note the `tokio` cfg *wins* if
both features are ever on.

**3. The features are inert on macOS and Windows.** `keyring-3.6.3/Cargo.toml` declares
`secret-service`, `zbus` and `dbus-secret-service` only under
`[target.'cfg(target_os = "linux")']` (plus freebsd/openbsd), and all three new features are weak
references into them: `async-secret-service = ["dep:secret-service", "dep:zbus"]`,
`async-io = ["zbus?/async-io"]`, `crypto-rust = ["dbus-secret-service?/crypto-rust", "secret-service?/crypto-rust"]`.
Measured: `cargo tree --locked --target x86_64-apple-darwin -i secret-service` and the same for
`zbus@4.4.0` both report "nothing to print", while
`cargo tree --locked --target x86_64-pc-windows-msvc -e features -i keyring` shows the four features
enabled on `keyring` with no zbus/secret-service edge underneath. Backend selection on those targets
is unaffected: `keyring-3.6.3/src/lib.rs:274` and `:293` gate `macos`/`windows` on
`target_os` + `apple-native`/`windows-native` only, and the `secret_service` module and its
`pub use secret_service as default` (`lib.rs:217-235`) are gated on the *nix target list. The
`compile_error!` at `lib.rs:195-202` fires only if `sync-secret-service` and `async-secret-service`
are both on, which they are not.

**Dropping `vendored` cannot regress macOS or Windows.** Old `vendored = ["dbus-secret-service?/vendored", "openssl?/vendored"]`.
Both arms are weak, neither crate exists on those targets, and nothing in keyring's feature graph
ever activates `dep:openssl` (`crypto-openssl` only forwards into the two secret-service crates). So
on macOS and Windows `vendored` was already a no-op.

**4. Build reality: clean.**
- `cargo metadata --locked` exits 0, so `Cargo.lock` is consistent with `Cargo.toml`.
- `cargo check --locked -p gate-connect-core --all-targets` exits 0 (builds `zbus v4.4.0`,
  `secret-service v4.0.0`, `keyring v3.6.3`).
- `cargo check --locked -p gate-connect-cli` exits 0.
- `dbus-secret-service` still appears in `Cargo.lock:995` and in keyring's dep list at
  `Cargo.lock:2628`, which looks alarming but is only a resolution artefact of the weak
  `dbus-secret-service?/crypto-rust` reference. It is not built:
  `cargo tree --locked -i dbus-secret-service` and `--target all -i dbus-secret-service` both report
  "nothing to print".

**5. The vendored C is gone from the shipped binary.** `ldd target/debug/gate-connect-desktop`
(built 12:54, after the 11:35 commit) resolves `libdbus-1.so.3 => /lib/x86_64-linux-gnu/libdbus-1.so.3`,
and `strings -a` finds **zero** `vendor/dbus/dbus` paths and no `dbus-sysdeps-unix` reference. The CLI
has no libdbus path at all: `cargo tree --locked -i libdbus-sys` shows the only route is
`libdbus-sys -> dbus 0.9.11 -> tao 0.35.3 -> tauri-runtime-wry -> tauri -> gate-connect-desktop`.

**6. The error variant `keychain.rs` matches survives the swap.** `crates/core/src/keychain.rs:122`
and `:147` match `keyring::Error::NoEntry`. That variant is produced by keyring's *own* backend code
(`secret_service.rs:348`, `:407`), not by either client's error enum, and those lines are not
cfg-gated per client. `decode_error` (`secret_service.rs`) maps `Locked | NoResult | Prompt` to
`NoStorageAccess` and everything else to `PlatformFailure` for both clients, and both client error
enums carry `Locked`, `NoResult`, `Prompt` and `Unavailable`. There is **no** error-string matching
anywhere in `crates/core/src/account.rs` (`grep` for `contains(` / `starts_with(` finds only URL
scheme checks at `:185` and `:194`). So the CLAUDE.md hazard - a failed key read silently
repointing the user at the default gateway - is not reachable through a changed error shape: on a
keychain error `account::load()` (`account.rs:109`) and `reconcile()` (`account.rs:533-534`)
propagate with `?` and delete nothing.

**7. Call-site classification for the re-entrancy question.** Every keychain path in the desktop app
is safe under either runtime choice, and there is exactly one that is not:

| Call site | Thread | Safe with `tokio`? |
| --- | --- | --- |
| `src-tauri/src/lib.rs:285` `get_account` (sync command -> `account::reconcile`, `has_api_key`) | main/GTK thread (`ExecutionContext::Blocking`, `tauri-macros-2.6.2/src/command/wrapper.rs:266`) | yes |
| `lib.rs:328`, `:337` key-prefix commands | main thread, same | yes |
| `lib.rs:192, 239, 401, 434, 474, 550, 577, 587, 617, 648, 823, 998, 1015, 1124, 1234, ...` (34 `spawn_blocking` sites) | tokio blocking pool | yes - a blocking-pool thread is not in a scheduler context |
| `lib.rs:4384` OAuth refresh loop | `std::thread::spawn` | yes |
| **`crates/core/src/proxy/relay.rs:377-379` and `:436-441`** `access_token_for_injection()` -> `oauth::current()` -> `keychain::get` | **inside `rt.block_on(...)` on a multi-thread tokio runtime** | **no - would panic** |
| `lib.rs:1065` `#[tauri::command(async)] fn diagnostics` | tokio worker (`respond_async_serialized` -> `async_runtime::spawn`, `tauri-2.11.2/src/ipc/mod.rs:343-375`) | n/a - `diagnostics.rs` touches no keychain |

The proxy engine does not read the keychain per request: the key and token are seeded into
`watch::channel`s (`proxy/engine.rs:1679`, `:1702`, `relay.rs:287`) and pushed by `refresh_token`.

---

## Findings

### M-1. The comment's causal story is inverted: dropping `vendored` is the fix, not the client swap

The comment frames the client swap as the crash fix and dismisses the `vendored` removal as
janitorial ("**`vendored` is dropped because it becomes inert**"). The evidence says the opposite.

Root cause, verified in the vendored build script: `libdbus-sys-0.2.7/build_vendored.rs` enables
`DBUS_HAVE_LINUX_EPOLL` and `HAVE_EPOLL` (`:303-304`) and **never defines `HAVE_POLL`**
(`grep -c 'HAVE_POLL"' build_vendored.rs` -> 0). The bundled dbus 1.14.4 guards `_dbus_poll` on
exactly that macro - `vendor/dbus/dbus/dbus-sysdeps-unix.c:3146`:

```c
#if defined(HAVE_POLL) && !defined(BROKEN_POLL)
...
#else /* ! HAVE_POLL */
```

so the vendored build compiles the `#else` branch, which builds three stack-allocated `fd_set`s and
does unbounded `FD_SET (fdp->fd, &read_set)` (`:3170-3180`) before `select()`. An `fd_set` is 128
bytes covering fds 0..1023; a `FD_SET` of any fd >= `FD_SETSIZE` writes past it on the stack. That is
`__stack_chk_fail` inside `_dbus_poll` reached from `socket_do_iteration`, exactly the reported stack,
and it explains the reported timing (a Tauri/WebKit process crosses fd 1024 seconds into launch, not
at startup) and why the distribution's copy - which defines `HAVE_POLL` and calls `poll()` - never
does it.

The consequence: `libdbus-sys` is a single package shared by keyring's old path
(`dbus-secret-service -> dbus -> libdbus-sys`) and tao's live path (`tao -> dbus -> libdbus-sys`), and
`keyring/vendored` expanded to `dbus-secret-service/vendored` -> `dbus/vendored`
(`dbus-secret-service-4.1.0/Cargo.toml`) -> `libdbus-sys/vendored` (`dbus-0.9.11/Cargo.toml`), so
feature unification made *both* callers use the miscompiled copy. Removing `vendored` alone -
keeping `sync-secret-service` - would have flipped `libdbus-sys` to its `pkg-config` branch
(`build.rs:16-27`) and fixed the crash. The client swap is defensible hardening, but it is not what
fixes the bug, and it carries the larger blast radius (new crypto handshake, new runtime, new error
text - see M-3, L-2, L-3).

Two things follow that the comment should say instead of "inert": that `libdbus-sys/vendored` via
`tao -> dbus` is what actually changed, and that the vendored build's missing `HAVE_POLL` is the
defect. As written, someone bisecting this or trying to reduce the change would revert the wrong half.

Related and worth stating plainly: because the vendored copy served both callers, the commit's
attribution of the crash to keyring's connection rests on 5 + 5 launch samples plus the
`GATE_CONNECT_TEST_SECRETS` correlation. Under this root cause, tao's own portal connection
(`tao-0.35.3/src/platform_impl/linux/portal.rs`, driven from `event_loop.rs:288`, enabled by
`tauri`'s default `dbus` feature) was equally exposed. It does not change the verdict - both callers
are fixed - but the attribution is not pinned down, and the claim "whenever the real Secret Service
was reached" is stronger than the sample supports.

### M-2. The re-entrancy example in the comment is the one call shape this repo does not have

The comment justifies `async-io` with "a blocking call from inside a tokio runtime panics - which
`#[tauri::command(async)]` handlers would do." That specific claim is wrong for this repo, twice
over:

- Every keychain-touching Tauri command routes through `tauri::async_runtime::spawn_blocking`
  (34 sites; `src-tauri/src/lib.rs:192, 239, 401, 434, 474, 550, 577, 587, 617, 648, ...`), and a
  tokio blocking-pool thread carries the runtime *handle* but not a scheduler context, so
  `Runtime::block_on` there does not panic. Blocking is what that pool is for.
- The only `#[tauri::command(async)]` on a sync fn is `diagnostics` (`lib.rs:1065`), which does run
  its body on a tokio worker (`tauri-macros-2.6.2/src/command/wrapper.rs:249` -> `body_async` ->
  `tauri-2.11.2/src/ipc/mod.rs:343` -> `async_runtime::spawn`) - but
  `crates/core/src/diagnostics.rs` never touches the keychain.

The real hazard is `crates/core/src/proxy/relay.rs`: `serve()` builds a multi-thread runtime at
`:370-373` and, inside `rt.block_on(...)`, calls
`crate::oauth::access_token_for_injection()` at `:377-379` and again in the refresh loop at
`:436-441`. That reaches `oauth::current()` (`crates/core/src/oauth.rs:472`) -> `keychain::get` ->
`secret_service::blocking` -> `zbus::block_on`, from inside a tokio runtime context. With
`keyring/tokio` that call panics; with `async-io` it blocks the root-future thread, which is
correct-if-slow (the spawned `accept_loop` keeps serving on the workers).

Why this matters beyond pedantry: the comment is the only thing standing between a future maintainer
and `keyring/tokio` (e.g. "we already depend on tokio, let's not carry two reactors"). Someone
checking whether the stated hazard still applies would grep the commands, find `spawn_blocking`
everywhere, conclude the hazard is gone, and land a panic in `gate-connect proxy relay`. Point the
comment at `relay.rs` instead. Also note that `secret-service`'s own module doc says the same thing
(`secret-service-4.0.0/src/blocking/mod.rs:14-17`: "It is important to not call these functions in an
async context or otherwise the runtime may stall").

### M-3. Undocumented behaviour change: the Secret Service session goes from Plain to DH-encrypted

`crypto-rust` is new - the old feature list had no crypto feature at all. That flips the session
type in `keyring-3.6.3/src/secret_service.rs:140-143`:

```rust
#[cfg(any(feature = "crypto-rust", feature = "crypto-openssl"))]
let session_type = EncryptionType::Dh;
#[cfg(not(any(feature = "crypto-rust", feature = "crypto-openssl")))]
let session_type = EncryptionType::Plain;
```

So before this commit, secrets crossed the session bus in the clear; now they are negotiated over
`dh-ietf1024-sha256-aes128-cbc-pkcs7`. That is a security improvement and worth having, and given
CLAUDE.md's "credentials are the product" it deserves a line in the comment rather than being smuggled
in as a build-graph detail.

The correctness cost is a new failure surface: `SecretService::connect(Dh)` now performs a key
exchange that can fail where `Plain` could not, and `secret-service-4.0.0/src/error.rs` gains
`Error::Crypto(&'static str)`, which `decode_error` funnels into `PlatformFailure`. gnome-keyring and
KWallet implement DH, and the commit's own 15 round-trips plus the read-back of the pre-existing
libdbus-written credential prove the local daemon is fine - but a minimal Secret Service
implementation that only advertises `plain` would now fail to open a session. Unverified for
implementations other than the one on the test machine.

### L-1. `crates/core/src/keychain.rs:6` still documents the backend this commit deleted

```rust
//! - Linux Secret Service (via `sync-secret-service`, vendored libdbus)
```

Both halves are now false, and it is the module doc of the very file the Cargo.toml comment points at
when it says "`keychain.rs` needs no change". The code needed no change; this line did.

### L-2. The commit message's OpenSSL claim is wrong

"OpenSSL leaves the tree entirely (openssl, openssl-sys, openssl-src, vcpkg, foreign-types), so we
stop vendoring a second C library." OpenSSL was never compiled, let alone vendored. Nothing in the
old feature set activated `dep:openssl`: keyring's `crypto-openssl` only forwards weakly into the two
secret-service crates, `dbus-secret-service`'s `crypto-openssl` was not enabled (the old list had no
crypto feature), and `keyring/vendored`'s `openssl?/vendored` arm is weak. The `keyring -> openssl`
edge in the old lock (`Cargo.lock` at `origin/feat/new-app-ui`) is a lockfile artefact of that weak
reference, which is precisely why removing `vendored` made it disappear. The five packages leaving the
lock is real; "we stop vendoring a second C library" is not. Cargo.toml's own wording is closer but
still attributes the removal to `crypto-rust`, which is not the mechanism either.

### L-3. New failure mode on Linux: zbus does not autolaunch a session bus

`zbus-4.4.0/src/address/mod.rs:65-84`: with `DBUS_SESSION_BUS_ADDRESS` unset, zbus resolves
`unix:path=$XDG_RUNTIME_DIR/bus` (falling back to `/run/user/<euid>/bus`) on Linux, and reaches for
`autolaunch:` only on Windows. libdbus's `dbus_bus_get(DBUS_BUS_SESSION)` does attempt X11 autolaunch
in that situation. So on a host with no session bus in the environment - SSH without
`XDG_RUNTIME_DIR`, a systemd system service - the CLI now fails where libdbus might have launched one.
This does not touch the desktop app (a graphical session always exports the variable), but
`relay.rs:433` explicitly contemplates "a headless host is long-lived", so the `gate-connect proxy relay`
and CLI paths are the ones exposed. Verified in zbus's source; the practical impact is **unverified**
(I did not test a bus-less environment, and libdbus autolaunch needs X11 anyway, so the real-world
delta may be nil).

Locked-keyring and dismissed-prompt behaviour is unchanged: both client error enums carry `Locked`
and `Prompt`, and `decode_error` maps both to `NoStorageAccess` either way.

### L-4. `async-io` is redundant, and nothing pins against `tokio`

`zbus-4.4.0/Cargo.toml` has `default = ["async-io"]`, and keyring's zbus dependency does not set
`default-features = false`, so `async-io` was already on. Listing it explicitly is harmless and
defensively right, but the comment's "One is required" overstates what this line does - it changes
nothing about the current build. It also does not *prevent* the failure it was chosen to avoid:
features are additive, `zbus/tokio` would unify in from anywhere, and `utils.rs:37`'s
`#[cfg(feature = "tokio")]` takes priority over the `not(...)` arm. See M-2.

### L-5. Linux builds gain an implicit system libdbus dependency, at build and at run time

With `vendored` gone, `libdbus-sys-0.2.7/build.rs:16-27` takes the `pkg_config ... probe("dbus-1")`
branch and panics if `libdbus-1-dev` and `pkg-config` are absent, where the vendored build needed
neither. CI is fine, transitively: `.github/workflows/{ci,release,e2e-real-tools}.yml` install
`libappindicator3-dev`, which `apt-cache depends` shows requires `libdbus-glib-1-dev`, which requires
`libdbus-1-dev` (verified locally). Nothing to change, but the workflows now depend on that chain
without saying so, and an apt-line cleanup that drops `libappindicator3-dev` would break the Rust
build in a way nothing connects back to here.

At run time the desktop binary now resolves `libdbus-1.so.3` dynamically (see verification 5) where
it previously carried the C statically. `libdbus-1-3` is effectively universal on Linux desktops so
the risk is low, but `src-tauri/tauri.conf.json`'s `bundle.linux.deb.depends` lists only
`libnss3-tools`, and the AppImage now has a shared-object dependency it did not have before. Given
the AppImage libwayland history in this repo, worth an eye on the next Linux release artefact rather
than a code change.

---

## What I could not verify

- **The launch-loop measurement itself** (3 of 5 before, 0 of 9 after). Not reproducible from a
  review; I verified the mechanism instead (M-1), which independently supports it.
- **Which dbus connection actually smashed the stack** - keyring's or tao's. Both used the same
  vendored copy, so both are fixed either way, but the `GATE_CONNECT_TEST_SECRETS` correlation is 5
  samples and is not proof of attribution. See M-1.
- **DH session support in Secret Service implementations other than the one on the test machine**
  (KeePassXC, `pass-secret-service`, KWallet). M-3 is a reasoned risk, not a measured one.
- **The bus-less / headless behaviour delta in L-3.** Verified in zbus's source; not exercised.
- **The `gate-connect` CLI binary's linkage empirically.** `target/debug/gate-connect` has no libdbus
  in `ldd` and zero vendored strings, but its mtime (11:26) predates the commit (11:35), so I did not
  rely on it. The dependency-graph evidence is conclusive on its own: `cargo tree -i libdbus-sys`
  shows the only path runs through `tao -> tauri -> gate-connect-desktop`, never the CLI.
- **A full `cargo build` of `gate-connect-desktop` from a clean target dir.** I ran `cargo check` on
  `gate-connect-core` (`--all-targets`) and `gate-connect-cli`, both exit 0, and the existing
  post-commit desktop binary links and loads, but I did not re-link the desktop crate from scratch.
- **macOS and Windows compilation.** Argued from keyring's `cfg` gating and `cargo tree --target`
  output (verification 3), which is strong, but not compiled on those hosts.
