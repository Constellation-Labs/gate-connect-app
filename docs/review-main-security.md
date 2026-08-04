# Security Review - Gate Connect (branch: main)

Scope: credential handling, TLS interception / CA, privileged helper + IPC,
system-proxy config, path/TOCTOU, frontend. Method: read the code and traced
each data flow. Confidence tagged CONFIRMED (traced end to end) or PLAUSIBLE
(likely but not fully exercised).

## Summary

| Severity | Count |
|----------|-------|
| Critical | 0 |
| High     | 1 |
| Medium   | 1 |
| Low      | 3 |

Overall the app is carefully built. The privileged-exec, helper-IPC,
atomic-file-write, and frontend surfaces are notably well hardened (details in
"What holds up" at the bottom). The one finding that matters for the product's
central promise is that the **Gate workspace API key is written in plaintext
into each CLI tool's own config file** while connected - so the literal claim
"the credential isn't sitting in a config file" does not hold for the
config-rewrite integrations (it does hold for the proxy/MITM path and for the
upstream provider key).

### Credential-in-keychain-not-config promise: partially upheld

- Upstream provider key (Anthropic/OpenAI, etc.): Gate Connect never stores or
  writes it. The tool keeps its own. Promise upheld. (`account.rs:1-8`)
- Gate account key (`sk-gw-...`): stored in the OS credential store
  (`account.rs`, `keychain.rs`). On the **proxy/MITM path** it is injected into
  requests at runtime from the keychain and never touches disk
  (`engine.rs:316-317, 405-408`). Promise upheld.
- **Config-rewrite integrations** (claude_code, codex, opencode, openclaw,
  hermes): the Gate key is written in cleartext into the tool's config file.
  Promise NOT upheld here. See Finding 1.

---

## Finding 1 - Gate API key written in plaintext to tool config files (and followed into synced/cloud locations)
Severity: High - Confidence: CONFIRMED

