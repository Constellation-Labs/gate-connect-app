# Security review: `fix/keyring-pure-rust-secret-service`

Lens: **security** (supply chain, crypto, secret handling, error-path exposure).

## Scope

Base `origin/feat/new-app-ui`, single commit `b4636c6` "Stop the app aborting on
launch: drop the C libdbus Secret Service client".

`git diff origin/feat/new-app-ui...HEAD --name-only` is exactly two files:

- `Cargo.toml` (+37/-9): keyring features go from
  `sync-secret-service` + `vendored` to `async-secret-service` + `crypto-rust` + `async-io`
- `Cargo.lock` (381 lines of graph churn)

No Rust or TypeScript source changed. `crates/core/src/keychain.rs`,
`crates/core/src/env.rs` and `crates/core/src/logging.rs` are read here as
context, not as part of the diff. The diff versus `main` is inherited
integration-branch work and is out of scope.

Everything below is checked against the on-disk crate sources in
`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/` and against real
feature resolution (`cargo tree --offline --target x86_64-unknown-linux-gnu`),
not against crate documentation. Where I could not verify something offline I
say so and rate it lower.

## Headline

**This is a security fix, not only a crash fix, and the commit message does not
claim the larger half of it.** On the base branch keyring was built with *no*
crypto feature, which makes its Secret Service backend negotiate the
**`plain`** session algorithm: the Gate API key was travelling the D-Bus session
bus in cleartext. Adding `crypto-rust` is what turns on the DH-negotiated
AES-128-CBC session. See F1.

Nothing in the diff is a must-fix. The two H-adjacent items are both *resolved*
by the commit; the M items are process gaps (no advisory gate, no `--locked`)
and one upstream hygiene regression (zeroize).

## Dependency delta

Lock entries: 672 -> 687 (net +15: 22 added, 7 removed).

### ADDED (22)

| Crate | Version | Enters via | Notes |
|---|---|---|---|
| `secret-service` | 4.0.0 | `keyring/async-secret-service` | pure-Rust SS client; 0 `unsafe` lines |
| `zbus` | 4.4.0 | keyring + secret-service | 38 `unsafe` lines; **older major, see F6** |
| `zbus_macros` | 4.4.0 | zbus 4 | **proc-macro, runs at build time** |
| `zbus_names` | 3.0.0 | zbus 4 | |
| `zvariant` | 4.2.0 | zbus 4 | |
| `zvariant_derive` | 4.2.0 | zvariant 4 | **proc-macro, runs at build time** |
| `zvariant_utils` | 2.1.0 | zvariant 4 | |
| `nix` | 0.29.0 | zbus 4 | **840 `unsafe` lines, has `build.rs`**; libc FFI wrapper |
| `async-fs` | 2.2.0 | zbus 4 (`async-io`) | |
| `xdg-home` | 1.3.0 | zbus 4 | resolves `$HOME` for the bus address |
| `static_assertions` | 1.1.0 | zbus 4 | compile-time only; maintenance status unverifiable offline |
| `aes` | 0.8.4 | secret-service `crypto-rust` | 111 `unsafe` lines (AES-NI intrinsics); `deny(unsafe_code)` on the soft fallback |
| `cbc` | 0.1.2 | secret-service `crypto-rust` | `forbid(unsafe_code)` |
| `cipher` | 0.4.4 | aes/cbc | 2 `unsafe` lines |
| `inout` | 0.1.4 | cipher | 22 `unsafe` lines |
| `block-padding` | 0.3.3 | inout | `forbid(unsafe_code)` |
| `hkdf` | 0.12.4 | secret-service `crypto-rust` | `forbid(unsafe_code)` |
| `hmac` | 0.12.1 | hkdf | 1 `unsafe` line |
| `num` | 0.4.3 | secret-service (**not** optional) | facade over num-bigint/-complex/-iter/-rational |
| `num-complex` | 0.4.6 | num | unused by the SS code path, pulled by the facade |
| `num-iter` | 0.1.46 | num | ditto |
| `num-rational` | 0.4.2 | num | ditto |

Already in the lock and now newly *compiled* on the secret path:
`num-bigint 0.4.6`, `num-integer 0.1.46`, `num-traits 0.2.19`, `sha2 0.10.9`,
`digest 0.10.7`, `generic-array 0.14.7`, `rand 0.8.6` / `rand_core 0.6.4`,
`subtle 2.6.1`, `once_cell`, `futures-util`, `async-io 2.6.0` (62 `unsafe`
lines), `rustix`, `polling`.

