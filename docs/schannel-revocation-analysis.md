# schannel `CRYPT_E_NO_REVOCATION_CHECK` against the Gate MITM CA (Windows)

Analysis only - no code changed. Every claim below is tagged **[tested]**,
**[read from source]**, or **[inferred]**.

## 1. Confirmed root cause

### What we actually emit

Dumped from our own code, not from recollection: a temporary `#[test]` inside
`crates/core/src/proxy/cert_authority.rs` called `ca_certificate_params()` and
`GateCa::gen_cert("claude.ai")`, wrote both to disk, then was reverted
(`git checkout`, tree clean). **[tested]**

Leaf for `claude.ai`:

```
X509v3 Authority Key Identifier:  01:14:55:...:09:7E
X509v3 Subject Alternative Name:  DNS:claude.ai
X509v3 Key Usage: critical        Digital Signature, Key Encipherment
X509v3 Extended Key Usage:        TLS Web Server Authentication
X509v3 Subject Key Identifier:    27:F5:D4:...:2F:26
X509v3 Basic Constraints: critical  CA:FALSE
```

Root:

```
X509v3 Key Usage: critical        Digital Signature, Certificate Sign, CRL Sign
X509v3 Name Constraints: critical Permitted: DNS:api.anthropic.com, claude.ai,
                                  api.openai.com, chatgpt.com, chatgpt.com,
                                  openrouter.ai, opencode.ai
X509v3 Subject Key Identifier:    01:14:55:...:09:7E
X509v3 Basic Constraints: critical  CA:TRUE
Validity: Not Before Jan 1 1975 / Not After Jan 1 4096
```

So: **no CRL Distribution Point and no Authority Information Access on either
the leaf or the root.** `basicConstraints`, `keyUsage` and `EKU` are all present
and correct. The hypothesis in the brief is confirmed on the certificate side.

### What schannel does with that

The brief says curl's schannel backend passes
`CERT_CHAIN_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT` by default. **That is not what
current curl does.** From `lib/vtls/schannel.c` at `master`, `curl-8_14_1`,
`curl-8_9_1`, `curl-8_4_0` and `curl-8_0_1` - all five identical on this point
**[read from source]**:

```c
if(conn_config->verifypeer) {
  ...
  if(ssl_config->no_revoke) {              /* --ssl-no-revoke */
    flags |= SCH_CRED_IGNORE_NO_REVOCATION_CHECK |
             SCH_CRED_IGNORE_REVOCATION_OFFLINE;
  }
  else if(ssl_config->revoke_best_effort) { /* --ssl-revoke-best-effort */
    flags |= SCH_CRED_IGNORE_NO_REVOCATION_CHECK |
             SCH_CRED_IGNORE_REVOCATION_OFFLINE |
             SCH_CRED_REVOCATION_CHECK_CHAIN;
  }
  else {                                    /* DEFAULT */
    flags |= SCH_CRED_REVOCATION_CHECK_CHAIN;
  }
}
```

Two corrections follow, and the second one changes the fix design:

1. The default is `SCH_CRED_REVOCATION_CHECK_CHAIN`, which per the `SCHANNEL_CRED`
   docs covers **every certificate in the chain including the root** - not the
   `..._EXCLUDE_ROOT` variant. curl never sets `..._EXCLUDE_ROOT` in any 8.x.
2. In the default branch neither `SCH_CRED_IGNORE_NO_REVOCATION_CHECK` nor
   `SCH_CRED_IGNORE_REVOCATION_OFFLINE` is set, so *any* inability to determine
   revocation status is fatal.

### The error code is itself the evidence

Windows distinguishes two failures, and the one we got is specific:

| Code | Name | Meaning |
|---|---|---|
| `0x80092012` | `CRYPT_E_NO_REVOCATION_CHECK` | No revocation *information* exists - the cert carries no CDP and no OCSP AIA |
| `0x80092013` | `CRYPT_E_REVOCATION_OFFLINE` | A CDP/AIA exists but the responder could not be reached |

The report is `0x80092012`. That rules out "the CRL fetch was blocked by the
proxy" and pins it on the certificate having no revocation pointer at all -
which is exactly what the dump above shows. **[inferred, but tightly]**

### Linux-side analogue

OpenSSL reproduces the same class of failure once you ask it to hard-check,
which is a useful local proxy for schannel's behaviour **[tested]**:

