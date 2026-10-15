import { describe, expect, it } from "vitest";
// `?raw` for the same reason config.test.ts gives: tsconfig only declares
// vite/client, which types `*?raw` and `import.meta.glob`, not node:fs.
import buildRs from "../../src-tauri/build.rs?raw";
import libRs from "../../src-tauri/src/lib.rs?raw";

/**
 * Each window can call only the application commands its `*-commands.json`
 * capability grants (AG-1138). Before build.rs named the commands, the app had
 * no ACL manifest, and Tauri let every window call every command: the tray
 * popover could save an account or switch the gateway.
 *
 * The cost of that is a new failure mode: a window that starts calling a
 * command its capability does not grant gets "not allowed by ACL" at runtime,
 * and nothing in the type system says so. These tests are the net. They walk
 * each window's import graph from its root and fail when the code reaches a
 * command the window is not granted.
 */

const sources = Object.fromEntries(
  Object.entries(
    import.meta.glob<string>(["../**/*.{ts,tsx}", "!../**/*.test.{ts,tsx}"], {
      query: "?raw",
      import: "default",
      eager: true,
    }),
  ).map(([key, src]) => [srcPath("lib/x.ts", key), src]),
);

const capabilities = Object.values(
  import.meta.glob<string>("../../src-tauri/capabilities/*-commands.json", {
    query: "?raw",
    import: "default",
    eager: true,
  }),
).map((raw) => JSON.parse(raw) as { identifier: string; windows: string[]; permissions: string[] });

/** A specifier resolved against the importing file, as a path under `src/`. */
function srcPath(from: string, spec: string): string {
  const parts = from.split("/").slice(0, -1);
  for (const seg of spec.split("/")) {
    if (seg === "..") parts.pop();
    else if (seg !== ".") parts.push(seg);
  }
  return parts.join("/");
}

function resolve(from: string, spec: string): string | null {
  if (!spec.startsWith(".")) return null;
  const base = srcPath(from, spec);
  return (
    [base, `${base}.ts`, `${base}.tsx`, `${base}/index.ts`, `${base}/index.tsx`].find(
      (p) => p in sources,
    ) ?? null
  );
}

