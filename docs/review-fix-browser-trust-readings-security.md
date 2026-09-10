# Security review: `fix/browser-trust-readings` (PR #243)

Base `origin/feat/new-app-ui` (merge-base `5e2fee30`), commits `28664a17`, `864747cb`, `4e39b03c`.

## Summary

The branch does the right things on the mechanical side of subprocess safety: both new
spawns go through one helper that bounds them with a deadline, kills and then *reaps*
the child, and neither `certutil` nor `gsettings` is ever handed to a shell - every
argument is a separate argv entry, the only variable ones are a `sql:`-prefixed path
under `$HOME`, a compile-time constant nickname, and the app-support cert path, and the
two genuinely shell-shaped commands in the same file (`anchor_install_script`,
`anchor_remove_script`) are untouched and still `sh_quote` everything they interpolate.
The test seam is `#[cfg(test)]` and so is absent from shipped builds. Where the branch is
weaker is the trust *reading* itself, which is the thing it exists to produce: `Trusted`
is inferred from a subprocess exit code plus a PEM body comparison that ignores NSS trust
flags, it is cached for the life of the process rather than probed, and the one NSS state
the module's own doc calls "the one failure here with a security edge" - an entry that
survives an explicit untrust - is the one state the new reporting channel is deliberately
silent about. There are no H findings; five M and four L follow.

## Findings

### M

**M1. A failed untrust records "no reading", so a Gate root left behind in Chromium's
store is the one NSS state the new channel cannot report.**

`untrust_nss` clears the recorded reading unconditionally, *before* it tries to remove
anything:

```
crates/core/src/proxy/ca_linux.rs:653-660
fn untrust_nss() {
    record_nss_trust(None);
    for dir in nss_db_dirs() { ... }
```

Every failure below that point is then invisible to the new channel: a missing `certutil`
takes the `Err(e @ CertutilFailure::Missing)` arm (`ca_linux.rs:667-671`) and a `-D` that
runs and refuses takes `ca_linux.rs:660-666` - both `eprintln!` and carry on, with
`RECORDED_NSS_TRUST` sitting at `None`. `None` is defined on the wire as "no reading, which
is not a negative reading" (`crates/core/src/proxy/mod.rs:1266-1268`), so `ProxyState.ca_nss_trust`
is null, `Diagnostics.ca_nss_write` is null, and `buildDiagnosticsReport` prints nothing
(`src/lib/diagnosticsReport.ts:267`, which is gated on `nssWrite && outcome !== "trusted"`).
Meanwhile `ca_trusted` goes false beside it, so the UI states the CA is removed.

