# `gate-connect` CLI reference

The CLI shares `gate-connect-core` with the Gate Connect desktop app, so
anything it does the app does too (same registry, same config edits, same secret store). Run
`gate-connect --help` or `gate-connect <command> --help` for the built-in
text.

## Quick start

```bash
gate-connect login --base-url https://your-gateway.example.com   # prompts for the key
# connect needs the proxy running: keep the Gate Connect app open, or run
# `gate-connect proxy enable` (on macOS/Windows it stays up, so use another terminal)
gate-connect connect codex
gate-connect status codex
```

## Account

| Command | What it does |
| --- | --- |
| `login --base-url <URL>` | Sign in. The URL goes to disk, the credential to the OS secret store (Keychain / Credential Manager / Secret Service). Re-run to update. |
| `logout` | Disconnect every tool Gate manages, then remove the stored base URL and the keychain entry. If a disconnect fails, you stay signed in. |
| `whoami` | Print the signed-in gateway URL, if any. |

`login` options:

| Option | Meaning |
| --- | --- |
| `--base-url <URL>` | Gateway URL. Required; also read from `GATE_BASE_URL`. |
| `--api-key <KEY>` | Gate API key on the command line. |
| `--api-key-file <PATH>` | Read the key from the first line of a file. |
| `--oauth` | Sign in through the Constellation Hosted UI in the browser instead of a key. Prints a URL and waits for the loopback redirect. Cannot be combined with `--api-key` / `--api-key-file`. |
| `--org <UUID\|SLUG>` | With `--oauth`, pick the organization up front. Chosen automatically when you belong to one; otherwise you get a numbered prompt. |

With neither `--api-key` nor `--api-key-file`, the key is read from a no-echo
prompt. Prefer the file or the prompt: they keep the secret out of shell
history and `ps` output.

## Tools

Tool slugs: `claude-code`, `codex`, `opencode`, `openclaw`, `hermes`,
`env-proxy`.

| Command | What it does |
| --- | --- |
| `list` | Supported tools and their current state. |
| `status <tool>` | Detailed status: `not installed`, `detected`, `connected`, or `drifted: <reason>`. |
| `connect <tool>` | Edit the tool's config to route through Gate. The tool keeps its own provider login; Gate forwards whatever it sends. Needs the proxy, and does not start it: Claude Code refuses to connect without it, and Codex and OpenCode connect to the relay's last address, which only answers while the proxy runs. |
| `disconnect <tool>` | Restore the tool's prior configuration. |

## Built-in proxy (macOS / Windows / Linux)

Routes apps with no config file (Claude Desktop, ChatGPT, ...) and CLI tools
through Gate. Only enabled provider domains are intercepted; every other host
is tunnelled untouched.

| Command | What it does |
| --- | --- |
| `proxy status` | Running or not, port, CA trust, provider domains. |
| `proxy enable` | Trust the local CA and point the system proxy at the loopback engine. May prompt for elevation. **Linux:** the engine runs in a background daemon, so the command returns and routing stays on until `proxy disable`. **macOS / Windows:** the engine lives in this process, so the command stays in the foreground. Stop it with Ctrl-C or by closing the terminal (or SIGTERM on macOS): it puts tools whose config names its relay or engine back on their own settings (the next enable or app launch reconnects them), stops routing and restores the prior proxy state. It refuses while the app, or another `enable`, is already routing. |
| `proxy enable --foreground` | Linux only. Stays up until Ctrl-C, SIGTERM or SIGHUP, then stops routing and restores the prior proxy state. For a systemd unit or CI job that should own the routing lifetime. |
| `proxy disable` | Turn routing off and restore the prior system-proxy state. Tool configs are left naming Gate, and the CA stays trusted (`untrust-ca` removes it). **macOS / Windows:** this is the cleanup after a host that stopped without restoring (`taskkill /F`, a crash); it refuses while one is still routing, since that one restores when you stop it. |
| `proxy relay` | Host only the loopback relay; blocks until killed. No CA, no system-proxy change. For containers, servers and CI. Sign in first. An alternative to `enable`, not a step before it. |
| `proxy domains` | List routable provider domains and whether each is on. |
| `proxy domain <slug> on\|off` | Turn routing on or off for one provider, e.g. `proxy domain anthropic on`. |
| `proxy trust-ca` | Trust the local CA without enabling the proxy. |
| `proxy untrust-ca` | Remove the CA's trust. The proxy must be off. |

`trust-ca` and `untrust-ca` take `--system-trust` to install or remove the CA
machine-wide with no dialog, for hosts nobody is sitting at. It needs root
(macOS/Linux) or an elevated prompt (Windows) and fails fast rather than
asking. It makes the CA a trusted root for **every user** on the machine; see
"Headless CA trust" in the README.

## Things to know

- A process that is already routing (usually the app) keeps the key it
  started with: after `login` it uses the old key until routing restarts
  there, and after `logout` it keeps using the deleted key until routing
  stops there. The CLI prints a note in both cases.

## Environment variables

| Variable | Used by |
| --- | --- |
| `GATE_BASE_URL` | Default for `login --base-url`. |