The Gate workspace key (`sk-gw-...`, a bearer credential that authorizes
billable traffic through the user's gateway) is written verbatim into the
connected tool's on-disk config:

- Claude Code: into `~/.claude/settings.json` `env.ANTHROPIC_CUSTOM_HEADERS`
  as `X-Gate-Api-Key: <key>` - `claude_code.rs:154` (`build_headers(&acct.api_key, ...)`),
  written at `claude_code.rs:318-324`.
- Codex: into `~/.codex/config.toml` `[model_providers.gate.http_headers]` -
  header write region around `codex.rs:391`, persisted `codex.rs:551-555`.
- OpenCode: into `opencode.json` `provider.<id>.options.headers` -
  `opencode.rs:463-474`, persisted `opencode.rs:669-676`.
- Same pattern in `openclaw.rs:692-699` and `hermes.rs:381-390`.

Data flow: keychain -> `Account.api_key` -> integration `connect()` -> config
file. So the key that the keychain is meant to protect is copied out into a
plaintext file whenever a CLI tool is connected.

Amplifier (this is what pushes it above the tools' own baseline): `write_file`
deliberately resolves symlinks and writes the *real target*
(`primitives.rs:21-28`). If the user's `settings.json` / `config.toml` is
symlinked into Dropbox / iCloud / a dotfiles repo (common), the Gate key is
written into that synced location and leaves the machine. The keychain promise
exists precisely to prevent a secret from ending up in a synced file.

Mitigations already in place (why it is not Critical):
- All these files are written `0o600` and the tempfile is created at that mode,
  so there is no transient world-readable window (`primitives.rs:57-79`).
- Disconnect fully removes the key from the config - no backup file is left
  (`disconnect_zero_residue.rs:1-5`; verified by the zero-residue test).
- It is the *gateway* key, not the upstream provider key.
- For env-var/config CLI tools there is no runtime-injection alternative the way
  there is for the MITM path, so this may be an accepted design tradeoff.

Recommendation: if it must live in config, prefer an indirection that keeps the
literal secret out of the file - e.g. Claude Code's `apiKeyHelper` (a command
that fetches the key from the keychain at runtime) instead of inlining it into
`ANTHROPIC_CUSTOM_HEADERS`; and refuse to follow a symlink that points outside
the tool's own config dir (or warn) so the key can't be written into a synced
folder. At minimum, make the docs/UI honest that connecting a CLI tool places
the Gate key in that tool's config file.

---

## Finding 2 - CA trust and CA private key persist after disconnect; Linux trust is system-wide
Severity: Medium - Confidence: CONFIRMED

Disconnecting the proxy does not remove the MITM trust anchor. Removing trust is
a separate explicit action (`manager.rs:11`, `manager_windows.rs:12` -
"removing it is a separate explicit action (`untrust_ca`)"). After a normal
disconnect: the CA cert remains installed as a trusted root and its private key
remains in the OS credential store. So the machine's capacity to have its
AI-provider TLS transparently intercepted is unchanged by "disconnect"; only an
explicit untrust reduces it.

Blast radius if the CA private key is ever exfiltrated from the credential store
(or the app is compromised while it holds the key in-process): the attacker can
mint server certs the machine will trust. It is bounded - the CA carries DNS
Name Constraints (`cert_authority.rs:22,54` + permitted subtrees from
`default_domains()`), so forged certs are limited to the built-in AI-provider
catalog, not the whole internet - but that catalog is exactly the high-value set
(api.anthropic.com, api.openai.com, generativelanguage.googleapis.com, ...).

On Linux this is worse than macOS/Windows: the CA is installed into the
**system** trust store (`ca_linux.rs:1-11`), so it is trusted by every user on
the box and by curl/git/openssl, and it stays there until explicit untrust.

Note on constraint scope: `permitted_subtrees` is the *entire catalog*, not just
the domains the user enabled. The runtime `should_intercept` gate limits what is
actually MITM'd, but the installed anchor's signing capability spans all catalog
domains.

Recommendation: untrust + remove the CA on disconnect (re-trusting is one
prompt), or surface persistent-trust state in the UI so the user knows the
anchor is still live after disconnect. Consider narrowing name constraints to
the enabled subset.

---

## Finding 3 - macOS CA trusted for all policies (no `-p ssl` scoping)
Severity: Low - Confidence: CONFIRMED

`security add-trusted-cert -r trustRoot -k <login keychain> <cert>`
(`ca.rs:210-214`) installs the CA as a trust root with no `-p` policy
restriction, so it is trusted for every trust policy (SSL, S/MIME, code signing,
etc.), not just TLS server auth. Practical exploitability is low because leaf
certs are `ExplicitNoCa` with only the `serverAuth` EKU (`cert_authority.rs:139`
+ EKU) and the DNS name constraints apply to TLS, but broadening trust to all
policies is looser than the design needs. The command is invoked with argv (not
a shell), so there is no injection concern here.

Recommendation: add `-p ssl` to scope trust to TLS server evaluation.

---

## Finding 4 - Debug logging records the full request path incl. query string
Severity: Low - Confidence: PLAUSIBLE

When `GATE_PROXY_DEBUG` is set, the handler logs the request line including the
full path (`engine.rs:303, 331-347`). Header-borne keys are redacted
(`x-goog-api-key` -> `<redacted>`, auth reduced to "bearer"/"x-api-key",
`engine.rs:337-359`), but query strings are not. Providers that pass the key as
a URL query param (e.g. Google `generativelanguage.googleapis.com/...?key=...`)
would have that key written to the log in cleartext. Opt-in via env var, so
severity is Low.

Recommendation: strip/redact the query string (or log only the path segment)
before logging.

---

## Finding 5 - CA name constraints cover the whole catalog, not the enabled subset
Severity: Low - Confidence: CONFIRMED

The installed trust anchor's `permitted_subtrees` is populated from
`default_domains()` (the full built-in catalog), not from the set of domains the
user actually enabled (`cert_authority.rs:54-67`). Runtime interception is still
gated per-host by `should_intercept`, so this does not widen live MITM, but it
means the trust anchor sitting in the OS store is capable of signing for every
catalog AI-provider domain regardless of what the user turned on. Sub-point of
Finding 2; listed separately because it is a concrete, easily-tightened property
of the cert itself.

Recommendation: build name constraints from the enabled-domain set (and rebuild
on toggle) rather than the whole catalog.

---

## What holds up (verified, no change needed)

- Upstream TLS is validated: the engine uses a real verifying rustls connector,
  `with_rustls_connector(aws_lc_rs::default_provider())` (`engine.rs:528`,
  hudsucker 0.24.1) - not a no-verification config. The gateway leg is not
  downgraded.
- CA private key never hits disk: stored in macOS Keychain / Windows Credential
  Manager / Linux Secret Service; only the public cert is on disk
  (`ca.rs:41-46, 103-109`, `ca_linux.rs:1-4, 41-43`). On Linux it is passed to
  the daemon only over the 0600 UID-checked socket.
- Privileged exec is injection-safe: `sh_quote` correctly single-quote-wraps and
  escapes (`primitives.rs:155-159`); every interpolated value is a static
  constant or an app-controlled path (`ca.rs:183-186`, `ca_linux.rs:174-179`,
  `system_proxy.rs:142-144`). macOS trust uses argv, not a shell (`ca.rs:210`).
- Helper IPC access control is layered and sound: `$XDG_RUNTIME_DIR` Unix socket
  (never TCP), 0700 dir / 0600 socket, `SO_PEERCRED` UID check, per-run token,
  and every requested intercept domain validated against the built-in catalog
  so an authenticated caller still cannot MITM an arbitrary host
  (`control.rs:10-19`, `helper.rs:1-11`, `validate_domains` test at
  `control.rs:212-213`).
- File writes are robust: atomic stage+fsync+chmod+rename, symlink-resolved,
  tempfile created already at the target mode so the payload is never transiently
  world-readable (`primitives.rs:20-81`). All secret-bearing files (account,
  domains, tool configs, integration state) are `0o600`; the non-secret Linux
  proxy drop-in is `0o644` and contains only proxy env vars + `NODE_EXTRA_CA_CERTS`
  (`system_proxy_linux.rs:1-7, 171`), no key.
- System-proxy cleanup is real: enable snapshots prior state and disable restores
  it; the zero-residue disconnect test confirms Gate edits and the in-file marker
  are fully reverted with no backup file left
  (`disconnect_zero_residue.rs:1-5, 231-233`).
- Frontend leaks nothing: `get_account` returns only `has_api_key: bool`, never
  the key; the UI shows a masked `sk-gw-••••` placeholder (`Settings.tsx:155`);
  keys flow one-way into `invoke`; no secret in `localStorage` (only a "seen"
  onboarding flag) and no `console.log` of keys / no `dangerouslySetInnerHTML`.
- Tauri command surface is narrow and validated: no command returns a stored
  secret, base URLs are `https://`-only (`claude_code.rs:145-149`), keys are
  prefix/shape-validated (`lib.rs:28`), and the onboarding deep-link source is
  normalized to known values - no arbitrary file-read or command-exec command.