This is the exact case the function's own doc comment names as the security one
(`ca_linux.rs:645-650`: "an entry that survives an explicit untrust leaves a root that can
sign for any host trusted in the browser while the app reports the CA removed"). The branch
built the plumbing to say so and opted this state out of it.

Mitigating, and worth stating: `untrust()` calls `remove_ca_material()` right after
(`ca_linux.rs:672`), which deletes the private key from the secret store and the cert from
disk, so the surviving root is not signable by Gate any more. The residual risk is a
still-trusted MITM root in the browser whose key was in a secret store that was asked to
delete it, not securely wiped. Suggested shape: record a `WriteFailed`-equivalent reading
on the failing branches rather than clearing up front, or clear only after the loop
completes with no failures.

**M2. `browser_proxy_channel()` proves the GNOME schema is readable, not that Gate's
writes landed, and it is cached for the process lifetime.**

```
crates/core/src/proxy/system_proxy_linux.rs:318-323
pub fn browser_proxy_channel() -> bool {
    static CHANNEL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CHANNEL.get_or_init(|| {
        !session_effects_suppressed() && gsettings_get("org.gnome.system.proxy", "mode").is_some()
    })
}
```

A readable `mode` key is not the same fact as "Gate's proxy pointer is in that key".
`gsettings_apply` is best-effort and every one of its six writes goes through
`run_best_effort`, which swallows a non-zero exit into an `eprintln!` and returns nothing
(`system_proxy_linux.rs:359-377` and `:175-186`). On a desktop where dconf is locked by a
system profile - a common managed-Linux configuration - `gsettings set` exits non-zero, the
keys never change, and `browser_proxy_channel()` still answers true because the schema reads
fine. The `OnceLock` then pins that answer for the rest of the process, so nothing later
corrects it.

The copy this drives is an affirmative claim about interception:
`src/lib/platform.ts:118-122` returns "That includes the same site in a browser that follows
your desktop proxy settings", appended to "Gate records and inspects this traffic"
(`src/lib/groups.ts:293-297`, `src/screens/GroupMembers.tsx:109-119`). The Rust field's own
doc says this is precisely the error it exists to prevent: "a present one that is wrong tells
them Gate is inspecting a browser tab it is not touching, which is the one error this field
exists to prevent" (`crates/core/src/proxy/mod.rs:1370-1377`). The probe is one step short of
the claim - reading `mode` back and checking it is `'manual'` with the engine host after
`gsettings_apply` would close it.

Same shape, unconditionally, on the other platforms: `manager_core.rs:180-186` sets
`browser_proxy_channel: true` with no reading behind it at all, so a failed PAC write on
macOS or Windows produces the same false claim.

**M3. `ca_nss_trust` is served from a process-local cache and is never invalidated by
anything outside this process.**

`manager_linux.rs` replaced the live probe with the recorded value:

```
crates/core/src/proxy/manager_linux.rs (was `ca_nss_trusted: ca::nss_ca_trusted()`)
ca_nss_trust: ca::recorded_nss_trust().map(|r| r.outcome),
```

backed by `static RECORDED_NSS_TRUST: Mutex<Option<NssReading>>` (`ca_linux.rs:523`), written
only at `ca_linux.rs:571`, `:587`, `:637` and `:657`. Keeping `certutil` off the polled path
is the right call and the reasoning at `mod.rs:1341-1350` is sound, but the consequence is
that once `Trusted` is recorded it stands until the process exits or an untrust happens.
Anything that removes the CA from the NSS store out of band - a Chromium profile reset, a
Flatpak reinstall that recreates `~/.var/app/.../.pki/nssdb`, another tool running
`certutil -D`, a second Gate - leaves the app reporting `trusted` for a store that no longer
holds the CA. The user then gets the benign "reopen your browser" advice
(`src/lib/groups.ts:252-257`) on a loop that cannot end, which is the same failure mode the
branch's own commentary criticises the previous boolean for.

The live probe still exists and is correct (`ca_linux.rs:493-500`, reached from
`diagnostics.rs:114`), so the fix is not to re-probe on every poll but to give the cache an
invalidation edge, or to fall back to the probe on the paths that already run off a user
action.

**M4. `Trusted` is derived from an exit code and a body-only PEM comparison; nothing reads
the store back, and NSS trust flags are not part of the check.**

`ensure_trusted_nss` starts at `Trusted` and only degrades (`ca_linux.rs:591`), so the value
is recorded whenever every store either skipped or exited zero:

- the skip is `nss_holds` (`ca_linux.rs:606-608`), which is `pem_body(held) == pem_body(pem)`
  (`ca_linux.rs:479-484`). `certutil -L -n <nick> -a` prints the certificate whatever its
  trust bits are, so a database holding our CA with flags `,,` instead of the `C,,` the add
  asks for (`ca_linux.rs:611`) compares equal, is skipped, and is reported `Trusted` while
  Chromium rejects every intercepted host.
- the non-skip path records `Trusted` on `certutil -A` exiting zero (`ca_linux.rs:612-614`,
  `:637`) with no read-back. The read-back function is right there and is used only *before*
  the write.

`nss_holds` and `pem_body` are pre-existing; what is new is that their output is now a
reported trust verdict on `ProxyState` and in the support report rather than an internal
skip optimisation. Comparing the trust flags (`certutil -L -n <nick>` without `-a` prints
them) or re-running `nss_holds` after the add would make the recorded `Trusted` a reading
rather than an inference.

**M5. `output_bounded` does not set stdin, and `gsettings_get` silently lost the
`Stdio::null()` that `Command::output()` was giving it.**

```
crates/core/src/primitives.rs:52-55
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
```

stdin is left at whatever the caller set, which for a bare `Command` is inherit. The doc
above it claims otherwise: "Everything else is the ordinary `std::process::Command::output`
contract" (`primitives.rs:19-20`) - and `output()` sets stdin to `Stdio::null()`. The
conversion at `system_proxy_linux.rs:273-285` replaced `.output()` with `output_bounded` and
did not add `.stdin(Stdio::null())`, so `gsettings` now inherits the parent's stdin
descriptor where it previously got `/dev/null`.

`gsettings` does not read stdin, so there is no exploit today. The reason to fix it is that
this helper was written to be the one shape "so the next caller is not a fourth"
(`primitives.rs:24-26`), and the hazard it will meet is one line away in the sibling caller:
`certutil_output` sets stdin null explicitly and its comment explains that without it a
password-protected NSS database hands the child the user's terminal and blocks there
(`ca_linux.rs:428-431`). Defaulting stdin to null inside `output_bounded` makes the doc true
and removes the trap.

### L

**L1. The support report now embeds absolute NSS store paths and raw, untruncated
`certutil` stderr.**

`src/lib/diagnosticsReport.ts:280-283` prints one `refused` line per store as
`${refusal.store} (${refusal.reason})`. `store` is `dir.display().to_string()`
(`ca_linux.rs:617`), i.e. `/home/<user>/.var/app/com.google.Chrome/.pki/nssdb` - the username
and the user's browser/Flatpak/Snap inventory. `reason` is
`CertutilFailure::Failed`'s text (`ca_linux.rs:618`), which for the exit-status branch is
`"certutil {argv} exited {status}: {stderr}"` (`ca_linux.rs:452-459`) - the argv repeats the
full `-i <app-support>/proxy/ca-cert.pem` path, and the stderr is whatever the binary wrote,
trimmed but not length-capped.

This is a nit rather than a finding because the report already prints
`row("data dir", ...)` (`diagnosticsReport.ts:220`), so the home path and username are
established precedent, and the module docstring's rule is about credentials
(`diagnosticsReport.ts:14-18`), which are not involved here - see the clean list below. Worth
a length cap on `reason` and, if the report is ever machine-uploaded rather than
copy-pasted, a `$HOME` elision pass over the whole builder.

**L2. `certutil` and `gsettings` are resolved through `PATH` while the privileged script in
the same file is careful not to be.**

`certutil_program()` returns `PathBuf::from("certutil")` (`ca_linux.rs:411`), spawned at
`ca_linux.rs:418`; `gsettings` likewise at `system_proxy_linux.rs:274` and `:176`. The same
file probes the trust store by absolute path (`ca_linux.rs:90-101`) and builds its escalated
script out of `/bin/mkdir` and `/usr/bin/install` (`ca_linux.rs:268`). A user-writable
directory early on `PATH` (`~/.local/bin` and `~/bin` are on the default `PATH` on Debian and
Fedora) therefore gets code execution on the CA-trust path, and - via M4 - a shim exiting
zero makes Gate record `Trusted` with nothing written. Same-user privilege, so this is a
hardening nit rather than a boundary crossing, but the surrounding code already holds the
higher standard.

**L3. `output_bounded` has no output size cap, and the captured stderr flows verbatim into
the report.**

`child.wait_with_output()` (`primitives.rs:69`) reads both pipes to EOF with no limit. The
5s deadline bounds the *time*, so the pipe-full deadlock the doc warns about
(`primitives.rs:36-42`) is in fact caught - but a child that writes fast for five seconds is
five seconds of memory, and all of its stderr lands in `NssRefusal.reason`
(`ca_linux.rs:458`, `:618`) and from there in the report. Truncating the stderr at capture
would cover both.

**L4. `nss_db_dirs` follows symlinks, and the db path is lossy on non-UTF-8.**

`.filter(|dir| dir.is_dir())` (`ca_linux.rs:342-351`) follows links, so `~/.pki/nssdb`
symlinked elsewhere silently redirects both the probe and the write; and
`format!("sql:{}", db.display())` (`ca_linux.rs:421`) is `to_string_lossy`, so a home path
with non-UTF-8 bytes is handed to `certutil` with U+FFFD substitutions and addresses a
different directory. Both are inside the user's own home and run unprivileged, so neither
crosses a boundary; the second fails closed (`certutil` errors, `WriteFailed` is recorded).

## Checked and clean

- **No shell, no metacharacter path.** `certutil` and `gsettings` are spawned via
  `Command` with one argv entry per argument (`ca_linux.rs:418-432`,
  `system_proxy_linux.rs:274`). No `sh -c` anywhere on the new paths.
- **No argument injection.** The only non-constant `certutil` arguments are
  `format!("sql:{}", db.display())`, which can never start with `-`, and `cert_arg` from
  `env::app_support_dir()`. The nickname is the compile-time constant
  `"Gate Connect Local CA"` (`crates/core/src/proxy/cert_authority.rs:126`). The two
  shell-string builders in the file are unchanged by the branch and `sh_quote` every
  interpolated path (`ca_linux.rs:262-283`).
- **No new privilege and no new IPC.** `ensure_trusted_nss` and `untrust_nss` run
  unprivileged by construction and say why (`ca_linux.rs:414-416`); the headless
  `ensure_trusted_system` / `untrust_system` never call them (`ca_linux.rs:212-252`). The
  diff adds no `pkexec`/`sudo` call site and no helper-daemon surface.
- **Child cleanup.** `output_bounded` kills *and* `wait()`s (`primitives.rs:61-65`), which
  is the bug it was written to fix, and there is a test that reads `/proc/<pid>/stat` for
  state `Z` on the child's own pid rather than counting process-wide zombies
  (`primitives.rs`, `a_killed_command_is_reaped`).
- **Both spawns are bounded.** 5s for `certutil` (`ca_linux.rs:289`), 1s for `gsettings`
  (`system_proxy_linux.rs:285`). A hung child is reported as a failure, not waited on.
- **No key material in the new fields.** `CertutilFailure::Failed` captures stderr only
  (`ca_linux.rs:452-459`); the certificate body comes back on stdout, and `nss_entry_pem`
  discards the error case entirely (`ca_linux.rs:472-474`). No PEM, no private key, no
  fingerprint reaches `NssRefusal` or the report. The CA private key is never touched on any
  path in this diff.
- **Refusal detail does not cross into the polled snapshot.** `manager_linux.rs` maps to
  `.outcome` only, so store paths and stderr stay in `Diagnostics` and never reach
  `ProxyState` or any UI string.
- **The new probe is not on a timer.** `Diagnostics.ca_nss_trusted` still shells out, but
  `collect()` is reached only from `#[tauri::command(async)] fn diagnostics`
  (`src-tauri/src/lib.rs:1065-1068`), bound to an explicit user action.
- **No XSS.** The report is a React text child of `<pre>` (`src/screens/Diagnostics.tsx:182`);
  `dangerouslySetInnerHTML` appears nowhere under `src/`.
- **The test seam is not shipped.** `CERTUTIL_OVERRIDE` is `#[cfg(test)]`
  (`ca_linux.rs:394-397`), stricter than the env-var seams elsewhere, and the shim tests
  install a temp binary rather than mutating `PATH`.
- **Frontend defaults make no claim on unknown state.** `proxy?.browser_proxy_channel ?? false`
  (`src/App.tsx:1288`, `src/NewUiApp.tsx:2185`) and `proxy?.ca_nss_trust ?? null`
  (`src/NewUiApp.tsx:2166`); an unresolved snapshot suppresses the browser sentence rather
  than asserting it.
- **Delete-then-add fails closed.** A `-D` that lands with a failing `-A` leaves no Gate CA
  in the store and records `WriteFailed` (`ca_linux.rs:609-635`) - the error direction is
  "trust missing", never "trust present and unreported". The SIGKILL at the deadline can land
  mid-write for the same reason, with the same closed direction.
- **`degrade` precedence is order-independent and tested.** A locked store cannot be
  reported as a missing package, so nobody is sent to install `libnss3-tools` they already
  have (`ca_linux.rs:550-556` and its three tests).
- **Wire words are pinned.** `the_nss_wire_words_are_what_the_frontend_expects` and
  `a_refusal_serialises_the_store_and_the_reason` (`crates/core/src/proxy/mod.rs:1976-1999`)
  stop a renamed variant from silently falling through to the wrong advice.
