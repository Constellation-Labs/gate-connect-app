# UI e2e

Browser-level tests for the popover. They load the **real** frontend bundle in
a real browser and drive it the way a person does: real `App.tsx`
orchestration, real `src/lib/api.ts`, real CSS at 360x520, real focus and
keyboard behaviour.

The one thing that isn't real is the Tauri process. `install.ts` puts a
stateful fake behind `window.__TAURI_INTERNALS__.invoke`, answering the same
command names the Rust side does and mutating its own state as commands land,
so `provider_enable` really does change what the next `list_tools` returns.

## What this layer is for

The vitest suites in `src/` render one screen against fixed props with
`vi.mock("../lib/api")`. That leaves a seam nothing covered: **App deciding
what to invoke, what to re-read afterwards, and which screen to show for the
result.** Everything here lives on that seam - boot resolution, screen
handoffs, backend events arriving unprompted, a command rejecting.

## What it is not for

- **The Rust backend.** Covered by the workspace crates' own integration
  tests and, against real AI CLIs and a real relay, by `ci/e2e/run.sh`.
- **Cross-browser rendering.** One Chromium project. The app ships in one
  webview per platform and none of them is Chromium; a matrix here would be
  coverage of something we don't ship. `platform.spec.ts` covers what the app
  *does* per platform (chrome, nouns, which controls exist) by faking
  `app_platform`; how any of it paints in WKWebView, WebView2 or WebKitGTK is
  not covered by anything.
- **Re-testing one screen's branches.** If it can be asserted by rendering a
  single component with props, it belongs in a `.test.tsx` next to that
  component - those run in a second and these don't.

## Running

```sh
pnpm test:e2e            # headless
pnpm test:e2e:ui         # Playwright UI mode, for writing them
pnpm typecheck:e2e       # these files sit outside tsconfig.json's `include`
```

The config starts Vite on port 5599 itself (not 5173, so a running `pnpm app`
is left alone) and reuses an already-running one outside CI.

## Writing one

`boot(patch)` merges `patch` one level deep into `defaultState()` - a
signed-in OAuth account, an org picked, routing off, three installed tools -
and returns an `App` handle:

```ts
test("turning routing on trusts the CA in the same step", async ({ boot }) => {
  const app = await boot({ runningAgents: 0 });

  await app.routingSwitch.click();

  expect((await app.state()).proxy.ca_trusted).toBe(true);
});
```

- `app.state()` - the backend's state after whatever the UI just did.
- `app.calls()` / `app.lastCall(cmd)` - what the frontend invoked, with args.
- `app.emit(event, payload)` - push a backend event (`quit-requested`,
  `proxy-state-changed`, `tauri://focus`) the way Rust does.
- `app.patch(...)` - change backend state out of band, for what moves while
  the popover is closed: a token expiring, the CLI enabling routing.
- `failures: { command: "message" }` in the boot patch makes a command reject,
  which is how the error paths get tested.

Assert on **roles and the copy a user reads**, not on classes. When a string
appears in both a pill and an `sr-only` description, `.first()` is the honest
fix - both are meant to be there.

An unknown `plugin:*` command resolves to null (the app touches window and
opener plumbing incidentally); an unknown *app* command rejects loudly, so a
renamed Tauri command fails here instead of passing quietly.
