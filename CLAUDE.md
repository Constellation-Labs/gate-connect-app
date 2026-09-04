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
topbar; navigation is a persistent 256px sidebar; content is a pane that
scrolls independently. Window controls belong to the operating system, so
the topbar only reserves space for them. Secondary flows are centred
dialogs, not stacked panels; their width is per dialog - the file draws
480, 512, **536**, 544 and 600, and `Modal`'s `width` prop is typed to those
five. The 536 is the quit confirmations (`694:33002`, `694:33340`) and it is
measured: they are the one pair that is *not* centred, sitting where a 512
centred in 1024 would start and ending ~24px past its mirror. A scan that
filters for centred frames will report no 536 anywhere and be wrong.

Theme: **light only**. Dark mode is not on the roadmap for the first
release.

### Aesthetic Direction

**The Figma is the source of truth**, not this file and not the older
`cg/` ink system. Where the file and a local judgement disagree, the file
wins, including on copy - standing instruction, 2026-08-26. Where the file
disagrees with *itself*, **match what the frame renders** for the surface
you are building - that is what "looks like the design" means, and it is
what a designer checks against. Reach for the component set only where no
frame draws the thing. Buttons are the worked example: the `Button`
component says radius 8 on a `base.input` line, and every pane instance
draws radius 4 on a `base.border` one, so panes get 4. Dialogs draw 8, and
get 8. Say which you took and why.

**The component library lives in three canvases, not the old page.**
`113:16762` really is empty, and concluding from that the library was deleted
is wrong: it moved to **Banners** (`744:37738` - `nav/topbar`, the update and
routing banners, the alert rows), **Menus** (`744:37691` - `topnav/menu`,
`chart/tooltip`) and **Sidenav** (`408:15625` - the rail, `Switch`,
`sidebar-menu-item`, `status-label`, the logo). Look there before deciding a
thing is undrawn. The `Button` set (`685:20855`) is still on a page MCP
refuses to open, which is why the button questions below are settled from
instances.

**Where the file contradicts itself in ways no rule settles**, the open
questions are collected in `docs/figma-questions-for-design.md` rather than
decided here. Do not resolve one of those by eye; the evidence for each is in
`docs/review-figma-*.md`.

Two copy exceptions are decided and stay decided. Do not "fix" either back
to match its frame:

- **Replace API key** labels its field `New API key`. **This is not a
  deviation - it is what the file draws**, and the entry used to say
  otherwise. The dialog is drawn twice, as one dialog in two states: the
  EMPTY state (`177:74332`) labels the field `New API key`, which is exactly
  what ships, and only the FILLED state (`177:74640`) says `New device
  name` - with `sk-gw-216c63…`, a key, typed into it. The section label above
  the pair (`191:80083`, "Settings / Update device name") carries the same
  slip: the designer duplicated the rename-device section and relabelled part
  of it.
  **The "newest node wins" tiebreak points the wrong way here** - `177:74640`
  is the newer id and it is the one with the slip - so an agent applying that
  rule mechanically would "correct" the code into calling an API key field a
  device name. That is why this stays written down.
