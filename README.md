# Gate Connect

Menu-bar app and `gate-connect` CLI that point your AI dev tools at a
[Constellation Gate](https://www.notion.so/Gate-Connect-PRD-35da94bd4b4f81afa883eebceab0f367)
gateway and keep the credential out of config files. Sign in once, flip a
provider on, and Gate Connect configures the tools you have installed - and,
for apps that have no gateway setting of their own (Claude Desktop / Cowork,
ChatGPT desktop), routes their traffic through a built-in proxy instead.

## Scope

- **Surfaces:** Tauri 2 menu-bar app (primary) and a `gate-connect` CLI. Both
 invoke the same `gate-connect-core` crate; nothing is implemented in only one
 place. The app lives in the menu bar / tray with no dock icon. Left-click the
 tray icon to toggle a popover anchored underneath; right-click for the menu.
- **Platforms:** macOS, Windows, and Linux.
- **Account:** one Gate AI login, configured once and reused for every tool.
 The gateway base URL lives in a small JSON file; the Gate API key
 (`sk-gw-...`) lives in the OS secret store (macOS Keychain, Windows
 Credential Manager, Linux Secret Service). It never lands in a config file.
- **Providers** (the user-facing switches): Claude Code / Cowork, OpenAI /
 Codex, OpenRouter, and Google / Gemini. Each provider orchestrates its
 config-file tool integrations and, when the proxy is running, its proxy
 domains - so the UI shows one toggle instead of the proxy-vs-config split.
- **Tools** (config-file integrations): Claude Code, Codex, OpenCode,
 OpenClaw, and Hermes. Each edits its own config file to route through Gate;
 no proxy or CA needed.
- **Config-less apps:** Claude Desktop / Cowork and ChatGPT desktop have no
 gateway setting, so they route through the built-in proxy.

## Architecture

A `gate-connect-core` Rust crate holds the account, registry, provider layer,
native primitives, per-tool integrations, and the proxy engine. The Tauri
shell and the CLI both depend on it.

```
apps/connect/
├── src/          React + Tailwind frontend (Vite)
├── src-tauri/    Tauri 2 Rust shell (commands → core)
└── crates/
  ├── core/       gate-connect-core
  │   ├── account.rs       single Gate login (base URL + keychain key)
  │   ├── registry.rs      tool integrations + Status
  │   ├── provider.rs      user-facing provider switches
  │   ├── keychain.rs      OS secret store
  │   ├── integrations/    claude_code, codex, opencode, openclaw, hermes
  │   └── proxy/           built-in MITM proxy (engine, CA, system proxy)
  └── cli/        gate-connect: thin CLI binary
```

## Two ways a tool gets routed

**Config-file tools.** Claude Code, Codex, OpenCode, OpenClaw, and Hermes each
expose a gateway setting in their own config. Connecting one edits that config
(e.g. Codex writes `~/.codex/config.toml`) to point at the Gate base URL and
supply the workspace via `X-Gate-Api-Key`. Cross-platform, no CA, no admin
prompt.

**The built-in proxy.** Apps with no gateway setting (Claude Desktop / Cowork,
ChatGPT desktop) route through a local MITM proxy instead:

1. Enabling the proxy trusts a locally generated root CA and points the system
 HTTPS proxy at the loopback engine. The CA **private** key lives in the OS
 keychain; only the public cert is written to disk.
2. For each TLS `CONNECT` the engine decides - before any handshake - whether
 the target host is one we route. Hosts we don't route are blind-tunnelled
 untouched, so cert-pinning apps and every other site are unaffected.
3. For hosts we do route, the engine terminates TLS and rewrites inference
 requests to the Gate gateway with `X-Gate-Api-Key` and `X-Gate-Upstream-Url`
 injected. Non-inference paths on the same host (e.g. an auto-updater) pass
 through to the real upstream.

On macOS and Windows the system proxy is driven by a loopback **PAC** (proxy
auto-config) the engine serves: it routes only Gate's enabled hosts to the
engine and sends everything else direct - or to the user's pre-existing proxy,
which is preserved as the PAC fallback. So unrelated traffic (Teams, browsers,
other apps) never traverses the engine at all, not just at the `CONNECT` gate,
and a corporate proxy keeps working while routing is on. Linux has no PAC
equivalent and uses env-var proxying instead.

The enabled-domain set is hot-swappable, so flipping a provider on or off only
pushes new rules - no engine restart, no extra admin prompt. Disabling the
proxy restores the previous system-proxy state but deliberately leaves the CA
trusted, so re-enabling is promptless; untrusting is a separate explicit
action.

Platform specifics: CA trust and system-proxy wiring are per-OS - macOS via
`security` + `networksetup` (auto-proxy URL / PAC), Windows via `certutil` +
the per-user WinINET registry settings (`AutoConfigURL` / PAC), Linux via the
system trust store (`update-ca-certificates` / `update-ca-trust`) + a
user-scoped systemd `environment.d` drop-in. Because CLI tools (Node-based
ones especially) ignore the system proxy and read `HTTP(S)_PROXY` /
`NODE_EXTRA_CA_CERTS` instead, routing-on also injects those for new shells:
the Linux drop-in already carries them, macOS gets a managed `~/.zshenv`
block, and Windows gets per-user env vars under `HKCU\Environment`.

Provider policy is **config-first, proxy-if-already-on**: flipping a provider on
always configures its installed tools, and additionally flips its proxy domains
only when the proxy is already running - so the switch never triggers a CA or
admin prompt on its own.

## Run the app

From this directory:

```bash
pnpm install   # one-time (runs from any package in the monorepo)
pnpm app       # tauri dev - opens the Gate Connect window
```

`pnpm app` boots Vite on :5173, compiles the Rust shell, and starts the app.
There is no dock icon - look for the Gate Connect glyph in the menu bar / tray.

### Use it

1. Click the Gate Connect icon in the menu bar. A popover slides down.
2. Sign in with your gateway base URL and API key (stored once, in the
 keychain).
3. Flip on the provider for the tool you use. Config-file tools are configured
 immediately.
4. For Claude Desktop / Cowork, enable the proxy when prompted - macOS shows a
 native auth panel for the CA / system-proxy write. Fully quit and relaunch
 the app afterward.

The popover dismisses on focus loss (click outside) or by clicking the tray
icon again. Right-click the tray icon for **Quit**.

### CLI parity

For power users and scripting. Same registry, same code path:

```bash
# Account
gate-connect login --base-url https://your-gateway.example.com --api-key sk-gw-...
gate-connect whoami
gate-connect logout

# Config-file tools
gate-connect list                 # supported tools + current state
gate-connect status codex         # detailed status for one tool
gate-connect connect codex        # edit the tool's config to route through Gate
gate-connect disconnect codex

# Built-in proxy (macOS / Windows / Linux)
gate-connect proxy status
gate-connect proxy enable
gate-connect proxy disable
```

`status` reports `not installed`, `detected`, `connected`, or
`drifted: <reason>`.