```
$ openssl verify -crl_check -CAfile root.pem leaf-today.pem
CN = claude.ai
error 3 at 0 depth lookup: unable to get certificate CRL   # exit 2
```

### Was `claude.ai` even supposed to be intercepted?

`crates/core/src/proxy/mod.rs:748` ships the `claude-web` domain (the only entry
carrying `claude.ai`) with `enabled: false`, and `engine.rs:394` `should_intercept`
consults `should_intercept_host`, which requires `d.enabled` - everything else is
blind-tunnelled. So `claude.ai/install.cmd` is only MITM'd on a machine where
someone ran `gate-connect proxy domain claude-web on`. **[read from source]**

Worth confirming with the reporter, because it changes the urgency: on a default
install the failing command is `api.anthropic.com`, not `claude.ai`. The
mechanism is identical either way - it hits any intercepted host.

## 2. Which trust store

`ca_windows.rs` targets:

- **Default / GUI path**: `certutil -user -addstore Root` - the per-user store
  (`HKCU\...\Root`), no admin, but Windows shows its native trust-confirmation
  dialog (`ca_windows.rs:7-12`).
- **`--system-trust` (CLI only, opt-in)**: no `-user`, i.e. `LocalMachine\Root`,
  requires an already-elevated process (`ca_windows.rs:361-379`).
- `is_trusted` consults **both** stores.

The Enterprise and Group Policy root stores are never touched, and shouldn't be.

Does the store choice change the revocation outcome? **No.** Revocation checking
is driven by the chain engine's flags and by what the certificates carry, not by
which physical store the anchor came from. A per-user root and a machine root
both produce `CRYPT_E_NO_REVOCATION_CHECK` for a leaf with no CDP. **[inferred]**

## 3. Fix options, ranked

| # | Option | Works for all schannel clients | User action | Security cost | Size |
|---|---|---|---|---|---|
| **A** | **CDP on the leaf + serve a signed CRL from the existing loopback PAC listener** | **Yes** (if loopback CDP fetch works - see open questions) | **None** | Low - an empty CRL we control; revocation was already unenforceable | ~80 lines, 3 files |
| B | OCSP AIA + a loopback OCSP responder | Yes | None | Low | Large - rcgen has no AIA support and no OCSP responder; hand-rolled DER + RFC 6960 responder |
| C | Stop intercepting the affected hosts | Only for excluded hosts | None | **Loses the feature** on those hosts | Small |
| D | Do nothing; document `--ssl-revoke-best-effort` | No - only curl, only when the user edits the command | High, per-invocation | None | Docs only |
| E | Machine-wide registry knobs relaxing revocation | Yes | High + elevation | **High - weakens revocation for every TLS client and every user on the box** | Small |

### Why the runners-up lose

**B (OCSP)** is strictly more work for the same result. rcgen 0.14.7 has
`crl_distribution_points` and a full `CertificateRevocationListParams::signed_by`
**[read from source]**, but nothing for AIA - you would hand-roll the
`1.3.6.1.5.5.7.1.1` extension as a `CustomExtension` *and* implement an OCSP
responder. CryptoAPI accepts CRL-only certs happily. The only argument for OCSP
is freshness, which is meaningless for a CA whose entire population of leaves is
minted in-process and never revoked.

**C (exclusion list)** cannot be the general answer, because the hosts that
break are the hosts we exist to intercept. `api.anthropic.com` and `chatgpt.com`
are the product. It *is* however the right answer for `claude.ai` specifically
if - and only if - the goal there is inference audit rather than covering the
whole host: `claude.ai/install.cmd` is a plain script download that Gate has no
reason to see, while `rewrite_prefixes: ["/organizations/"]` is the part that
matters. But TLS interception is decided at CONNECT time on the host alone
(`engine.rs:394-436`), so there is no way to MITM `/organizations/` and tunnel
`/install.cmd` on the same host. Excluding `claude.ai` means giving up the
`claude-web` surface entirely. Note also that `a-api.anthropic.com` (telemetry)
is already deliberately left out of the catalog (`mod.rs:662-664`).

**D (document the workaround)** leaves a broken default. The blast radius is not
"one curl command": it is every schannel client on the machine, for every
intercepted host, for as long as the proxy is on. `--ssl-revoke-best-effort`
also cannot be applied to callers you do not control - a script that pipes
`install.cmd`, an installer, a Store app.