- **Disconnect Gate?** says the session ends and configs are kept, not the
  drawn sentence about the keychain, which describes Reset. Verified: the
  frame (`164:73502`) really does say "your API key is removed from the
  keychain", and step 3 of the Reset dialog (`177:73975`) says nearly the
  same words, which is the evidence it was pasted from there. This one IS a
  deviation from the drawn copy, and it stays.
  The dialog also carries a hidden alternate subtitle (`143:70624`, "Your
  next requests will use Constellation Gate PAYG credits") - deliberately
  off, and about billing rather than the keychain. Worth knowing before
  anyone rewrites this copy; not a third exception.

If you find a third of these, raise it rather than deciding it.

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
- **Ink is `base.foreground #030712`**, not `neutral-900`. The three
  foreground tokens the Figma variables name are the only ones to use:
  `base.foreground` on body and headings, `base.primary-foreground
  #f9fafb` on a filled primary button, `base.destructive-foreground
  #fef2f2` on a filled destructive one. Never `text-white` on a button.
- **The `Button` set has at least four sizes, not two.** `default` (h36,
  10/12), `sm` (h32, 8/12), **`xs`** (h24, 4/10) and **`icon`** (square).
  All carry a moulded elevation - `shadow-base-btn`, `-btn-sm`,
  `-btn-primary`, `-btn-destructive` - never a flat `shadow-base-2xs`, and a
  filled primary also takes the white/black 8% vertical gradient over
  `base.primary`.
  This file said "exactly two" for months. The set node (`685:20855`) will
  not resolve over MCP, which is why: but **Figma reports each instance's
  variant NAME**, so `744:37756` comes back as `Variant=Outline,
  State=Default, Size=xs` (`685:20937`) and the set publishes
  `height/h-6: 24`. That is how to identify a variant when the set itself is
  unreachable. `xs` is drawn on `banner/update`'s dismiss and the table View
  buttons; `banners.tsx` already matched it exactly on every property.
  **Radius follows the VARIANT, not the surface** - which is why one shared
  component kept being wrong for somebody. Measured: `icon` is 4px
  (`127:46660`, the topbar), `sm` is 8px (`694:34124`, the tray footer), `xs`
  is 4px, all three on a `base.input` line, and both icon glyphs export
  `#203DE2` `base.primary`. So `OutlineIconButton` takes its radius from the
  call site. The older pane/dialog phrasing - panes 4px on `base.border`,
  dialogs 8px on `base.input` - still describes most instances and is
  confirmed on the Overview dialogs (`694:32469/70`, `694:33509/18`) and the
  Gate-model confirmation (`130:48311/2`); it is contradicted by the Settings
  dialogs (4px on all six) and the picker footer (4px on `base.border`).
  Those two are raised with design rather than chased.
- **Row icons are `base.foreground` on the app rows**, not a grey. A 20px
  glyph at `neutral-500` beside a `#030712` label reads as disabled, and
  `AppPane`'s row frames resolve `base/card` + `base/border` + `base/foreground`
  (`683:20439`) at 36px around a 20px glyph.
  **It is a per-surface rule, not a global one.** The Overview policy and
  savings tables draw their row glyphs at `base/muted-foreground` #6b7280
  (`116:26721`), deliberately quieter than the label beside them. Resolve the
  node before applying either half of this.
- **Radii** come from `tailwind.config.ts`'s own scale, not from Tailwind's,
  which redefines Tailwind's own `sm` (2px -> **6px**), `lg` and `xl`. So
  `rounded-sm` is 6px, and reaching for it because you want 2px is a mistake
  this repo has made repeatedly.
  `rounded-md` (8px) on cards and rows, `rounded-2xl` (**16px**) on dialogs.
  The 16px is measured - every dialog frame in Settings, Overview and App
  carries it - and it replaces the 12px this file used to call locked.
  **Controls measure 4px, not 6px**, wherever the file actually draws one:
  menu rows (`744:37693`), the topbar icon button (`127:46660`), the picker's
  selected row and its "Unselect all". Measured exceptions worth knowing:
  the picker checkbox is **1.667px** (`665:19094`) so it takes `rounded-xs`;
  the Overview action pills are 2px while the feed badges are 4px, which is
  two components, not a drift; and the tray's footer icon button is 8px where
  the topbar's identical-looking one is 4px, so `OutlineIconButton` takes its
  radius from the call site.
- **Ground is `base.background #f9fafb`**, cards and chrome are white. Not
  `gray-100`: the window frames fill `#F9FAFB`, and the darker grey read as
  the single most obviously wrong thing on screen.
- **Geist + Geist Mono.** Mono is for identifiers (URLs, hosts, keys,
  model ids, install ids, versions, status pill labels, the sidebar
  eyebrow), never for body copy.
  **This one is contested and unresolved.** The model picker draws its model
  ids in Geist Medium *sans* (`665:18400`, `665:19064`), and Settings draws
  four more identifiers sans: six drawn sans, none drawn mono. The app is
  still all-mono and nothing has been flipped, because it would change a lot
  of screens on one reading of a few frames. Raised with design; until it
  comes back, keep mono and do not "fix" a call site either way.
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

6. **A figure is a measurement, or the card says it isn't.** A number on
   screen is something Gate actually measured. A counter with no reading
   behind it reads `N/A`; a section that was never read says so in words
   ("Policies couldn't be read"); a value still in flight draws a
   `Skeleton`. Never a `0` for an answer nobody gave - it is a claim about
   the user's traffic, made on the one screen they would check it on, and
   a number outranks any sentence beside it. The Figma draws only the
   no-traffic case (`228:89333`: `0` / `0` / `N/A` for an org that sent
   nothing); the failure and loading states are inferred from this rule
   rather than from a frame. `lib/activity.ts` keeps the distinction in
   the model - `null` is no reading, `0` is a reading - so a surface that
   flattens the two is doing it on purpose and had better be right.

## Implementation notes that bite

- **Figma `letterSpacing` is a PERCENTAGE, not pixels.** `heading/20` at -1%
  is -0.2px, `heading/16` at -1% is -0.16px, `mono/eyebrow` at 8% is 0.96px
  at 12px and 1.12px at 14px. Convert before comparing, and add the step to
  `letterSpacing` under the Figma variable's own name rather than reusing a
  numerically equal one - `label-12` and `button-xs` are both -0.12px and are
  deliberately separate. Note the file uses `label/14` at BOTH 0% and -1%.
- **A text node's HEIGHT tells you its size**, which is faster than resolving
  every node: 36px tall is 32px type, 28 is 24, 24 is 16, 20 is 14, 16 is 12.
- **There are two layers of colour variable, so a `text-neutral-*` is not
  automatically wrong.** The file exposes the raw `tailwind colors/*` ramps
  (`neutral/500` #737373, `neutral/600` #525252, `green/600`, `red/400` …)
  *and* the `base/*` semantics. Some nodes genuinely use the raw ramp:
  `116:30213`, the app header's status line, really does resolve
  `neutral/500` for the qualifier after the dash. Others do not:
  `116:30215`, the On label beside it, resolves `base/foreground`, so its
  `neutral-600` was wrong. **Resolve the node before changing an ink**, and
  where the variable set is ambiguous - two neutrals on one node - render it
  and sample the pixels. That settled the wordmark ("Connect" is
  `neutral/600`, sampled (82,82,82)) and the feed badge radius.
- **The window minimum is enforced twice, and has to be.** `minWidth`/
  `minHeight` in `tauri.conf.json` is what macOS, Windows and X11 honour.
  Wayland ignores it: tao asks with `gtk_window_set_geometry_hints` and
  `GDK_HINT_MIN_SIZE`, which is the X11 `WM_NORMAL_HINTS` mechanism, and GTK
  never translates it into `xdg_toplevel.set_min_size`. So `lib.rs` clamps on
  `Resized` as well, Linux only. Change `MAIN_MIN_SIZE` and the config
  together.
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

## Running the app locally

Use **`pnpm app:local`**, not `pnpm app`.

`tauri dev` rebuilds the Rust binary on every change, and macOS scopes a
keychain item's ACL to the binary that created it. So each rebuild is an
executable the login keychain has never seen, and the OS re-prompts for
permission to read the stored Gate key. "Always Allow" fixes exactly one
build; the next `cargo run` invalidates it, and answering needs the login
password.

Dismissing the prompt is worse than answering it. An unreadable key reads
as an unusable account, and the app then falls back to the built-in
default gateway and **rewrites `account.json`** on the way. A developer
pointed at staging who denies the prompt is silently moved back to the
production URL, with nothing on screen saying so.

`pnpm app:local` sets `GATE_CONNECT_TEST_SECRETS`, the seam in
`crates/core/src/keychain.rs` that backs secrets with files instead of the
OS store, so the keychain is never touched and the prompt cannot appear.
It also pins `VITE_GATE_DEFAULT_BASE_URL`, so the fallback above lands on
staging rather than production. It defaults to staging; pass a URL to
point somewhere else:

```
pnpm app:local                       # staging
pnpm app:local http://localhost:3000 # a gateway you are running yourself
```

The key is entered once and persists across rebuilds. It is a plaintext
file under `~/.gate-connect-dev/secrets`, outside the repo so no `git add`
can reach it, which is a real reduction in protection and the reason this
is a dev script rather than the default. **Use a staging key here, never a
production one.** `GATE_CONNECT_TEST_SECRETS` is unset in shipped builds,
so released copies always use the real keychain.

## Migration status

`plans/new-app-ui-figma.md` is the working plan: what is built, what is
still open, and the values sampled from Figma. Read it before starting UI
work.

Both shells exist in one build, and **the new window UI is the default**.
`gcNewUi(false)` in devtools returns to the popover, or `VITE_NEW_UI=0` at
build time; see `src/lib/newUi.ts`, which is the authority. This file said
the popover was still the shipping default until 2026-09-01, which was
wrong and is the kind of error that decides whether a change is treated as
user-facing: anything landing in the new shell ships to production users.

The new shell's routing actions are still inert until they are wired
through drift review and certificate trust, which is why the popover
remains reachable at all.

NOTE: never use "—"
