# Agent-harness integration validation (OpenClaw, Hermes)

Validated 2026-08-03 against upstream documentation. Both integrations are
**hidden from the popover UI** as of this document; they remain in
`registry::registry()` and in the `gate-connect` CLI.

Why hidden rather than removed: anyone who already connected one of these
tools with an earlier build has a config pointing at the loopback relay.
Removing them from the registry would strand that config, because
`disconnect_all_managed`, the master-off sweep
(`snapshot_and_disable_everything`) and `restore_swept_tools` all walk the
registry. Hiding is done at the UI boundary only (`list_tools` in
`src-tauri/src/lib.rs`), via `Integration::hidden_in_ui`.

Sources consulted:

- <https://docs.openclaw.ai/concepts/model-providers>
- <https://docs.openclaw.ai/concepts/oauth>
- <https://deepwiki.com/openclaw/openclaw/3.3-model-providers-and-authentication>
- <https://github.com/NousResearch/hermes-agent/blob/main/cli-config.yaml.example>
- <https://hermes-agent.nousresearch.com/docs/user-guide/configuring-models>

## What validated cleanly

**OpenClaw config shape.** `models.providers.<id>` with `baseUrl` (camelCase)
and `headers` are the documented key names, and match what
`integrations/openclaw.rs` writes. `~/.openclaw/openclaw.json` with the
`OPENCLAW_CONFIG_PATH` override is correct. Merge semantics (`mode: "merge"`
is the default) mean `apiKey`, `api`, model lists and other user options
survive, which is what our merge relies on.

**OpenClaw loopback redirect is explicitly permitted.** The docs state that a
custom endpoint "trust[s] that exact configured `scheme://host:port` origin
for guarded model requests, including loopback, LAN, and tailnet hosts", so
pointing `baseUrl` at the relay needs no
`models.providers.<id>.request.allowPrivateNetwork: true`. Note the corollary:
*different ports* are not covered by that trust. A relay port change makes the
stale origin both wrong and policy-blocked, so it fails harder than a plain
bad URL. Our drift detection covers the detection side.

**Hermes `model.base_url` and `model.default_headers` are both correct.** A
secondary source claimed the header key must be `extra_headers`; the reference
config disproves that: `extra_headers` is "accepted as an alias of
`default_headers` (merged, with `extra_headers` winning when both are set)".
Our `HERMES_DEFAULT_BASE_URL` of `https://openrouter.ai/api/v1` matches the
documented default exactly.

## Findings

### H1 (blocking) - Hermes: `default_headers` is OpenAI-wire only

`cli-config.yaml.example` says of `default_headers`, verbatim: **"Applies on
the OpenAI wire only (not native Anthropic / Bedrock)."**

`integrations/hermes.rs` never reads `api_mode` or transport. On a
native-Anthropic Hermes setup our `X-Gate-Upstream-Url` is therefore never
sent, and the relay rejects the request outright:
`proxy/relay.rs:392` returns `missing X-Gate-Upstream-Url header`. That is a
hard failure, not a degradation.

Worse, `status()` still reports `Connected`, because it only checks that
`model.base_url` equals the expected relay URL and that the header key exists
in the file. The file is correct; the wire ignores it.

Fix options: read `api_mode` and refuse to connect with a real reason, or
write the hint where the native wire reads it. Do not ship the current
combination of a hard failure and a green pill.

### H2 - OpenClaw: auth-profile discovery reads a location OpenClaw does not use

`auth_profile_providers()` reads `settings["auth"]["profiles"]` from inside
`openclaw.json`. Per the docs, auth profiles live per agent at
`~/.openclaw/agents/<agentId>/agent/auth-profiles.json`, and in current
versions inside the agent's SQLite store
(`~/.openclaw/agents/<agentId>/agent/openclaw-agent.sqlite`), with legacy
`auth.json` and `credentials/oauth.json` paths. No documentation describes an
`auth.profiles` block in `openclaw.json`.

If that is right, the second discovery signal finds nothing in practice, and a
provider configured purely through `openclaw models auth login` - with no
explicit `models.providers.<id>` block - is silently not routed. The unit test
`auth_profile_providers_discovers_plugin_providers` asserts the
`openclaw.json` shape, so it encodes the same assumption rather than checking
it against a real install.

Needs verification against a real OpenClaw installation before either reading
the agent store or dropping the signal and documenting the limitation.

### H3 - OpenClaw: we trigger Anthropic beta suppression and do not disclose it

For `api: "anthropic-messages"` on any host that is not public
`api.anthropic.com`, OpenClaw suppresses implicit beta headers - the docs name
`claude-code-20250219` and `interleaved-thinking-2025-05-14` - plus OAuth
markers, so proxies do not reject unsupported beta flags.

Pointing `anthropic` at our relay triggers exactly this, so enabling Gate
silently drops interleaved thinking for OpenClaw users. The documented remedy
is setting `headers["anthropic-beta"]` explicitly, which we could do, since we
already own that header map.

### H4 - OpenClaw: two further silent behaviour deltas

- On non-native `openai-completions` endpoints OpenClaw forces
  `compat.supportsDeveloperRole: false`, overriding an explicit `true`, and
  skips native-only shaping (`service_tier`, Responses/Completions `store`,
  prompt-cache hints, reasoning-compat).
- OpenClaw adds its attribution headers (`originator`, `version`,
  `User-Agent`) only on native `api.openai.com`, `chatgpt.com/backend-api`
  and verified `openrouter.ai` routes, never on a generic proxy. The gateway
  therefore sees different headers than a direct call.

Neither is a defect on our side; both are worth knowing if Gate keys anything
off those headers.

### H5 - Hermes: a fresh install ships `model: ""`

The docs state a new install has `model: ""`, a string sentinel upgraded to a
mapping by the first `hermes setup` / `hermes model`. `connect()` does
`if !settings.contains_key("model") { insert mapping }`, so an existing empty
*string* is not replaced, and the following `as_mapping_mut()` bails with
"model is not a mapping". The refusal is correct; the message is opaque for
what is the default state of an unconfigured install.

### H6 - Hermes: only `model.base_url` is covered

Hermes also supports a `providers:` dict (and a legacy `custom_providers:`
list) with per-entry base URL - `api`, `base_url` and `url` are accepted
aliases - and per-entry `extra_headers`. We touch none of it, so a Hermes user
driving a custom provider entry is not intercepted at all. This is the
OpenCode-style fan-out that Hermes does not currently get.

### H7 - OpenClaw: stale module doc

The module header says a provider set up purely via auth login "isn't
auto-discovered", but the code later gained a second discovery signal
intended to do exactly that. The comment contradicts the code regardless of
whether the signal works (see H2).

## Suggested order of work

1. H1. A silent hard failure under a green "Routed" pill is the worst
   combination in the product.
2. H2. Verify against a real install, then either read the agent store or
   document the limitation honestly.
3. H3. Preserve the betas, or surface what changed.
4. H5, H7, H6 are lower stakes.