const INVOKE = /\binvoke(?:<[^()]*?>)?\(\s*"([a-z_]+)"/g;
const literals = (src: string) => [...src.matchAll(INVOKE)].map((m) => m[1]);
const stripComments = (src: string) => src.replace(/\/\/.*$|\/\*[\s\S]*?\*\//gm, "");

const API = "lib/api.ts";

/** api.ts's exports, each with the commands it invokes, directly or through
 * another export it calls. */
const apiExports = (() => {
  const blocks = new Map<string, string>();
  for (const part of sources[API].split(/^(?=export )/m)) {
    const name = part.match(/^export\s+(?:const|async function|function)\s+(\w+)/)?.[1];
    if (name) blocks.set(name, part);
  }
  const commands = (name: string, seen = new Set<string>()): string[] => {
    const block = blocks.get(name);
    if (!block || seen.has(name)) return [];
    seen.add(name);
    const body = stripComments(block.replace(/^export\s+\S+\s+\w+/, ""));
    const callees = [...blocks.keys()].filter((o) => new RegExp(`\\b${o}\\(`).test(body));
    return [...literals(block), ...callees.flatMap((o) => commands(o, seen))];
  };
  return (name: string) => commands(name);
})();

const IMPORT = /(?:import|export)\s+(type\s+)?([\s\S]*?)\s+from\s+"([^"]+)"|import\s+"([^"]+)"/g;

/** Every command reachable from `roots`, by file for ordinary modules and by
 * imported name for api.ts. The file-level part over-approximates: importing
 * one helper from a module counts every command that module reaches. */
function reach(roots: string[], skip: string[] = []): Set<string> {
  const out = new Set<string>();
  const seen = new Set<string>(skip);
  const queue = [...roots];
  while (queue.length) {
    const file = queue.pop()!;
    if (seen.has(file)) continue;
    seen.add(file);
    const src = sources[file];
    literals(src).forEach((c) => out.add(c));
    for (const m of src.matchAll(IMPORT)) {
      const [, typeOnly, clause = "", spec, bare] = m;
      const target = resolve(file, spec ?? bare);
      if (!target || typeOnly) continue;
      if (target !== API) {
        queue.push(target);
        continue;
      }
      const ns = clause.match(/\*\s+as\s+(\w+)/)?.[1];
      const names = ns
        ? [...src.matchAll(new RegExp(`\\b${ns}\\.(\\w+)`, "g"))].map((u) => u[1])
        : (clause.match(/\{([\s\S]*)\}/)?.[1] ?? "")
            .split(",")
            .map((n) => n.trim())
            .filter((n) => n && !n.startsWith("type "))
            .map((n) => n.split(/\s+as\s+/)[0]);
      names.flatMap(apiExports).forEach((c) => out.add(c));
    }
  }
  return out;
}

// main.tsx is one entry for every window: its boot code (analytics, the error
// context, ErrorBoundary) runs in all three, and it picks one root by label.
const ROOTS = { main: "NewUiApp.tsx", tray: "TrayApp.tsx", onboarding: "screens/Onboarding.tsx" };
const windowReach = (label: keyof typeof ROOTS) =>
  new Set([...reach(["main.tsx"], Object.values(ROOTS)), ...reach([ROOTS[label]])]);

/** Commands the walk reaches only through a module the window imports for
 * something else. Each is checked to still be reached and still be ungranted,
 * so an entry cannot outlive its reason. */
const REACHED_NOT_CALLED: Record<keyof typeof ROOTS, Record<string, string>> = {
  main: {
    reveal_popover: "NewUiApp imports TOUR_SEEN_EVENT from screens/Onboarding",
  },
  tray: {
    activity_cached_overview: "lib/activity is imported for useInstallations",
    gate_credits: "lib/toolModels is imported for its pure helpers",
    gate_model_catalogue: "lib/toolModels is imported for its pure helpers",
    set_tool_model: "lib/toolModels is imported for its pure helpers",
    tool_model_preferences: "lib/toolModels is imported for its pure helpers",
  },
  onboarding: {
    cowork_setting_check: "analytics' Cowork check runs from the main window's routing flow",
  },
};

const granted = (label: string) =>
  new Set(
    capabilities
      .filter((c) => c.windows.includes(label))
      .flatMap((c) => c.permissions)
      .map((p) => p.replace(/^allow-/, "").replace(/-/g, "_")),
  );

const appCommands = [...buildRs.matchAll(/^\s+"([a-z_]+)",$/gm)].map((m) => m[1]);

describe("application command ACL", () => {
  it("names exactly the desktop invoke_handler's commands in build.rs", () => {
    const start = libRs.indexOf("tauri::generate_handler![");
    const handler = libRs.slice(start, libRs.indexOf("]", start));
    const registered = [...handler.matchAll(/^\s+([a-z_]+),$/gm)].map((m) => m[1]);
    expect(registered.length).toBeGreaterThan(0);
    expect(appCommands).toEqual(registered);
  });

  it("grants only commands build.rs names, and only as allow- permissions", () => {
    const known = new Set(appCommands);
    for (const cap of capabilities) {
      for (const p of cap.permissions) {
        expect(p, `${cap.identifier}: ${p}`).toMatch(/^allow-[a-z-]+$/);
        expect(known.has(p.slice("allow-".length).replace(/-/g, "_")), `${cap.identifier}: ${p}`).toBe(
          true,
        );
      }
    }
  });

  for (const label of Object.keys(ROOTS) as (keyof typeof ROOTS)[]) {
    it(`grants the ${label} window every command its code calls`, () => {
      const allowed = granted(label);
      const exempt = REACHED_NOT_CALLED[label];
      const reached = windowReach(label);
      const missing = [...reached].filter((c) => !allowed.has(c) && !(c in exempt));
      expect(missing).toEqual([]);
      for (const c of Object.keys(exempt)) {
        expect(reached.has(c), `${label}: ${c} is no longer reached`).toBe(true);
        expect(allowed.has(c), `${label}: ${c} is granted, drop the exemption`).toBe(false);
      }
    });
  }

  it("keeps account and gateway changes out of the tray popover", () => {
    const tray = granted("tray");
    for (const c of [
      "save_account",
      "clear_account",
      "switch_gateway",
      "set_org",
      "oauth_begin_login",
      "oauth_sign_out",
      "proxy_disable",
    ]) {
      expect(tray.has(c), c).toBe(false);
    }
  });
});
