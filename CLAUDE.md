# Gate Connect - Claude Code instructions

Scoped to this app (`gate/apps/connect/`). For repo-wide Gate context,
see `gate/CLAUDE.md` (if present).

## Design Context

### Users

Open developer public - anyone with a Constellation Gate gateway URL who
wants to point their AI dev tools (Cowork/Claude Desktop, Codex, OpenCode)
at it once and stop thinking about credentials. They open the app at the
moment they're already mid-task: setting up a new machine, swapping keys,
debugging why a tool isn't connecting. Context is low-attention,
high-stakes (their actual API traffic is about to flow through this thing).

Job to be done: "Tell my dev tools to use this gateway, and reassure me
that the credential isn't sitting in a config file somewhere."

macOS, Windows and Linux. Not a menubar popover: see Surface below.

### Brand Personality

Pragmatic · friendly · approachable. Tailscale / 1Password energy - the
product that mediates something sensitive (your AI traffic, your keys)
and makes you feel like grown-ups are running the kitchen. Warm at the
edges, serious in the middle. Not stiff, not playful, not techbro
brutalist.

Emotional goal: **reassuring gatekeeper**. The user should leave the
window thinking "good, that's handled" - not "I hope I configured that
right."

### Surface

**A 1024x720 desktop window**, not a 360px popover. Chrome is a 48px
topbar; navigation is a persistent 250px sidebar; content is a pane that
scrolls independently. Window controls belong to the operating system, so
the topbar only reserves space for them. Secondary flows are centred
dialogs at 600px, not stacked panels.

Theme: **light only**. Dark mode is not on the roadmap for the first
release.

### Aesthetic Direction

**The Figma is the source of truth**, not this file and not the older
`cg/` ink system:
`https://www.figma.com/design/9FrccCojXy0f8QD8Wm5Lln/Gate-Connect`

It is shadcn-flavoured on Tailwind's default palette. Tokens live in
`tailwind.config.ts` under `base.*`, named to mirror the Figma variables
one-to-one so any value can be traced back without guessing.

- **Primary is blue-ribbon `#203de2`** (`base.primary`). It backs switches,
  active nav, links and filled primary buttons. The "Gate" half of the
  wordmark is `blue-ribbon-800`.
- **Borders are real 1px lines.** `base.border #e5e7eb` on cards and
  dividers, `base.input #d1d5db` on controls. Cards additionally carry
  `shadow-base-sm`.
- **Radii: 4px** (`rounded-base`) on controls, nav items and pills, **8px**
  (`rounded-lg`) on cards and rows, **12px** (`rounded-xl`) on dialogs.
- **Ground is `gray-100`**, cards and chrome are white.
- **Geist + Geist Mono.** Mono is for identifiers (URLs, hosts, keys,
  model ids, install ids, versions, status pill labels, the sidebar
  eyebrow), never for body copy.
- Destructive actions are filled `red-600`. There are only ever a couple
  per screen; if a third appears, question it.

References (positive): shadcn/ui, 1Password 8, Tailscale macOS app,
Linear's desktop polish, Raycast's restraint.

Anti-references (locked):
- **No generic AI aesthetic** - no purple/blue gradients, no neon
  accents on dark, no sparkles, no chatbot UIs.
- **No dev-tool brutalism** - no all-mono UI, no ASCII chrome, no
  all-caps everything, no terminal cosplay.

The Overview pane is a dashboard and may look like one. That is a
deliberate reversal of an earlier "no dashboard" rule.

### Design Principles

1. **Credentials are the product.** Every screen should make the user
   feel where their key lives (keychain), where it doesn't (no config
   files), and what's being sent over the wire. Reassurance comes from
   transparency, not from hiding the mechanism.

2. **Observed state and intent are two different things.** A row's status
   line says what is *happening*; its switch says what the user *asked
   for*. They diverge legitimately: a tool can be switched on and not
   routing because the master is off, the certificate is untrusted, or its
   config drifted. `lib/groups.ts` documents the bug that comes from
   conflating them - the switch renders off, and clicking it turns off the
   setting the user was trying to turn on. Never drive a switch from
   observed state.

3. **Apps are nouns, routing is a verb.** The sidebar lists apps; each row
   pairs a status line with a switch. Status vocabulary is
   Protected / Not protected / Config drifted / Not routed, coloured green
   or amber, with any qualifier ("2m ago", "Off") in grey after a dash.

4. **Mono earns its place.** Geist Mono only where identity or precision
   matters. Body copy and labels are sans. Mono is a signal, not a vibe.

5. **Destructive things get a dialog, and the dialog defaults to safe.**
   Config replacement, disconnecting, closing running apps and resetting
   all go through a confirmation. When the primary action is destructive,
   initial focus goes to the *secondary* button - `useFocusTrap` takes an
   `initialFocus` ref for exactly this.

## Implementation notes that bite

- **Never use `bg-blue-*` or `text-blue-*`.** `tailwind.config.ts`
  redefines Tailwind's `blue` as an OKLCH ramp for the old ink system, so
  those classes render the wrong colour. Use `base.primary`,
  `blue-ribbon-*`, or the semantic `chart.*` group.
- **Font sizes go in rem, never px.** `useTextScale` scales the whole ramp
  from the root, and a px literal opts that call site out of it entirely.
  Use the `base-*` / `gc-*` `fontSize` tokens.
- **The design names shadows on Tailwind v4's scale; this repo is on
  v3.4.** Figma `shadow/sm` is v3's default `shadow`, not `shadow-sm`.
  The `base-*` shadow tokens absorb the mapping; shift any new value one
  step before use.
- **Figma draws borders inside the frame; CSS adds them outside the
  padding box.** Expect measured heights to run ~2px over the Figma
  number on bordered cards. Not worth contorting the markup for.
- **The `gc.*` palette and `gc/ui.tsx` are still live**, backing the
  popover screens until they are retired. Don't delete them, and don't
  reach for them in new UI either.

## Migration status

`plans/new-app-ui-figma.md` is the working plan: what is built, what is
still open, and the values sampled from Figma. Read it before starting UI
work.

Both shells exist in one build. `gcNewUi(true)` in devtools switches to
the new window UI and reloads; `gcNewUi(false)` returns to the popover,
which is still the shipping default. The new shell is currently
**read-only** - its switches are inert until routing actions are wired
through drift review and certificate trust.

NOTE: never use "—"
