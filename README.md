# Gate Connect (prototype)

Smallest end-to-end slice of the [Gate Connect PRD](https://www.notion.so/Gate-Connect-PRD-35da94bd4b4f81afa883eebceab0f367)
— enough to point **Claude Desktop's Cowork** at a custom gateway and
verify the round trip.

## Scope

- **Surfaces:** Tauri 2 menu-bar app (primary) and a `gate-connect` CLI. Both
  invoke the same `gate-connect-core` crate; nothing is implemented in only
  one place. The app lives in the macOS menu bar — no dock icon. Left-click
  the tray icon to toggle a popover anchored underneath; right-click for the
  menu.
- **Platforms:** Cowork ships on macOS and Windows; Claude Code, Codex, and
  OpenCode target macOS, Linux, and Windows.
- **Integrations:** Cowork (Claude Desktop 3P / `inferenceProvider: gateway`),
  Claude Code, Codex, and OpenCode.
- **Auth:** mocked. The UI takes the gateway base URL and API key directly.
  Real OAuth/PKCE against the Gate AI identity service comes later.

## Architecture

Matches the PRD: a `gate-connect-core` Rust crate that holds the registry,
native primitives, and per-integration logic. The Tauri shell and the CLI
both depend on it.

```
apps/connect/
├── src/                React + Tailwind frontend (Vite)
├── src-tauri/          Tauri 2 Rust shell (commands → core)
└── crates/
    ├── core/           gate-connect-core: registry, keychain, plist, Cowork
    └── cli/            gate-connect: thin CLI binary
```

## How the Cowork integration works

`connect` runs three writes:

1. **Keychain.** Stores the gateway API key as a generic password under
   `ai.constellation.gate-connect.cowork.gateway-api-key`. The long-lived key
   never lands in any file on disk.
2. **Credential helper.** Writes `~/Library/Application Support/Gate Connect/cowork-credential-helper.sh`
   (mode 0755). The script calls `security find-generic-password ... -w` and
   prints the key on stdout — exactly what Cowork's `inferenceCredentialHelper`
   contract expects.
3. **Managed plist.** Writes `/Library/Managed Preferences/<user>/com.anthropic.claudefordesktop.plist`
   with:
   - `inferenceProvider = "gateway"`
   - `inferenceGatewayBaseUrl = <your URL>`
   - `inferenceGatewayAuthScheme = "bearer"`
   - `inferenceCredentialHelper = <path to helper>`
   - `inferenceCredentialHelperTtlSec = 3000`
   - `deploymentOrganizationUuid = <cached install id>`

   The path is owned by `root:wheel`. From the GUI the user gets a native
   macOS authentication panel (via `osascript … with administrator privileges`).
   From the CLI they get the standard `sudo` prompt.

`disconnect` reverses all three: removes the plist, removes the helper,
deletes the keychain entry.

`status` checks all three and reports `not installed`, `detected`,
`connected`, or `drifted: <reason>`.

## Run the app

From this directory:

```bash
pnpm install        # one-time (runs from any package in the monorepo)
pnpm app            # tauri dev — opens the Gate Connect window
```

`pnpm app` boots Vite on :5173, compiles the Rust shell, and starts the app.
There is no dock icon — look for the Gate Connect glyph in the macOS menu bar
(top-right of the screen).

### Use it

1. Make sure Claude Desktop is installed at `/Applications/Claude.app`. Don't
   sign in yet.
2. Click the Gate Connect icon in the menu bar. A popover slides down.
3. Click **Connect** next to Cowork.
4. Enter your gateway base URL and API key. The macOS auth panel will pop
   asking for your password — that's the managed-prefs write.
5. Cmd+Q Claude Desktop fully and relaunch.
6. The sign-in screen should offer to skip Anthropic authentication. Take it.
7. Verify via **Help → Troubleshooting → Copy Managed Configuration Report**
   that `inferenceProvider = gateway` is in effect.

The popover dismisses on focus loss (click outside) or by clicking the tray
icon again. To revert, reopen the popover and click **Disconnect**, then
fully restart Claude Desktop again. Right-click the tray icon for **Quit**.

### CLI parity

For power users / scripting:

```bash
cargo run --release --bin gate-connect -- list
cargo run --release --bin gate-connect -- connect cowork \
  --base-url https://your-gateway.example.com \
  --api-key sk-...
cargo run --release --bin gate-connect -- disconnect cowork
```

Same registry, same code path.

## Verifying nothing leaked

```bash
# Plist should not contain the key — only a path to the helper script
sudo plutil -p /Library/Managed\ Preferences/$USER/com.anthropic.claudefordesktop.plist

# Helper script contains no secret either, just a `security` invocation
cat ~/Library/Application\ Support/Gate\ Connect/cowork-credential-helper.sh

# Key lives here, accessible only to the user
security find-generic-password \
  -s ai.constellation.gate-connect.cowork.gateway-api-key -a "$USER" -w
```

## Known gaps vs. the PRD

- No PKCE/OAuth flow. There's no browser-based authorization; instead Gate
  delegates to the existing Claude Code session, reading its keychain entry
  via the `claude_session_delegate.rs` session-delegate module. An API key can also be
  provided directly through the connect form.
- No code signing, notarization, or auto-update. `bundle.active` is `false`;
  this is a dev build only.
- No signed registry pulled from the Gate API; registry is hard-coded with
  entries for Cowork, Claude Code, Codex, and OpenCode.
- No audit events emitted to the gateway.
- No drift watcher — status refreshes on action, not on a schedule.
- macOS is the most complete target; Windows and Linux support varies by
  integration and is still being hardened.
- Telemetry is not wired up.
- Placeholder app icon (`src-tauri/icons/icon.png` is a 64×64 generated PNG).
  Bundle/build requires a full icon set (icns, ico, multiple sizes).

These are the next things to build; the core mechanism is now testable end
to end through the GUI.