Measured: keyring's Linux build subtree (`cargo tree -p keyring -e normal,build
--target x86_64-unknown-linux-gnu`) is now **104 unique crates**. The base's
D-Bus side was `dbus-secret-service` -> `dbus` -> `libdbus-sys` plus `cc` /
`pkg-config` build deps. I did not measure the base subtree exactly, because
doing so needs a checkout of the base and I was told not to modify the tree, so
treat "104 vs a handful" as directional on the D-Bus half and exact on the
current number.

Pre-1.0 among the added set: `aes 0.8.4`, `block-padding 0.3.3`, `cbc 0.1.2`,
`cipher 0.4.4`, `hkdf 0.12.4`, `hmac 0.12.1`, `inout 0.1.4`, `nix 0.29.0`,
`num-* 0.4.x`. For RustCrypto that is normal versioning, and each of these is
the current published line of its crate rather than an abandoned one. All are
`RustCrypto/*` org crates, which is the maintained upstream, not a fork.

Two new proc-macro crates now execute in the build: `zbus_macros 4.4.0` and
`zvariant_derive 4.2.0`. One new `build.rs`: `nix 0.29.0`. Nothing added
compiles C: `cargo tree -e features | grep vendored` returns nothing, and no
`*-sys` crate other than the pre-existing `libdbus-sys` (F7) is in the graph.

### REMOVED (7)

| Crate | Version | Was via |
|---|---|---|
| `openssl` | 0.10.80 | optional dep of keyring / dbus-secret-service |
| `openssl-sys` | 0.9.116 | openssl |
| `openssl-src` | 300.6.0+3.6.2 | openssl-sys (**a vendored OpenSSL source tree**) |
| `openssl-macros` | 0.1.1 | openssl |
| `foreign-types` | 0.3.2 | openssl |
| `foreign-types-shared` | 0.1.1 | foreign-types |
| `vcpkg` | 0.2.15 | openssl-sys |

Note: these were lock entries for *optional* deps that base never activated
(nothing enabled `crypto-openssl`), so removing them shrinks the lock and the
audit surface but does not remove an OpenSSL that was actually being compiled.
`openssl-probe 0.2.1` stays, via a different dependent.

### Duplicate majors now coexisting

`zbus 4.4.0` + `5.13.2`, `zvariant 4.2.0` + `5.9.2`, `zbus_names 3.0.0` +
`4.3.1`, `zbus_macros 4.4.0` + `5.13.2`, `zvariant_derive 4.2.0` + `5.9.2`,
`zvariant_utils 2.1.0` + `3.3.0`. See F6.

`dbus-secret-service 4.1.0` and `libdbus-sys 0.2.7` are still *lock entries* -
Cargo.lock records the union of optional deps regardless of feature selection -
but `cargo tree -p keyring --target x86_64-unknown-linux-gnu` shows neither in
keyring's built graph. `libdbus-sys` is still built, for a different reason: F7.

---

## Findings

### F1 - The base branch sent the API key over D-Bus in cleartext; this commit fixes it. (H, resolved by this commit)

This is the answer to "does the wire encryption still happen at all", and it
comes out the opposite way round from the question.

`keyring 3.6.3` picks the session algorithm from *its own* crypto features, at
two call sites:

- `keyring-3.6.3/src/secret_service.rs:141-144` (`set_secret`)
- `keyring-3.6.3/src/secret_service.rs:333-336` (`map_matching_items`, the read/delete path)

Both read:

```rust
#[cfg(any(feature = "crypto-rust", feature = "crypto-openssl"))]
let session_type = EncryptionType::Dh;
#[cfg(not(any(feature = "crypto-rust", feature = "crypto-openssl")))]
let session_type = EncryptionType::Plain;
```

Base `Cargo.toml` (`git show origin/feat/new-app-ui:Cargo.toml`, lines 33-38)
enabled `apple-native`, `windows-native`, `sync-secret-service`, `vendored` -
and **neither crypto feature**. `keyring`'s manifest defines no `default`
feature contents (`keyring-3.6.3/Cargo.toml:52-88`), and `cargo tree -i keyring`
shows `gate-connect-core` is its only dependent in both revisions, so no
feature unification from elsewhere could have supplied one.

So base resolved to `EncryptionType::Plain`. On that path
`dbus-secret-service-4.1.0/src/session.rs:158-163` builds the `EncryptedSecret`
with the caller's bytes unchanged, and `:180` returns
`Ok(secret.data.clone())` on read. `dbus-secret-service-4.1.0/src/session.rs:38-43`
confirms `Dh` is not even a variant without a crypto feature.

The exposure that closes: a D-Bus session bus permits `BecomeMonitor`
eavesdropping for the session owner, so `dbus-monitor` (and any same-user
process able to run it) could observe the `CreateItem` / `GetSecret` arguments
carrying a raw `sk-gw-…` key. Same-user only, so not a privilege boundary, but
it is exactly the property `CLAUDE.md`'s first design principle sells to the
user: "makes the user feel ... what's being sent over the wire". It also means a
D-Bus trace attached to a bug report could carry a live key.

Head resolves `crypto-rust`, so both call sites take `EncryptionType::Dh`, and
`secret-service-4.0.0/src/session.rs:180-188` (`new_blocking`) opens the session
with `ALGORITHM_DH` and a fresh public key. Verified end to end:
`cargo tree -p keyring -e features -i keyring` lists
`keyring feature "crypto-rust"` as enabled by `gate-connect-core`.

There is no silent fallback to `Plain`. `secret-service` makes the omission a
build failure, not a downgrade: `session.rs:141-144`, `:300-308` route to
`feature_needed!()`, i.e. `compile_error!` (`session.rs:53-58`), when neither
crypto feature is on. And `keyring/src/lib.rs:195-202` rejects
`sync-secret-service` + `async-secret-service` together, so the chosen
combination cannot degrade into a mixed backend either.

**No action.** Recorded because the commit message frames the change as a crash
fix and omits this, and a future reader weighing a revert needs to know that
going back to `sync-secret-service` alone reinstates plaintext transport.

### F2 - The DH exponentiation is not constant time. Pre-existing, unchanged by `crypto-rust`, and not fixable through this crate's features. (M)

`secret-service-4.0.0/src/session.rs:225-239`:

```rust
fn powm(base: &BigUint, exp: &BigUint, modulus: &BigUint) -> BigUint {
    ...
    while !exp.is_zero() {
        if exp.is_odd() {
            result = result.mul(&base).rem(modulus);
        }
        exp = exp.shr(1);
        base = (&base).mul(&base).rem(modulus);
    }
```

Textbook square-and-multiply with a **data-dependent branch on a bit of the
private exponent**, no blinding, no dummy multiply, on `num-bigint 0.4.6` -
which offers no constant-time `modpow` and carries no `subtle`-style
constant-time contract. It is used for both the public key
(`session.rs:80`) and the shared secret (`session.rs:90`), so the private
exponent's Hamming weight and bit pattern are leaked to a timing or
microarchitectural observer of either operation. It is the single most
security-relevant line reachable from this diff, so, concretely:

**`crypto-rust` is not what caused this, and `crypto-openssl` would not have
avoided it.** In `secret-service 4.0.0` only `hkdf` and `encrypt`/`decrypt` are
feature-gated (`session.rs:110-139` and `:241-298`); `powm` at `:225` sits
outside every `cfg` and is the same code under both features. There is no
feature combination of this crate that routes the modexp through OpenSSL's
`BN_mod_exp`.

**The backend it replaces was identical.** `dbus-secret-service-4.1.0/src/session.rs:384-398`
is the same function, same librespot lineage, same `if exp.is_odd()`, also
outside the crypto `cfg`s. So there is no regression here: base either used this
same algorithm, or (per F1) no DH at all.

**Assessment: acceptable.** The attacker has to be co-resident with the process
and able to time or side-channel one modexp per keychain operation - and a
same-user attacker already has the far cheaper option of reading the session bus
or attaching to the process. The value protected is a session key for a hop that
never leaves the machine (an `AF_UNIX` socket to the keyring daemon). The
alternative implementation available in the ecosystem has the same flaw, and the
only real fix is upstream in `secret-service`. Rated M rather than L because it
is worth knowing and worth a link in an upstream issue, not because it should
block this merge.

### F3 - The new backend has no `zeroize`; the one it replaces did. (M)

`dbus-secret-service-4.1.0/src/session.rs:46-53` derives `ZeroizeOnDrop` on
`EncryptedSecret` (wiping `salt`, `data`, `mime`; `#[zeroize(skip)]` on the
session path only), and `:75-81` derives it on `Session`, wiping `shared_key`.
It also gives `Session` a hand-written `Debug` that prints `"(Hidden)"` instead
of key material (`:84-98`).

`secret-service 4.0.0` has **none of that**: `grep -rn 'zeroize\|Zeroize'` over
the whole crate returns nothing, and its manifest declares no `zeroize`
dependency (`secret-service-4.0.0/Cargo.toml`). Specifically unwiped on drop:

- `session.rs:76` `private_key_bytes: [u8; 128]` - the raw DH private exponent
- `session.rs:79` the `BigUint` copy of it, and every heap intermediate in `powm`
- `session.rs:92-94` `common_secret_bytes` / `common_secret_padded` - the DH shared secret
- `session.rs:103` `okm: [u8; 16]` and the `AesKey` in `Session.aes_key` (`:148`)
- the decrypted plaintext `Vec<u8>` from `session.rs:262`

Net effect of the commit on secret hygiene is mixed and worth stating plainly:
the bytes on the wire go from plaintext to AES-128-CBC (F1, a clear gain), while
plaintext key material now lingers in freed heap pages instead of being wiped
(a loss). The gain is larger - an eavesdropper needs only `dbus-monitor`,
whereas reading freed heap needs code execution in the process or a core dump.

Note this is not new to `keychain.rs` either: `crates/core/src/keychain.rs`
holds secrets in plain `String`/`HashMap<String, String>` throughout
(`:96-99`, `:115`, `:211`, `:221-228`), so process memory was never a
zeroized path on any platform. The regression is real but it is one layer of a
wall that was already open.

**Suggested action:** nothing in this diff. Worth an upstream issue on
`secret-service-rs`, and worth a line in the `Cargo.toml` comment so the next
person comparing the two backends does not have to rediscover it.

### F4 - No supply-chain gate exists for a change like this. (M)

Verified absent:

- no `deny.toml`, no `audit.toml`, no `.cargo/audit.toml` at the repo root
- `rg -n 'cargo-audit|cargo audit|cargo-deny|cargo deny|advisor|RUSTSEC|osv|dependabot' .github/ ci/` returns **zero hits** across all five workflows (`ci.yml`, `e2e-real-tools.yml`, `release-notes-slack.yml`, `release-verify.yml`, `release.yml`)
- `cargo audit` and `cargo deny` are not installed (`error: no such command`), and there is no local advisory DB (`~/.cargo/advisory-db` does not exist), so per instructions I installed nothing and **could not screen the 22 added crates against RUSTSEC**

`.github/workflows/ci.yml:47-58` is the whole Rust gate: `rustfmt`, `clippy
--workspace --all-targets -- -D warnings`, `cargo test --workspace`. None of
those has an opinion about a dependency being yanked, unmaintained, or subject
to an advisory.

So a commit that adds 22 crates - including two new build-time proc-macros and
`nix`, with 840 `unsafe` lines - to the module that handles the user's API key
passes CI on formatting and lint alone. For a project whose stated first
principle is "credentials are the product", that is the gap worth closing.

**Suggested action:** add a `cargo-deny` (or `cargo-audit`) job to `ci.yml`.
A minimal `deny.toml` with `[advisories]` and `[bans] multiple-versions = "warn"`
would have surfaced both F4's blind spot and F6 automatically. Rated M, not H:
it is a pre-existing process gap that this commit exposes rather than creates.

### F5 - CI and release never pass `--locked`, so the reviewed lock is not necessarily what ships. (M)

`ci.yml:55` and `:58` run `cargo clippy` / `cargo test` with no `--locked` or
`--frozen`, and `release.yml:67,111` drives `tauri-apps/tauri-action@v0` with
`args: ${{ matrix.args }}` (`--target universal-apple-darwin` or empty), which
likewise carries neither flag. `rg -n 'locked|frozen' .github/workflows/` finds
only the pnpm side: `ci.yml:189` and `release.yml:43` both use
`pnpm install --frozen-lockfile`.

`Cargo.lock` is tracked (`git ls-files` confirms), so the JS half of the build
is reproducible and the Rust half is not. A build can silently resolve newer
semver-compatible versions of `zbus 4.x`, `aes 0.8.x`, `secret-service 4.x` and
the rest than the 381 lines under review here. That makes reviewing a lock
delta partly ceremonial, and it is exactly the wrong asymmetry for the
credential path.

**Suggested action:** add `--locked` to the clippy and test invocations, and set
it for the release build. Cheap, and it makes this review binding.

### F6 - Two D-Bus stacks are now linked, and the one carrying the secret is the older major. (L)

`cargo tree -i` confirms the split:

- `zbus 4.4.0` <- `keyring 3.6.3` and `secret-service 4.0.0` (the secret path)
- `zbus 5.13.2` <- `notify-rust 4.17.0`, `tauri-plugin-opener 2.5.4`, `tauri-plugin-single-instance 2.4.2`

with `zvariant 4.2.0`/`5.9.2`, `zbus_names 3.0.0`/`4.3.1`,
`zbus_macros 4.4.0`/`5.13.2`, `zvariant_derive 4.2.0`/`5.9.2` and
`zvariant_utils 2.1.0`/`3.3.0` duplicated alongside.

Two consequences. First, patch surface: a zbus advisory now has to be checked
against two lines, and the credential path sits on `4.x`, which upstream
maintains behind `5.x`. Second, this is not fixable locally -
`keyring-3.6.3/Cargo.toml:163-165` pins `zbus = "4"` for `cfg(target_os = "linux")`,
so deduplication needs a keyring release that moves to zbus 5.

The pre-existing graph already carries 20 other duplicated majors (`syn 1`/`2`,
`thiserror 1`/`2`, `rand 0.8`/`0.9`/`0.10`, `dirs 4`/`5`/`6`, and so on), so
this is a house style rather than a new sin - but the others are not on the
secret path.

**Suggested action:** none now. Note it, and watch keyring for a zbus-5 release.

### F7 - libdbus C is still linked. The `Cargo.toml` comment's "no C library is compiled or linked" is over-broad. (L)

`Cargo.toml:47-48` (new comment) says `async-secret-service` "is the pure-Rust
client (`secret-service` + `zbus`), so no C library is compiled or linked."

The first half is right and the second is not, though for a reason that does
not undermine the fix. `cargo tree -i libdbus-sys --target x86_64-unknown-linux-gnu`:

```
libdbus-sys v0.2.7
└── dbus v0.9.11
    └── tao v0.35.3
        └── tauri-runtime-wry v2.11.2
            └── tauri v2.11.2
```

So libdbus arrives through Tauri's windowing layer regardless of keyring, and it
is still linked after this commit.

What actually changed - and this is the more precise account of the crash than
the commit message gives - is *vendoring*, and it changed for Tauri too. In
base, `keyring/vendored` expanded to `dbus-secret-service/vendored`, which
`dbus-secret-service-4.1.0/Cargo.toml:57-60` defines as
`["dbus/vendored", "openssl?/vendored"]`. Cargo unifies features per crate
across the graph, so `dbus/vendored` applied to **tao's** `dbus` as well: the
whole process, windowing included, ran a self-compiled libdbus. In head,
`cargo tree -e features | grep vendored` returns nothing at all, and
`cargo tree -e features -i dbus@0.9.11` shows only `dbus feature "default"`
reached from `tao feature "dbus"`. So tao now links the distribution's libdbus,
which is the copy that gets security updates from the distro, and no
`openssl-src` C tree is compiled either (see REMOVED). Both are improvements on
the base, and both are broader than "the secret path no longer uses C".

**Suggested action:** soften the comment to say the *secret path* is pure Rust
and that no C is vendored, since libdbus still arrives via tao. One sentence.

### F8 - `keychain.rs`'s doc comment is now wrong on both halves. (L)

`crates/core/src/keychain.rs:6`:

```
//! - Linux Secret Service (via `sync-secret-service`, vendored libdbus)
```

After this commit it is `async-secret-service`, and nothing is vendored. The
file is not in the diff, so this drifted rather than broke - but it is the
module doc on the credential seam, it is the first thing a reader of
`keychain.rs` sees, and `CLAUDE.md`'s change discipline is explicit that a
rename must account for every hit including string references.

**Suggested action:** update the line in this commit. One line, same subject.

### F9 - The `GATE_CONNECT_TEST_SECRETS` plaintext path is untouched and remains unreachable in a shipped build. (no finding; verified)

Confirmed, since the whole point of the change is that this seam is what hid the
crash:

- `keychain.rs` is not in the diff (`--name-only` is `Cargo.lock`, `Cargo.toml`).
- `crates/core/src/keychain.rs:45-49` reads the seam through `crate::env::test_seam`, and the three file-backed branches (`:82-88` write, `:104-111` read, `:129-136` delete) are all gated on that returning `Some`.
- `crates/core/src/env.rs:19-25` is the gate, and it is a build-kind gate, not an env gate:

```rust
pub(crate) fn test_seam(name: &str) -> Option<std::ffi::OsString> {
    let value = std::env::var_os(name)?;
    if cfg!(debug_assertions) {
        return Some(value);
    }
    eprintln!("[gate] ignoring {name}: test seams are disabled in release builds");
    None
}
```

A release binary therefore ignores `GATE_CONNECT_TEST_SECRETS` and says so on
stderr, so an attacker who can set a shipped process's environment cannot
redirect secret storage to a plaintext directory. Nothing in this diff adds a
feature or `cfg` that could reach it: the diff contains no `[features]` change
and no `cfg` at all, and `rg 'crypto-rust|crypto-openssl' --glob 'Cargo.toml'`
hits only the root manifest's own comment and feature list.

The in-memory test backend (`keychain.rs:65-73`) is likewise install-only via
`use_in_memory_backend()`, `None` unless a test calls it, and untouched.

### F10 - Error strings can now carry D-Bus session detail, but not the secret. (L)

Traced the whole path rather than assuming, because `CLAUDE.md` documents that
raw backend errors reach the user under a Details disclosure.

The secret does **not** reach an error string:

- `keychain.rs` context strings carry only `{service}/{account}` (`:97`, `:99`, `:119`, `:123`, `:144`, `:148`, `:224`) - never `value`.
- `secret-service-4.0.0/src/error.rs:35-48` forwards zbus's `Display`.
- `zbus-4.4.0/src/error.rs:139-144` is the one variant that could leak a message body, and it deliberately does not:

```rust
Error::MethodError(name, detail, _reply) => write!(
    f, "{}: {}", **name,
    detail.as_ref().map(|s| s.as_str()).unwrap_or("no details")),
```

The `Message` - which on a `CreateItem` failure holds the secret argument - is
bound to `_reply` and dropped. zbus's `Error` *derives* `Debug`
(`zbus-4.4.0/src/error.rs:14`), which would print it, but the app never formats
an error with `Debug`: every Tauri boundary uses `format!("{e:#}")`
(`src-tauri/src/lib.rs:112`, `:195`, `:218`, `:228`, `:241`, `:300`, `:303`,
`:306-309`, `:329`, `:337`, and further), which is `anyhow`'s alternate
`Display` chain, not `Debug`. `rg '\{:\?\}|\{:#\?\}'` over `crates/core/src`
and `src-tauri/src` finds no keychain or keyring error among the hits.

What *can* newly surface is bus plumbing detail: `Error::Address(String)`
(`zbus-4.4.0/src/error.rs:21`, printed at `:128`), `Error::Handshake(String)`
(`:35`, printed at `:130`) and `Error::InputOutput` (`:23`, printed at `:130`)
can put the D-Bus address or socket path into a user-visible string. That string
is rendered verbatim: `src/components/gc/banners.tsx:382-402` `ErrorDetails`
prints `{raw}` with no redaction and offers a copy button (`:403-407`), reached
from `src/components/gc/dialogs.tsx:705`. The `redact()` backstop
(`crates/core/src/logging.rs:105-110`) guards only the log file and only matches
`sk-gw-` / `sk-ant-` prefixes, so it would not help here and does not need to.

A `$XDG_RUNTIME_DIR/bus` path is not a secret, and the old backend's
`dbus::Error` was no quieter. Rated L on that basis; recorded because "raw
backend error into a copyable disclosure" is a pattern that only stays safe as
long as someone keeps checking what the backend puts in it.

### F11 - The RNG behind the DH private key improves. (L, positive)

Worth recording because it makes the *choice between the two fixes* clear, not
just the fix itself.

- `secret-service-4.0.0/src/session.rs:32` imports `rand::rngs::OsRng`, and `:75-77` fills the 128-byte private exponent from it. `OsRng` is `getrandom`-backed OS entropy: correct for key generation.
- `dbus-secret-service-4.1.0/src/session.rs:194` imports `fastrand::Rng`, and `:316-318` fills the same private exponent from it. `fastrand` is a small non-cryptographic Wyrand PRNG (declared at `dbus-secret-service-4.1.0/Cargo.toml:92-94`), and it also generates the AES IV via `salt()`.

So the other available fix - keeping `sync-secret-service` and adding
`crypto-rust` to it - would have produced DH private keys and CBC IVs from a
non-cryptographic PRNG. The commit picked the better of the two. That reasoning
is not in the `Cargo.toml` comment and deserves a line there, because it is the
argument against the reviewer's obvious "why not just add `crypto-rust` and keep
the sync client?".

### F12 - 1024-bit DH group, mandated by the protocol. (L, informational)

`secret-service-4.0.0/src/session.rs:38-51` hardcodes generator 2 and the
1024-bit MODP prime (RFC 2409 Second Oakley Group), with a 1024-bit private
exponent (`:76`) and a 128-bit AES key from HKDF-SHA256 (`:103-104`, `:131-139`).
1024-bit finite-field DH is below current guidance, but
`dh-ietf1024-sha256-aes128-cbc-pkcs7` is what the freedesktop Secret Service
specification defines, so both clients use it and this repo has no say. The
peer is the local keyring daemon over a unix socket, so the practical exposure
is small.

No action.

---

## Summary table

| ID | Severity | Finding |
|---|---|---|
| F1 | H, **fixed by this commit** | Base sent the API key over D-Bus in cleartext (`Plain` session); `crypto-rust` turns on DH+AES |
| F2 | M | DH modexp is not constant time; pre-existing, identical in both backends, unreachable by any feature choice |
| F3 | M | `secret-service 4.0.0` has no `zeroize`; the replaced backend wiped its secret buffers |
| F4 | M | No `cargo-audit` / `cargo-deny` / `deny.toml` / advisory CI job; 22 new crates unscreened |
| F5 | M | CI and release never pass `--locked`, so the reviewed lock is not binding |
| F6 | L | zbus 4 + zbus 5 both linked; the secret path is on the older major; not fixable until keyring moves |
| F7 | L | libdbus C still linked via tao; the "no C linked" comment is over-broad (vendoring did stop, for tao too) |
| F8 | L | `crates/core/src/keychain.rs:6` doc comment now wrong on both halves |
| F9 | - | `GATE_CONNECT_TEST_SECRETS` untouched; `env.rs:19-25` gates it on `debug_assertions`; verified release-safe |
| F10 | L | Bus address / socket path can reach `ErrorDetails` unredacted; the secret cannot (zbus drops the `Message`) |
| F11 | L, positive | New backend uses `OsRng`; the alternative fix would have used non-cryptographic `fastrand` |
| F12 | L, info | 1024-bit DH group, mandated by the Secret Service spec |

## Verdict

**Approve on the security lens.** The change is a net improvement on the
property that matters most here - it takes the Gate API key off the session bus
in cleartext - and it does so via the better of the two available backends
(F11). Nothing in the diff is a must-fix.

Recommended before merge, all small: fix the stale `keychain.rs:6` comment (F8),
soften the "no C linked" claim (F7), and add the F1 and F11 reasoning to the
`Cargo.toml` comment so a future revert is not argued from the crash alone.

Recommended as follow-ups, independent of this commit: a `cargo-deny` job (F4)
and `--locked` in CI and release (F5). Those two are the actual gap this change
revealed - not the dependency swap, but that a 22-crate expansion of the
credential path could be merged with no advisory screening and no guarantee
that the reviewed lock is the one that ships.

## What I could not verify

- **RUSTSEC advisory status of the 22 added crates.** `cargo audit` / `cargo deny` are not installed and there is no local advisory DB; per instructions I installed nothing. F4 is the finding; the individual crates' clean-or-not status is genuinely unknown from this review.
- **Maintenance status of `static_assertions 1.1.0` and `xdg-home 1.3.0`.** No network, and the registry cache carries no download or release-date metadata. Neither is on the secret path (`static_assertions` is compile-time only), so rated as unverifiable rather than as a finding.
- **Exact crate count of the base's keyring subtree.** Measuring it needs a checkout of the base, and the brief forbade modifying the tree. The head number (104) is measured; the base comparison in the dependency section is directional.
- **That the tree builds.** I ran no `cargo check` or `cargo build`. Feature resolution is verified (`cargo tree` resolves cleanly, `zbus 4.4.0` gets `async-io` and not `tokio`, and neither of keyring's two `compile_error!` guards at `lib.rs:195-202` nor `secret-service`'s `feature_needed!` is triggered), but compilation and the runtime keychain round-trip belong to the correctness lens.