**E (registry knobs)** turns a Gate-scoped problem into a machine-scoped
weakening. Disabling or soft-failing revocation checks machine-wide means every
TLS client for every user on that box stops noticing genuinely revoked
certificates from *public* CAs. Gate should not ship that, and should not
document it as a recommendation. Last resort, and only ever as something the
user chooses knowingly.

## 4. Recommended fix: A

Add a CRL distribution point to the leaf, pointing at a `/gate-ca.crl` path on
the loopback HTTP listener the engine **already runs on Windows**, and serve a
signed, empty, non-expired CRL from it.

Why this is the right shape here:

- The infrastructure exists. `engine.rs:727` `serve_pac` is already a plain-HTTP
  loopback responder, bound at `engine.rs:788` on a port persisted via
  `port_persist` and drawn from the stable `47100..47200` band. It is spawned
  exactly when the proxy is on, and it dies with the engine - the same lifetime
  the CRL needs. It is a *different listener* from the MITM proxy port, which
  matters: a revocation fetch that happened during a handshake on the proxy port
  would be re-entrant.
- The leaves are minted in memory and cached per host in a `HashMap` that never
  outlives the process (`cert_authority.rs`), so baking a port into the CDP URL
  cannot go stale - a port change means a new engine means new leaves.
- **It requires no CA regeneration and no re-trust.** This is the decisive
  practical advantage, see the root-CDP caveat below.

### Verified properties of the prototype

I built the exact leaf + CRL with rcgen 0.14.7 in a scratch crate **[tested]**:

```
X509v3 CRL Distribution Points:
    Full Name:
      URI:http://127.0.0.1:51713/gate-ca.crl
```

- `openssl crl -text` parses it: v2, ECDSA-SHA256, correct issuer, CRL Number 1,
  `nextUpdate` +7d, no revoked certs. **[tested]**
- `openssl verify -crl_check -CAfile root.pem -CRLfile ca.crl leaf.pem` → **OK**,
  where today's leaf fails the same check. **[tested]**
- `openssl verify` without `-crl_check` → still OK (no regression). **[tested]**
- **rustls/webpki accepts the CDP leaf** for `claude.ai` against our root
  (`WebPkiServerVerifier::verify_server_cert`) → no regression for the reqwest
  client in `proxy_e2e.rs`. **[tested]**
- `CertificateRevocationListParams::signed_by` rejects an issuer lacking
  `KeyUsagePurpose::CrlSign`. **Our root already sets `CrlSign`**
  (`cert_authority.rs:157`), so no root change is needed to sign CRLs.
  **[read from source + tested]**

### What would change, and where

1. **`crates/core/src/proxy/cert_authority.rs`**
   - `GateCa::new` takes an extra `crl_url: Option<String>`, stored on the struct.
   - In `gen_cert`, when present:
     `params.crl_distribution_points = vec![CrlDistributionPoint { uris: vec![url] }];`
   - Add a `pub(crate) fn sign_crl(&self) -> CertificateRevocationList` (or a free
     function over the `Issuer`) producing an empty CRL with
     `this_update = now - 1h`, `next_update = now + 7d`, `crl_number = 1`.
     The `-1h` backdate matters for clock skew, same reasoning as the existing
     `NOT_BEFORE_OFFSET_SECS`.
   - `Option` rather than unconditional: Linux has no PAC listener, so there is
     no URL to point at there (and no client that would check it).

2. **`crates/core/src/proxy/engine.rs`**
   - `serve_pac` currently ignores the request entirely - it reads up to 1024
     bytes, does not parse, and returns the PAC body for *any* path
     (`engine.rs:744-752`). It must parse the request line and branch:
     `/gate-ca.crl` → `200 application/pkix-crl` with the DER body; anything
     else → the PAC script as today. Renaming it (`serve_loopback_http`) would
     stop the name lying.
   - The `pac_listener` is bound at `engine.rs:788`, before `GateCa::new` at
     `engine.rs:855`, so `pac_port` is already in hand when the CA is built -
     no reordering needed.
   - Regenerate the CRL per request (one ECDSA signature; CryptoAPI caches, so
     this is not a hot path) rather than caching one that can outlive its
     `nextUpdate` on a long-running engine. This is the same failure mode
     `LEAF_RENEW_MARGIN_SECS` already exists to prevent for leaves.

