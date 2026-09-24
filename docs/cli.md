# `gate-connect` CLI reference

The CLI shares `gate-connect-core` with the Gate Connect desktop app, so anything it does
the app does too (same registry, same config edits, same secret store). Run
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
| `connect <tool>` | Edit the tool's config to route through Gate. The tool keeps its own provider login; Gate forwards whatever it sends. Fails unless the proxy is running (see below); it does not start it. |
| `disconnect <tool>` | Restore the tool's prior configuration. |

## Built-in proxy (macOS / Windows / Linux)

Routes apps with no config file (Claude Desktop, ChatGPT, ...) and CLI tools
through Gate. Only enabled provider domains are intercepted; every other host
is tunnelled untouched.

| Command | What it does |
| --- | --- |
| `proxy status` | Running or not, port, CA trust, provider domains. |
| `proxy enable` | Trust the local CA and point the system proxy at the loopback engine. May prompt for elevation. On macOS and Windows the engine lives in this process, so the command stays in the foreground until Ctrl-C / SIGTERM; then it puts tools whose config names its relay back on their own settings (they reconnect on the next enable), stops routing and restores the prior proxy state. It refuses if the app is already routing. |
| `proxy enable --foreground` | Linux only. There the engine runs in a background daemon and `enable` returns; `--foreground` instead stays up until Ctrl-C / SIGTERM, then stops routing and restores the prior proxy state. For a systemd unit or CI job that should own the routing lifetime. |
| `proxy disable` | Turn the proxy off and restore the prior system-proxy state. |
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

- If the app is running the proxy, a new `login` key only takes
  effect once the proxy is toggled off and on. The CLI prints a note when
  that applies.

## Environment variables

| Variable | Used by |
| --- | --- |
| `GATE_BASE_URL` | Default for `login --base-url`. |
