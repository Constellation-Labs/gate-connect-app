# Review: `fix/keyring-pure-rust-secret-service` (code quality)

**Base**: `origin/feat/new-app-ui` · **Commit**: b4636c6 "Stop the app aborting
on launch: drop the C libdbus Secret Service client"
**Scope**: `Cargo.toml` (37 lines) and `Cargo.lock` (381 lines). Nothing else is
touched, so this review is about the dependency configuration, the comment block
that justifies it, and what the rest of the tree still says about the thing that
was removed.

**Verification basis**: the resolved graph (`cargo tree --offline`), both
versions of `Cargo.lock`, the vendored sources of `keyring-3.6.3`,
`secret-service-4.0.0`, `zbus-4.4.0` and `libdbus-sys-0.2.7` under
`~/.cargo/registry/src/`, the built binary in `target/debug/`, and `apt-cache`
on this machine. Where I could not verify something (the GitHub runner image,
the AppImage AppDir contents) I say so.

The change itself is correct and the comment is unusually well checked: five of
its six factual claims hold exactly as written. Findings below are one stale
doc comment elsewhere in the tree, three consequences of un-vendoring that the
manifest does not mention, a coverage gap, and nits.

---

## H1. `keychain.rs` still documents the crashing backend

`crates/core/src/keychain.rs:6` reads:

    //! - Linux Secret Service (via `sync-secret-service`, vendored libdbus)

That is exactly the configuration this commit removes, and it is now the only
place in the repo that still names it (`rg 'sync-secret-service|dbus-secret-service|vendored'`
finds nothing else outside `Cargo.toml`, `Cargo.lock` and unrelated hits). The
irony is direct: the new manifest comment cites this file by name
(`Cargo.toml:51-52`, "`keychain.rs` needs no change"), and it is true of the
code and false of the doc comment three lines above the `use keyring::Entry`
(`crates/core/src/keychain.rs:17`).

One-line fix: `via `async-secret-service`, pure-Rust zbus client`. This is the
one thing in the branch I would block on, because a future reader debugging the
Linux secret path will read that module header before the workspace manifest.

## M1. "no C library is compiled or linked" is contradicted by the binary

`Cargo.toml:46-47` says `async-secret-service` is the pure-Rust client "so no C
library is compiled or linked". True of keyring's backend; false of the shipped
binary. Verified on the post-change build:

- `objdump -p target/debug/gate-connect-desktop | grep NEEDED` lists
  `libdbus-1.so.3` as a **direct** NEEDED entry.
- `libgtk-3.so.0` does not need libdbus, so it is not coming in through GTK: it
  is `tao` -> `dbus` -> `libdbus-sys`, confirmed by
  `cargo tree -i libdbus-sys` (`tao 0.35.3` -> `tauri-runtime-wry` -> `tauri`).