3. **`crates/core/src/proxy/engine.rs` - `pac_script` (`engine.rs:699`)**
   - Add `if (h === "127.0.0.1") return "DIRECT";` before the upstream fallback.
     Today, when the user had a pre-existing corporate proxy, the fallback is
     `return "PROXY <upstream>"`, and `isPlainHostName("127.0.0.1")` is **false**
     (it has dots), so a loopback CRL fetch that honours the PAC would be sent to
     the corporate proxy and fail. This is a real bug the CDP would walk into.
     **[read from source]**

4. **Tests** - `crates/core/tests/proxy_e2e.rs` currently passes
   `--ssl-no-revoke` to curl at two sites (lines ~435 and ~620) with comments
   naming exactly this problem. Those become the regression test: drop the flag
   on Windows, or better, assert the CRL endpoint serves a parseable CRL.

### The root-CDP caveat - RESOLVED, no root change needed

**Tested (section 8): the root does not need a CDP.** Leave the rest of this
subsection as the reasoning that made it the key question.



Because curl uses `SCH_CRED_REVOCATION_CHECK_CHAIN` and **not** `..._EXCLUDE_ROOT`,
the root is nominally in scope for revocation checking too. Our root has no CDP.

My expectation is that CryptoAPI skips revocation for a self-signed trust anchor
(there is no separate issuer to publish a CRL, and the `..._EXCLUDE_ROOT` flag
would be pointless if the root were always skipped anyway - but equally, checking
a self-signed root against a CRL it signs for itself is circular). **I could not
verify this**, and it is the single highest-value thing to test on a real box.

If the root *is* checked, option A needs a CDP on the root as well, and that is
much more expensive:

- It changes `ca_certificate_params()`, so every existing user's root must be
  regenerated and **re-trusted** - which on Windows means the native
  trust-confirmation dialog again, and on macOS the Security-Agent dialog.
- The auto-regeneration mechanism would not even fire: `catalog_host_fingerprint()`
  hashes the **host set only** (`cert_authority.rs:59-85`). Changing the root's
  extensions leaves the fingerprint identical, so `host_fingerprint_is_current`
  returns true and `load_or_create` keeps the old root forever. You would have to
  fold a schema/version marker into the fingerprint input.
- The root's CDP URL would have to be stable across engine restarts *and* valid
  when the engine is off, since the root sits in the trust store permanently. The
  persisted port band helps but does not guarantee it.

**Test the root question before writing any code.** It is the difference between
a ~80-line change and a migration.

## 5. Blast radius beyond curl - TESTED

