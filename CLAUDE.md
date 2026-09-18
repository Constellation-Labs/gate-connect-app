# Gate Connect - Claude Code instructions

Scoped to this app (`gate/apps/connect/`). For repo-wide Gate context,
see `gate/CLAUDE.md` (if present).

## Design Context

### Users

Open developer public - anyone with a Constellation Gate gateway URL who
wants to point their AI dev tools (Cowork/Claude Desktop, Codex, OpenCode)
at it once and stop thinking about credentials. They open the macOS
menubar popover at the moment they're already mid-task: setting up a new
machine, swapping keys, debugging why a tool isn't connecting. Context
is low-attention, high-stakes (their actual API traffic is about to flow
through this thing).

Job to be done: "Tell my dev tools to use this gateway, and reassure me
that the credential isn't sitting in a config file somewhere."

### Brand Personality

Pragmatic · friendly · approachable. Tailscale / 1Password energy - the
product that mediates something sensitive (your AI traffic, your keys)
and makes you feel like grown-ups are running the kitchen. Warm at the
edges, serious in the middle. Not stiff, not playful, not techbro
brutalist.

Emotional goal: **reassuring gatekeeper**. The user should leave the
popover thinking "good, that's handled" - not "I hope I configured that
right."

### Aesthetic Direction

Light surface, ink-on-paper. The `cg/` design system
(`gate/packages/frontend-ui/src/cg/tokens.css`) is already locked and
this app aligns to it:

- **Primary is ink-900, never blue.** Inline links use ink + ink-200
  decoration. Brand indigo is reserved for the logo glyph only.
- **Shadow-as-border** on all elevated surfaces. No 1px solid borders
  on cards or popovers.
- **OKLCH neutral ink ramp** (chroma 0) - never gray-on-blue, never
  pure black/white.
- **Geist + Geist Mono.** Mono is for identifiers (URLs, hosts, keys,
  status pill labels, tabular numerics), never for body copy.
- **12px modal radius LOCKED**, 4px grid, asymmetric spacing where
  the rhythm calls for it.
- **Canvas #ecece7** is the brand background - but this app lives on
  white (popover surface) because it's a small floating window, not
  a page on canvas.
- **Tauri transparent window** with 12px-rounded NSWindow corners (via
  CALayer.cornerRadius on the content view's layer in
  `src-tauri/src/lib.rs`). The white card fills the window edge-to-edge.

References (positive): 1Password 8, Tailscale macOS app, Linear's
desktop polish, Raycast's restraint.

Anti-references (locked):
- **No generic AI aesthetic** - no purple/blue gradients, no neon
  accents on dark, no sparkles, no chatbot UIs.
- **No enterprise SaaS dashboard look** - no heavy cards, no bright
  blue CTAs, no Material/Bootstrap defaults.
- **No dev-tool brutalism** - no all-mono UI, no ASCII chrome, no
  all-caps everything, no terminal cosplay.

Theme: **light only**. Dark mode is not on the roadmap for the first
release.

### Design Principles

1. **Credentials are the product.** Every screen should make the user
   feel where their key lives (keychain), where it doesn't (no config
   files), and what's being sent over the wire. Reassurance comes from
   transparency, not from hiding the mechanism.

2. **Tools are nouns, status is a verb.** The list of integrations is
   the home view. Each row is one tool; its status pill is the single
   most important pixel on the screen. Mono lowercase status labels
   read as system-state, not as marketing copy.

3. **Mono earns its place.** Use Geist Mono only where identity or
   precision matters - URLs, hosts, API key placeholders, status pills,
   tabular counts. Body copy and labels are sans. Mono is a signal,
   not a vibe.

4. **Shadow-as-border, ink-on-paper.** Cards, inputs, and popovers
   carry weight through layered inset shadows (the cg `--shadow-border`
   stack), not through solid 1px lines. Surfaces are white sitting on
   white-with-shadow - paper, not sheet metal.

5. **The popover is one room.** It's 360px wide and ~520px tall on a
   typical screen. Every screen must work inside that frame without
   scrolling chrome - content scrolls within the body, header and
   footer never. No modals stacked on modals; secondary surfaces (tool
   detail, account form, migrate form) slide as full-popover panels and
   animate in/out, not as nested dialogs.

## Implementation notes that bite

- **`pnpm app:local` cannot see a keychain bug, by construction.**
  `GATE_CONNECT_TEST_SECRETS` replaces the OS secret store with files, so every
  code path that talks to the real Keychain / Credential Manager / Secret
  Service is skipped. A libdbus stack smash in the Secret Service client
  aborted `pnpm app` on most Linux launches while `pnpm app:local` stayed clean
  for weeks. CI cannot see it either: every Rust test uses the in-memory
  backend or the file seam, and `ci/e2e/run.sh` exports the seam too. Anything
  touching `keyring` or `keychain.rs` has to be checked with `pnpm app` on each
  OS, and nothing green proves otherwise.

- **Never read a secret on a timer.** On Linux every `keychain::get` opens a
  fresh D-Bus connection and Secret Service session, and `gnome-keyring-daemon`
  never frees what a session leaves behind. Measured on 46.1: **~7.85 KB per
  session, and zero for a thousand reads or three hundred writes over one
  session that stays open** (16 B and 55 B per op, which is noise). The unit is
  the *session*, not the connection and not the operation - reusing a
  connection while opening a session per call still leaks ~4.7 KB a call, and
  closing the session politely still leaks ~3.4 KB, so this is an upstream bug
  rather than a client mistake. Two things therefore look like fixes and are
  not: swapping the encrypted session for a plain one removes about a quarter
  of it and puts the secret on the bus in cleartext, and nothing on the client
  side helps, because the client is already disconnecting cleanly. No fix
  upstream through gnome-keyring 50.0.
  The app's 30s background loop used to open eight sessions per tick: six for the OAuth
  bundle (stored chunked, so one logical read is six lookups), one for the Gate
  key, one for the CA key, the last two from `manager_linux::refresh_token`
  rebuilding the whole intercept config to re-push an unchanged token. That is
  ~180 MB a day of *somebody else's* process; a dev machine running the app on
  and off for three weeks had `gnome-keyring-daemon` at 2.43 GB.
  Read through **`keychain::get_cached`**, which takes a *witness*: something
  cheap you already have that must change when the secret does. `account.json`
  witnesses the Gate key, the CA certificate witnesses its own private key -
  each written in the same breath as the secret it vouches for. The witness is
  what makes this honest across processes, which a bare cache could not be: the
  CLI can write a new key while the app runs. The OAuth bundle witnesses on
  `account.json` too, through `account::file_witness()`, because login rewrites
  that file and sign-out removes it while a token refresh leaves it alone -
  which is exactly the split between what must not be served stale and what is
  safe to miss. Work out that split before caching anything new: a cache whose
  witness cannot move on the transition that matters is worse than no cache,
  because the stale answer never expires.
  This is the bullet above in its purest form: `pnpm app:local` skips the code
  entirely, CI skips it, and it was found on a dev box only because a daemon
  had been accumulating for three weeks. It reaches production users harder
  than developers.

NOTE: never use "—"