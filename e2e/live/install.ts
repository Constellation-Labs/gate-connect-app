/**
 * Installs a *live* Tauri IPC into the page: every `invoke` goes to the real
 * Rust command table hosted by `src-tauri/examples/ui-harness.rs`.
 *
 * The sibling of `e2e/install.ts`, and deliberately the same seam. That file
 * answers `window.__TAURI_INTERNALS__.invoke` from a fake in the page; this one
 * forwards the identical call over loopback to the real command, so the app
 * above it cannot tell the difference and needs no test build.
 *
 * Serialized by Playwright and evaluated in the page, so the body must be
 * entirely self-contained - no imports, no module-scope references. Everything
 * it needs arrives in `opts`.
 *
 * Two things stay in the page rather than crossing to Rust:
 *
 *   - **`plugin:event|*`.** Tauri's core event plugin exists on the harness, but
 *     its `emit` would deliver to a mock webview that runs no JavaScript. So
 *     listeners live here and the backend's emits arrive by polling `/events`.
 *   - **`plugin:updater|check`.** There is no updater on the harness, and an
 *     update panel is not what any of these tests are about.
 *
 * Every other `plugin:*` call resolves to null, matching `e2e/install.ts`: the
 * window/opener/process surfaces the app touches incidentally are not what a
 * routing test is asserting on. An unknown *app* command is a different matter
 * and reaches the harness, which answers with the same "unknown command"
 * rejection the real backend gives.
 */
export interface LiveOptions {
  /** Base URL of the harness, e.g. `http://127.0.0.1:5610`. */
  harness: string;
  /** Required on every harness route; see `playwright.live.config.ts`. */
  token: string;
  /** `getCurrentWindow().label`, which `main.tsx` reads to pick its shell. */
  windowLabel: string;
  /**
   * Event-log offset to start from.
   *
   * The harness keeps every event since it started, and a page reload would
   * otherwise replay the whole history at a freshly mounted app - re-opening
   * dialogs the previous page had already answered. The fixture reads the
   * current offset before navigating, so each page sees only what the backend
   * emits from its own boot onwards.
   *
   * This value seeds the FIRST page of a browser context only. An init script
   * runs again on every navigation with the same baked number, so a
   * `page.reload()` would replay everything since the original boot - the very
   * problem this option exists to avoid. The live offset therefore lives in
   * `sessionStorage`, which survives a reload and dies with the context.
   */
  since: number;
}

/** The handle each spec drives the live backend through, from inside the page. */
export interface LiveHandle {
  /** Every command the frontend has invoked, in order. */
  calls: { cmd: string; args: Record<string, unknown> }[];
  /** Push an event at the frontend, for what the harness cannot emit itself. */
  emit: (event: string, payload?: unknown) => void;
}

declare global {
  interface Window {
    __GATE_LIVE__: LiveHandle;
  }
}

export function installLiveTauri(opts: LiveOptions): void {
  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  const callbacks = new Map<number, (payload: unknown) => void>();
  const listeners = new Map<string, Set<number>>();
  let nextId = 1;

  function emit(event: string, payload?: unknown): void {
    const ids = listeners.get(event);
    if (!ids) return;
    for (const id of ids) {
      const cb = callbacks.get(id);
      if (cb) cb({ event, id, payload });
    }
  }

  async function call(cmd: string, args: Record<string, unknown>): Promise<unknown> {
    const res = await fetch(`${opts.harness}/invoke`, {
      method: "POST",
      headers: { "content-type": "application/json", "x-gate-harness-token": opts.token },
      body: JSON.stringify({ cmd, payload: args }),
    });
    const body = await res.json();
    // A command that returned `Err` rejects, which is the shape Tauri gives the
    // frontend and therefore the shape every error path in the app expects.
    if (body && typeof body === "object" && "err" in body) {
      throw body.err;
    }
    return (body as { ok: unknown }).ok;
  }

  const invoke = (cmd: string, args: Record<string, unknown> = {}) => {
    calls.push({ cmd, args: args ?? {} });

    if (cmd === "plugin:event|listen") {
      const { event, handler } = args as { event: string; handler: number };
      let ids = listeners.get(event);
      if (!ids) {
        ids = new Set();
        listeners.set(event, ids);
      }
      ids.add(handler);
      return Promise.resolve(handler);
    }
    if (cmd === "plugin:event|unlisten") {
      const { event, eventId } = args as { event: string; eventId: number };
      listeners.get(event)?.delete(eventId);
      callbacks.delete(eventId);
      return Promise.resolve(null);
    }
    if (cmd === "plugin:event|emit" || cmd === "plugin:event|emit_to") {
      return Promise.resolve(null);
    }
    if (cmd === "plugin:updater|check") return Promise.resolve(null);
    if (cmd.startsWith("plugin:")) return Promise.resolve(null);

    return call(cmd, args ?? {});
  };

  (window as any).__TAURI_INTERNALS__ = {
    invoke,
    transformCallback(callback: (payload: unknown) => void) {
      const id = nextId++;
      callbacks.set(id, callback);
      return id;
    },
    unregisterCallback(id: number) {
      callbacks.delete(id);
    },
    convertFileSrc: (path: string) => path,
    metadata: {
      currentWindow: { label: opts.windowLabel },
      currentWebview: { windowLabel: opts.windowLabel, label: opts.windowLabel },
    },
  };

  (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(event: string, eventId: number) {
      listeners.get(event)?.delete(eventId);
      callbacks.delete(eventId);
    },
  };

  (window as any).__GATE_LIVE__ = { calls, emit } satisfies LiveHandle;

  // Drain the harness's event log into the page. Long-polls, so an idle run
  // costs one request a second rather than a spin; it dies with the page.
  const OFFSET_KEY = "gc.live.eventOffset";
  const readOffset = () => {
    try {
      const saved = window.sessionStorage.getItem(OFFSET_KEY);
      return saved === null ? opts.since : Number(saved);
    } catch {
      return opts.since;
    }
  };
  const saveOffset = (n: number) => {
    try {
      window.sessionStorage.setItem(OFFSET_KEY, String(n));
    } catch {
      /* noop */
    }
  };

  let since = readOffset();
  let stopped = false;
  window.addEventListener("beforeunload", () => {
    stopped = true;
  });
  void (async () => {
    while (!stopped) {
      try {
        const res = await fetch(`${opts.harness}/events?since=${since}`, {
          headers: { "x-gate-harness-token": opts.token },
        });
        const body = (await res.json()) as {
          next: number;
          events: { event: string; payload: unknown }[];
        };
        // Deliver first, then advance. The other order dropped the rest of a
        // batch whenever a listener threw, and swallowed the error with it:
        // `since` had already moved past events nobody received.
        for (const e of body.events) {
          try {
            emit(e.event, e.payload);
          } catch (err) {
            // A handler that throws is the app's bug, not the bridge's. Say so
            // where a spec can see it rather than losing it in this catch.
            console.error(`[live] listener for ${e.event} threw`, err);
          }
        }
        since = body.next;
        saveOffset(since);
      } catch {
        // The harness going away ends the run; sleeping keeps this from
        // spinning against a dead port while Playwright tears the page down.
        await new Promise((r) => setTimeout(r, 200));
      }
    }
  })();
}