All rows below were **executed** on the guest (Windows 11 26200.9168) with the
test root in `LocalMachine\Root`, against a leaf with no CDP (today's shape) and
a leaf with a loopback CDP (the fix). No row here is inferred.

| Client | no-CDP leaf (today) | loopback-CDP leaf (fix) |
|---|---|---|
| `curl.exe` 8.21.0 (Schannel), default | **FAIL** exit 35, `CRYPT_E_NO_REVOCATION_CHECK (0x80092012)` | **200** |
| `curl.exe --ssl-revoke-best-effort` | 200 | 200 |
| PowerShell 5.1 `Invoke-WebRequest` | 200 (soft-fails) | 200 |
| .NET `HttpWebRequest`, revocation **off** (the default) | 200 (soft-fails) | 200 |
| .NET `HttpWebRequest`, revocation **on** | **FAIL** `remote certificate is invalid according to the validation procedure` | **200** |
| WinHTTP COM (`WinHttpRequest.5.1`) | 200 (soft-fails) | 200 |

Two conclusions:

1. **The default-configured Windows HTTP stacks all soft-fail.** WinHTTP,
   WinINet-lineage, PowerShell and .NET do not check revocation unless a caller
   opts in (`ServicePointManager.CheckCertificateRevocationList = $true`, which
   defaults to `false`). The breakage is narrower than feared.
2. **`curl.exe` is the outlier, and it is the one that matters** - it is the
   binary in every "paste this to install" instruction, it ships in System32,
   and it hard-fails with no way for a script author to know in advance.

The fix repairs **every** client that was broken and regresses none.

### The tools this app integrates with

Unchanged from the earlier reading, and consistent with the above: Claude Code
(Node/OpenSSL), Codex (Rust/rustls - and rustls acceptance of a CDP leaf is
**[tested]** in section 4), OpenCode (Node/Bun), Cowork / Claude Desktop
(Electron/BoringSSL) do not check revocation. **None of Gate's own integrations
is affected.** What breaks is the ambient Windows tooling the user reaches for
while the proxy is on - which is still worth fixing, because "curl stopped
working while Gate is on" is exactly the experience that destroys the
reassuring-gatekeeper feeling.

## 6. macOS / Linux

**Linux: not affected.** OpenSSL/GnuTLS clients do not check revocation by
default, and there is no PAC listener (`serve_pac` is
`#[cfg(any(target_os = "windows", target_os = "macos"))]`).

**macOS: not currently affected, and a CDP will not regress it.** Apple's
Security framework does revocation best-effort for non-EV certificates and does
not hard-fail on an unreachable or absent CDP. This is the same class of bug as
the precedent in the module header - Network.framework rejecting leaves without
`serverAuth` EKU while OpenSSL accepted them - but the polarity is different:
there, Apple was the strict one; here, Windows is.

Adding a CDP is verified non-regressive for rustls **[tested]** and OpenSSL
**[tested]**. For Apple's stack it is **[inferred]** - a loopback CDP that
Security decides to fetch would be a new network dependency inside the
handshake. Gating the CDP behind `cfg(windows)` sidesteps the question entirely
and costs nothing, since Windows is the only platform with a client that cares.
**I would ship it Windows-only.**

## 7. Open questions that need a real Windows box (SETTLED - see section 8)

I started the `win11` libvirt VM on this host to try to settle these. It boots and
has SSH listening on `192.168.122.130:22`, but only password/keyboard-interactive
auth is offered for every account I tried, and entering credentials is not
something I'll do. **The VM is currently left running** - `virsh shutdown win11`
to stop it.

In priority order:

1. **Is the self-signed root revocation-checked under
   `SCH_CRED_REVOCATION_CHECK_CHAIN`?** Decides whether option A is 80 lines or a
   migration. See the caveat in §4.
2. **Does CryptoAPI fetch an `http://127.0.0.1:<port>/` CDP at all?** It may
   refuse loopback or non-standard ports, and `cryptnet.dll` may use the *machine*
   WinHTTP proxy config (`netsh winhttp`) rather than the user's WinINet/PAC
   settings - in which case the PAC fix in §4.3 is unnecessary but a machine-level
   corporate proxy setting becomes the hazard instead.
3. **How long does CryptoAPI cache the CRL, and does it honour `nextUpdate`?**
   Affects how often the endpoint is hit and whether a stale cache survives an
   engine restart onto a different port.
4. **Which of the §5 clients actually hard-fail?** Especially PowerShell 7 and
   any Store app, which I have only inferred.
5. **Confirm the reporter had `claude-web` enabled**, per §1 - this needs no
   Windows box, just a question to them.

A ready-to-run script for questions 1-4 is at
`docs/schannel-revocation-verify.ps1`.

## 8. Windows test results (2026-08-25) - all questions settled

Executed on the `win11` libvirt guest over SSH. **Windows 11 build 26200.9168,
`curl 8.21.0 (Windows) libcurl/8.21.0 Schannel`** - the real backend, not a
substitute.

### Method

Synthetic certificates minted on the Linux host with rcgen using the **exact
parameter sets from our code** (`ca_certificate_params()` and
`GateCa::gen_cert()`, reproduced field-for-field in a scratch crate, including
the DNS name constraints), varying only the CRL distribution point:

| Set | Root | Leaf | TLS port |
|---|---|---|---|
| A | no CDP | no CDP | 14431 |
| B | no CDP | CDP -> `http://192.168.122.1:8000/r1.crl` | 14432 |
| C | no CDP | CDP -> `http://127.0.0.1:18080/r1.crl` | 14433 |
| E | CDP (served) | CDP (served) | 14434 |

TLS served by `openssl s_server` on the host; the guest reached it with
`curl --resolve gate-test.example:<port>:192.168.122.1`, so **no `hosts` file
entry and no trust-store change were needed**. Trust was supplied with
`--cacert`.

That last point needs a caveat stated up front: `--cacert` puts curl on the
`SCH_CRED_MANUAL_CRED_VALIDATION` path, where curl validates the chain itself in
`schannel_verify.c` rather than letting schannel do it. Reading that file
(`schannel_verify.c:762-806`) confirms it uses the **same**
`CERT_CHAIN_REVOCATION_CHECK_CHAIN` flag and hard-fails on
`CERT_TRUST_REVOCATION_STATUS_UNKNOWN` unless `--ssl-revoke-best-effort` is
given - so the mechanism under test is the same, but the error text differs
(`schannel: the revocation status is unknown`, exit 60) from the reported
`CRYPT_E_NO_REVOCATION_CHECK` on the `AUTO_CRED_VALIDATION` path. See the
still-open item below.

### Results

| Test | Result | Meaning |
|---|---|---|
| **A default** | **exit 60**, `schannel: the revocation status is unknown` | **Bug reproduced** on today's cert shape |
| A `--ssl-revoke-best-effort` | exit 0, HTTP 200 | Workaround confirmed |
| A `--ssl-no-revoke` | exit 0, HTTP 200 | Workaround confirmed |
| **B default** | **exit 0, HTTP 200** | **Leaf-only CDP is sufficient** |
| **C default** | **exit 0, HTTP 200** | **Loopback CDP works** |
| E default | exit 0, HTTP 200 | Control |

### Q1 - is the self-signed root revocation-checked? **No.**

Test B passed with a leaf carrying a CDP and a root carrying **none**. Since
curl requests `CERT_CHAIN_REVOCATION_CHECK_CHAIN` (whole chain *including* root,
not the `..._EXCLUDE_ROOT` variant), a chain engine that actually demanded
revocation data for the anchor would have failed B exactly as it failed A. It
did not.

**Consequence: the expensive path is off the table.** No root regeneration, no
re-trust dialog for existing users, no need to fold a version marker into
`catalog_host_fingerprint()`. The fix is leaf-only, which is the ~80-line
version.

### Q2 - does CryptoAPI fetch an `http://127.0.0.1:<port>` CDP? **Yes.**

Proven with the cache flushed so the result could not come from a warm entry:

1. `certutil -urlcache "http://127.0.0.1:18080/r1.crl" delete` -> `entries deleted: 1`
2. Confirmed the in-guest listener log had **zero** hits.
3. **One** handshake -> `HTTP=200`, `EXIT=0`.
4. Listener log: exactly one `HIT ... GET /r1.crl HTTP/1.1`.
5. Cache repopulated with that URL.

So CryptoAPI issues a plain-HTTP GET to a **loopback, non-standard-port** CDP
mid-handshake, accepts a `Content-Type: application/pkix-crl` DER body, and is
satisfied by an empty CRL. This is the load-bearing assumption of the
recommendation and it holds.

An earlier apparent pass for C was **not** valid evidence and nearly misled me:
the listener had been started with `Start-Process` from an SSH session and was
killed when that session ended, so the 200s were being served from CryptoAPI's
cache. Starting the listener detached via `([WMICLASS]"Win32_Process").Create(...)`
made it survive across sessions and allowed the clean run above.

### Q3 - caching

- Per-**user** cache: `%LOCALAPPDATA%Low\Microsoft\CryptnetUrlCache\{Content,MetaData}`,
  keyed by URL. Our entry: 249 bytes, `Last Sync Time` recorded.
- **Reused across processes**: four consecutive `curl.exe` invocations produced
  **one** listener hit and three cache hits.
- Honouring of `nextUpdate` (ours was +7d) is **[inferred]** - measuring the real
  expiry would need clock manipulation and was not attempted.

Practical read: the endpoint is hit rarely, so regenerating the CRL per request
is cheap. And because the cache is keyed by URL, an engine restart onto a
different port simply misses and refetches - no stale-cache hazard.

### Incidental confirmation of the PAC gap (section 4.3)

The Linux host running this test has Gate's own proxy env active
(`HTTP_PROXY=http://127.0.0.1:32841`, `no_proxy=localhost,127.0.0.1,::1`). A
plain `curl http://192.168.122.1:8000/...` from that shell returned **502 Bad
Gateway** - the request went to Gate's proxy because `192.168.122.1` is not in
the bypass list. Same shape as the PAC gap flagged in section 4.3: loopback is
excluded, other private addresses are not. It did not affect the CDP test (the
guest's proxy is disabled, `ProxyEnable=0`), but it is live evidence that the
bypass list needs widening, and a reason to keep the CDP host **literally
`127.0.0.1`** rather than any other local address.

### Second round: the `AUTO_CRED_VALIDATION` path (trust store)

With the throwaway root installed into `LocalMachine\Root` and `--cacert`
dropped, schannel does the validation internally - the exact configuration that
produced the original report.

| Test | Result |
|---|---|
| A (leaf no CDP) | **exit 35, `schannel: next InitializeSecurityContext failed: CRYPT_E_NO_REVOCATION_CHECK (0x80092012)`** |
| B (leaf CDP -> host, root no CDP) | exit 0, HTTP 200 |
| C (leaf CDP -> loopback) | exit 0, HTTP 200 |

Test A reproduces the reported error **verbatim**, including the exit code and
the hex HRESULT. The mechanism is settled.

### The confound I had to design around

Test C's first pass on this path was **not** valid evidence. `certutil -urlcache
<url> delete` flushes only the CryptNet *URL* cache; CryptoAPI also matches a
cached CRL to a chain by **issuer**, so the `r1.crl` that test B had just fetched
from `192.168.122.1:8000` satisfied test C's check without any loopback request
(the listener log stayed empty while curl still returned 200).

The clean experiment needed a **fresh issuer whose CRL is reachable at no other
URL**: root R3 (new key, no CDP on the root, its own name constraint) issuing a
leaf whose only CDP is `http://127.0.0.1:18080/r3.crl`. Installed R3, flushed,
ran one handshake:

```
HTTP=200 EXIT=0
listener: 2026-08-25T12:34:06 HIT GET /r3.crl
```

That is the definitive result. **On the real `AUTO_CRED_VALIDATION` path, with
the root in the Windows trust store: schannel fetches a loopback CDP over plain
HTTP mid-handshake, accepts an empty CRL, and does not require the root to carry
revocation information.** Both Q1 and Q2 are closed in favour of the leaf-only
fix.

### Incidental findings

- **CN collision.** My test root used the same subject CN as the shipping CA
  (`Gate Connect Local CA`), and the guest has a real Gate install whose CA sits
  in `CurrentUser\Root` (thumbprint `5A3F5FBF...`, distinct from my test root's
  `55DD2F81...`). `ca_windows.rs` looks up by CN in places (`certutil -delstore
  Root <CN>`), so two roots sharing a CN in different stores is a state the trust
  reconciliation logic can encounter. Not a bug found, but worth a look.
- **The 4096 validity is real in the wild.** The installed Gate CA reports
  `NotAfter: 12/31/4095`, confirming the rcgen default noted in section 9 reaches
  actual users.
- **Test harness note**, in case anyone repeats this: `openssl s_server` is
  single-threaded and one stalled connection blocks every later probe, which
  produced a spurious "PowerShell times out" result. Replaced with a small
  threaded rustls server. Separately, `Start-Process` from an SSH session is
  killed when that session ends - use `([WMICLASS]"Win32_Process").Create(...)`
  for anything that must outlive the call - and `R` is a built-in alias for
  `Invoke-History`, so it is a poor choice of helper-function name.

### Cleanup

Both test roots removed from `LocalMachine\Root` and verified absent from both
machine and user stores; the two `hosts` entries removed; the loopback listener
stopped; CryptNet cache entries flushed; the guest test directory deleted. Gate's
own pre-existing CA in `CurrentUser\Root` was identified by thumbprint and left
untouched. Host-side TLS and CRL servers torn down.

## 9. Things I could not verify

After the Windows round, this list is short:

- **`nextUpdate` expiry semantics.** The CRL is demonstrably cached and reused
  across processes, and the cache is keyed by URL, but I did not measure how long
  CryptoAPI honours `nextUpdate` - that needs clock manipulation. Low risk: the
  engine regenerates the CRL per request, so a cache miss is always serviceable.
- **Apple's Security framework with a loopback CDP.** Untested, and moot under
  the recommendation, which gates the extension on `cfg(windows)`.
- **Windows Store / UWP apps.** Not tested; they were the one row of the original
  blast-radius table I had no clean way to drive. Given that WinHTTP, WinINet,
  .NET and PowerShell all soft-fail by default, a Store app would have to opt
  into revocation explicitly to be affected.
- **Whether the original reporter had `claude-web` enabled** (section 1). This
  needs a question to them, not a machine. It does not change the fix.
- **The `Not After: 4096` root validity** is confirmed to reach real installs but
  is unrelated to this bug and unexamined. Worth its own look.