The commit message gets this right ("tao's libdbus-sys now links the system
libdbus instead of compiling its own"), but the manifest comment is the durable
artifact and it is the one that overstates. The accurate sentence is "no C
library is compiled or linked *for the secret store*; the app still links the
distribution's libdbus through tao", which is also the sentence that explains
why the fix works: the vendored copy is gone, the patched system one is used.

Related precision point, same paragraph: `Cargo.toml:57-59` explains `vendored`
as merely "inert". It is inert given the client swap (with `dbus-secret-service`
disabled, `dbus-secret-service?/vendored` cannot activate), so the claim is
literally correct. But the two changes together are what un-vendored *tao's*
libdbus, and that is the consequence with platform-visible effects (M2, M3).
The manifest says nothing about it.

## M2. Linux builds now require `libdbus-1-dev`, and no CI file says so

`libdbus-sys-0.2.7/build.rs` branches on its own `vendored` feature: without it
(lines 18-26) it runs `pkg_config::Config::new().atleast_version("1.6").probe("dbus-1")`
and `panic!()`s with "check whether packages 'libdbus-1-dev' and 'pkg-config'
are installed". With it, it compiles the C instead and needs neither.

The lock records the flip precisely:

- base `Cargo.lock:2633-2636`: `libdbus-sys` deps `["cc", "pkg-config"]` (vendored on)
- HEAD `Cargo.lock:2705-2707`: `["pkg-config"]` (vendored off)

So a Linux build that previously needed no dbus headers now hard-fails without
them. None of the three workflows names the package:
`.github/workflows/ci.yml:33`, `release.yml:29`, `e2e-real-tools.yml:39` all
install `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`.

It is satisfied transitively today. Verified with `apt-cache depends` on this
machine: `libwebkit2gtk-4.1-dev` -> `libgtk-3-dev` -> `libatk-bridge2.0-dev` ->
`libatspi2.0-dev` -> `libdbus-1-dev`. I could **not** verify this on the
`ubuntu-22.04` runner image, and the chain is four hops of somebody else's
packaging holding up our build. Adding `libdbus-1-dev` to the three apt lines
costs nothing and makes the requirement declared rather than accidental.

## M3. Shipped Linux artifacts gained a host shared-library dependency

Following from M1: the binary now resolves `libdbus-1.so.3` at runtime instead
of carrying the C statically. Two places that describe what we ship do not
account for it:

- `src-tauri/tauri.conf.json:68-72` declares deb `depends: ["libnss3-tools"]`
  only. Tauri's deb bundler does not run `dpkg-shlibdeps`, so `libdbus-1-3`
  will not be declared. In practice it is present on every desktop that has a
  Secret Service at all, so this is a correctness-of-metadata point, not a
  breakage.
- The AppImage is the one I would actually check. I could not verify whether the
  AppDir now bundles `libdbus-1.so.3`, and if it does, a libdbus built against
  ubuntu-22.04 talking to a newer host's session bus is the same failure shape
  as the bundled-libwayland bug that `release.yml`'s "Fix bundled libwayland in
  AppImage" step exists to strip. Concrete check on the next release artifact:
  `./*.AppImage --appimage-extract && ls squashfs-root/usr/lib | grep dbus`. If
  it is there, it belongs in the same strip step.

## M4. Nothing in CI executes the backend that changed

The commit swaps the implementation of every credential read and write, and the
test suite cannot see it:

- every test uses one of the two seams: `keychain::use_in_memory_backend()`
  (`crates/core/tests/audit_e2e.rs:368`, `billing_mode.rs:60`,
  `security_feed_e2e.rs:75`, and ~20 more) or the file seam
  (`crates/core/tests/keychain_chunking.rs:7`, `reconcile_enabled.rs:8`,
  `provider.rs:1235`)
- the e2e leg exports it too: `ci/e2e/run.sh:104`
- `crates/core/src/keychain.rs:243-300` unit-tests only chunk splitting and
  manifest parsing, all above the backend

So `cargo test --workspace` on `ubuntu-22.04` (`ci.yml:57`) *compiles* keyring's
new linux module and never runs one Secret Service round-trip. The comment
itself names the mechanism at `Cargo.toml:39-41`: the seam "hid it entirely",
which is why `pnpm app:local` never showed the crash. That is a precise
statement that the seam which makes local dev safe also makes this whole code
path invisible to automation, and the only evidence the swap preserves
compatibility is the manual verification in the commit message (15 round-trips,
plus reading back a credential the libdbus client wrote).

Naming the gap precisely: there is no test, ignored or otherwise, that
constructs a real `keyring::Entry`. The cheap fix is an `#[ignore]`d
integration test doing set/get/delete against the real store, runnable under
`dbus-run-session` with `gnome-keyring-daemon`, so the next backend change has
something to run instead of a paragraph to trust.

## L1. "eight lines below" no longer measures anything

`Cargo.toml:43` says `hudsucker` is "eight lines below". From that line,
`hudsucker = { ... }` is at `Cargo.toml:77`, i.e. 34 lines below; the only
reading that yields eight is `keyring =` (line 61) to the start of hudsucker's
comment (line 69). In the base file the same distance was 15 lines
(`keyring` at 33, `hudsucker` at 48), so the number was not carried over
correctly and will rot again on the next edit. Name the crate and drop the
distance.

## L2. "neither of those crates is in the tree" is true of the graph, not of the lock

`Cargo.toml:57-59`. Verified correct as stated: `cargo tree -e normal -i
dbus-secret-service` returns "nothing to print", and `openssl` is gone from
`Cargo.lock` outright (with `openssl-sys`, `openssl-src`, `openssl-macros`,
`vcpkg`, `foreign-types`, `foreign-types-shared`). But `Cargo.lock` still
carries `dbus-secret-service` (line 995), `dbus` (984) and `libdbus-sys` (2701),
because a lockfile records unactivated optional dependencies. A reader who
checks the claim the obvious way (grep the lock) will conclude the comment is
wrong. One clause fixes it: "in the build graph; the lock still lists them as
unactivated optional deps".

## L3. 28 lines of incident narrative in a workspace manifest

`Cargo.toml:27-59` is 33 comment lines above a 7-line dependency entry, and
roughly two thirds of it is the crash story: the gdb frames, the 3-of-5 launch
count, the Ubuntu version, why `app:local` masked it. All of that is already in
the commit message, in more detail and in the place designed to hold it.

What a manifest reader needs is the three decisions, which are also the three
things a future edit could get wrong: async client not sync, `async-io` not
`tokio`, `vendored` no longer meaningful. That is about eight lines. The
reusable lesson - "the dev script's secret seam hides every real keychain
bug, so `pnpm app` is the only way to see them" - is textbook material for
CLAUDE.md's "Implementation notes that bite" section, where a future agent will
actually encounter it, rather than in a TOML comment nobody opens unless they
are already editing dependencies. This is a proportionality nit, not a defect;
the repo's comment culture is deliberately heavy and this is within it.

## L4. Markdown emphasis in a TOML comment

`Cargo.toml` now has four `**bold**` runs (lines 33, 50, 53, 57). Zero exist in
`crates/core/Cargo.toml`, `crates/cli/Cargo.toml`, `src-tauri/Cargo.toml`, or in
the base version of this file. Consistent with CLAUDE.md's prose style,
inconsistent with every other manifest in the repo.

## L5. Two zbus majors now compile into one binary

Unmentioned side effect worth a clause given the comment's length: keyring 3.6.3
pins `zbus 4.4.0` (`Cargo.lock:2634`) while `tauri-plugin-notification`,
`tauri-plugin-opener` and `tauri-plugin-single-instance` pull `zbus 5.13.2`
(verified with `cargo tree -i zbus@5.13.2`). The binary now links two complete
D-Bus client stacks. Net crate count 671 -> 686. Not a defect, and not
avoidable without changing keyring, but it is the kind of thing the next person
comparing build times will want explained.

## L6. Commit message arithmetic

"ten launches against the real Secret Service" is followed by "before 3 of 5"
and "after 0 of 9", which is fourteen, plus a further "0 of 5" with the seam
set. The `Cargo.toml` comment quotes only the 3-of-5 half and is
self-consistent. Cosmetic, and the measurement is convincing either way.

---

## Claims that check out

Recorded because the review brief asked for each to be verified individually,
and because a future reader should not have to redo this.

1. **keyring 3.6.3** - `Cargo.lock:2622-2624`. The version the comment claims to
   have checked against is the version resolved.
2. **The backend wraps `secret_service::blocking::*`** - correct, verbatim:
   `keyring-3.6.3/src/secret_service.rs:94-96` imports
   `secret_service::blocking::{Collection, Item, SecretService}` under
   `#[cfg(feature = "async-secret-service")]`, and the same pattern repeats at
   lines 850-851 and 862-863. `Entry` therefore stays sync, and
   `crates/core/src/keychain.rs:81-152` (`set_raw` / `get_raw` / `delete_raw`)
   needs no change. Correct.
3. **A runtime feature is genuinely required** - `secret-service-4.0.0/src/session.rs:53-57`
   defines a `feature_needed!` macro expanding to `compile_error!("Please enable
   a feature to pick a runtime ...")`. keyring exposes the choice as
   `async-io = ["zbus?/async-io"]` and `tokio = ["zbus?/tokio"]`
   (`keyring-3.6.3/Cargo.toml:58,81`). "One is required" is right.
4. **The tokio hazard is real** - `zbus-4.4.0/src/utils.rs:37-50`: under the
   `tokio` feature, `block_on` builds its own current-thread `Runtime` in a
   `OnceLock` and calls `runtime.block_on(future)`, which panics when reached
   from a thread already inside a runtime. The `async-io` variant
   (`utils.rs:33-35`) is a plain `async_io::block_on`. The repo has already been
   bitten by this exact class of panic and documents it at
   `src-tauri/src/lib.rs:2320-2328`, which is strong corroboration for the
   choice.
   Nuance worth recording: no *current* call path reaches the keychain from an
   entered runtime. The three `#[tauri::command(async)]` handlers
   (`src-tauri/src/lib.rs:1065` diagnostics, `1956`, `2009`) do not touch
   secrets, every secret-touching command hands off to `spawn_blocking`
   (`lib.rs:191`, `380`, `545`, `577`, `587`), where a nested `block_on` is
   legal, and `crates/core/src/proxy/relay.rs:359` reads the account *before*
   `rt.block_on`. So `async-io` is a correctly-chosen guard against a hazard the
   tree does not currently trigger, and the comment's conditional "would" is
   the right tense. No finding.
5. **`vendored` expands as quoted** - `keyring-3.6.3/Cargo.toml:82-85`:
   `vendored = ["dbus-secret-service?/vendored", "openssl?/vendored"]`. Exact.
   And `crypto-rust` is what removes openssl: `secret-service-4.0.0`'s
   `crypto-openssl` is `["dep:openssl"]` while `crypto-rust` is
   `["dep:aes", "dep:cbc", "dep:sha2", "dep:hkdf"]`.
6. **Cross-target safety** - the new feature names are unconditional in
   keyring's `[features]` table while the crates they gate are target-gated
   (`keyring-3.6.3/Cargo.toml:147-166`), so enabling `async-secret-service`,
   `crypto-rust` and `async-io` from the shared `[workspace.dependencies]` entry
   is inert on macOS and Windows. No per-target split needed.
7. **No em dash in the new comment** - a grep for the character in `Cargo.toml` returns only lines
   22 and 23, which are pre-existing and untouched by this commit. The new block
   uses hyphens throughout (`Cargo.toml:53`, `56`). Clean.
   (Aside, out of scope: those two pre-existing violations at `Cargo.toml:22-23`
   are still there and would be worth a sweep.)
8. **No other stale references** - the full sweep for
   `sync-secret-service|dbus|libdbus|vendored|secret-service` leaves nothing
   else needing an edit. `README.md:20` says "Linux Secret Service", which is
   backend-agnostic and stays true. `crates/core/src/proxy/system_proxy_linux.rs:23,191,204,217,223`
   is about the `dbus-update-activation-environment` CLI, unrelated. The
   `vendored` hits in `src/components/gc/BrandMark.tsx:7`, `docs/review-figma-sidenav.md`
   and `docs/cognito/*` are about vendored SVGs and logos. Only
   `crates/core/src/keychain.rs:6` (H1) is wrong.
