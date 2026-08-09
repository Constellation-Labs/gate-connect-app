# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

(The app ships as a Tauri 2 menu-bar/tray app on macOS, Windows, and Linux,
but its design language is web: React + Tailwind rendering inside a
transparent popover window, not native AppKit/WinUI controls.)

## Users

Open developer public: anyone with a Constellation Gate gateway URL who wants
to point their AI dev tools (Claude Code / Cowork, Codex, OpenCode, OpenClaw,
Hermes, ChatGPT desktop) at it once and stop thinking about credentials. They
open the menubar popover mid-task and low-attention: setting up a new machine,
swapping keys, or debugging why a tool isn't connecting. Stakes are high;
their real API traffic is about to flow through this thing. No provisioned
org can be assumed; the app must self-explain without organizational
hand-holding.

## Product Purpose

Tell the user's dev tools to use their Gate gateway, and reassure them that
the credential isn't sitting in a config file somewhere. Sign in once, flip a
switch, and Gate Connect configures the tools it finds installed; apps
with no gateway setting of their own are routed through a built-in local
proxy.

Success for the first release is **sustained routing**: users keep the app
running and routing their traffic daily. Churn, silent disconnects, and tools
drifting off the gateway are the failure signals. Design decisions should
optimize for the app staying trusted and running, not just for a flashy first
run.

## Positioning

Two claims a neighboring product could not truthfully copy:

1. **Everything routable on one ledger, one honest switch each.** Config
   tools and proxy-routed apps sit side by side on the home screen, each
   with its own truthful status pill and switch; the config-vs-proxy
   mechanism stays invisible per row. (The provider grouping still exists
   in the core crate and CLI, but the UI's nouns are tools and apps.)
2. **The key never lands in a config file.** The Gate API key (`sk-gw-...`)
   lives only in the OS secret store (macOS Keychain, Windows Credential
   Manager, Linux Secret Service); config files get the base URL and an
   indirection, never the secret.

Reference energy: Tailscale / 1Password, the product that mediates something
sensitive and makes you feel like grown-ups are running the kitchen.

## Operating Context

- Lives in the menu bar / system tray with no dock icon. Left-click toggles a
  popover anchored underneath; right-click opens a menu.
- The popover is one room: 360px wide, ~520px tall. Every screen works inside
  that frame; content scrolls within the body, header and footer never.
  Secondary surfaces slide as full-popover panels, never stacked modals.
- Both the app and the `gate-connect` CLI call the same `gate-connect-core`
  Rust crate; nothing is implemented in only one place.
- Config-file tools (Claude Code, Codex, OpenCode, OpenClaw, Hermes) are
  routed by editing their own configs. Config-less apps (Claude Desktop /
  Cowork, ChatGPT desktop) route through the built-in MITM proxy with a local
  CA.
- On Linux the proxy engine outlives the GUI as a detached helper daemon, so
  quitting the app does not break CLI tools mid-flight.
- Auth: OAuth (Cognito) sign-in with org selection is the primary path; a
  pasted API key is the alternative.

## Capabilities and Constraints

- Platforms: macOS (primary design target), Windows, Linux.
- The gateway, not this app, is what inspects traffic: prompt-injection
  attempts stopped, sensitive values redacted, compression trimming token
  spend. The onboarding tour may claim these because they are real Gate
  capabilities; the popover does not evidence them yet. Surfacing gate
  status in the app is planned for a future iteration, and is the honest
  home for that proof.
- Gate Connect can close running tools and apps; it cannot start them.
  Copy says "close", never promises a restart, and never implies the app
  will reopen anything. A routing change takes effect the next time the
  user opens the tool or app themselves.
- The onboarding tour's "Do not show this intro again" checkbox starts
  clear: opting out of the intro is a choice the user makes, not a default
  they discover afterwards.
- Routable surfaces: config tools (Claude Code, Codex, OpenCode, OpenClaw,
  Hermes) and proxy-routed app domains (Anthropic for Claude Desktop /
  Cowork, OpenAI, OpenRouter). Provider-level switches live in the CLI and
  core crate, not the popover.
- The UI must surface routing status truthfully; status pills are system
  state, not marketing copy.
- Quitting while CLI tools still route through Gate warns the user
  (macOS/Windows); routing changes take effect without breaking running
  agents where possible.
- Theme: light only. Dark mode is not on the first-release roadmap.
- Frontend stack: React 18 + Tailwind 3 + Vite inside Tauri 2; tests via
  Vitest.

## Brand Commitments

- Name: Gate Connect, part of Constellation Gate ("Constellation Gate AI"
  in auth surfaces). Brand indigo is reserved for the logo glyph only.
- Visual system: the locked `cg/` design system
  (`gate/packages/frontend-ui/src/cg/tokens.css`). Ink-on-paper, OKLCH
  neutral ink ramp, shadow-as-border, 12px modal radius, 4px grid,
  Geist + Geist Mono (mono only for identifiers, URLs, keys, status pills).
- Personality: pragmatic, friendly, approachable; reassuring gatekeeper.
  Warm at the edges, serious in the middle.
- Anti-references (locked): no generic AI aesthetic (purple/blue gradients,
  neon, sparkles, chatbot UI), no enterprise SaaS dashboard look, no
  dev-tool brutalism (all-mono UI, ASCII chrome, all-caps).
- Copy rule: never use the em dash character.

## Evidence on Hand

- Working product: real screens in `src/screens/` (Home, Onboarding,
  FirstRun, OrgPicker, Settings, ToolDetail, Success) and components in
  `src/components/`.
- PRD lives in Notion (linked from README.md).
- No testimonials, case studies, benchmarks, or press on hand; future
  surfaces must not fabricate any.

## Product Principles

1. **Credentials are the product.** Every screen should make the user feel
   where the key lives (keychain), where it doesn't (no config files), and
   what's being sent. Reassurance through transparency, not hiding the
   mechanism.
2. **Tools are nouns, status is a verb.** The integrations list is home; each
   row's status pill is the most important pixel on screen.
3. **Optimize for staying, not starting.** Sustained daily routing is the
   success metric; prefer designs that keep long-running state legible and
   trustworthy over ones that only demo well on first run.
4. **One room, no stacks.** Everything fits the 360px popover; secondary
   surfaces slide as full panels, never nested dialogs.
5. **Mono earns its place.** Precision typography is a signal for identity
   and system state, never a vibe.

## Accessibility & Inclusion

Target WCAG 2.1 AA: contrast ratios and full keyboard operability inside the
popover. Light theme only for the first release, so AA must be met on the
white popover surface without relying on a dark-mode fallback.

Text resizes to 200% (SC 1.4.4) through the app's own five-step control in
Settings, not through the OS or a browser zoom. A fixed, non-resizable
menubar window exposes no other route, so the mechanism has to be ours: the
type ramp is expressed in rem and scaled from one root variable. Layouts that
must change at large text key off the type size in `em`, since a single fixed
window has no width breakpoints to key off instead.
