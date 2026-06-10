// Builds the Cowork credential-helper binary and stages it where Tauri's
// `externalBin` expects it: src-tauri/binaries/gate-connect-cowork-helper-<triple>[.exe].
//
// Run before `tauri build` (wired via build.beforeBuildCommand in
// tauri.windows.conf.json). Tauri validates externalBin paths at compile
// time, so this must run before the src-tauri crate is compiled — hence
// beforeBuildCommand rather than beforeBundleCommand.
//
// Target triple: COWORK_HELPER_TARGET env var if set (match whatever
// `tauri build --target` uses), otherwise the host triple from `rustc -vV`.

import { execFileSync, execSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

function hostTriple() {
  const out = execSync("rustc -vV", { encoding: "utf8" });
  const line = out.split("\n").find((l) => l.startsWith("host:"));
  if (!line) throw new Error("could not parse host triple from `rustc -vV`");
  return line.slice("host:".length).trim();
}

const triple = process.env.COWORK_HELPER_TARGET || hostTriple();
const ext = triple.includes("windows") ? ".exe" : "";
const binName = `gate-connect-cowork-helper${ext}`;

console.log(`[stage-cowork-helper] building for ${triple}`);
execFileSync(
  "cargo",
  ["build", "--release", "-p", "gate-connect-cowork-helper", "--target", triple],
  { cwd: projectRoot, stdio: "inherit" },
);

const src = join(projectRoot, "target", triple, "release", binName);
const destDir = join(projectRoot, "src-tauri", "binaries");
const dest = join(destDir, `gate-connect-cowork-helper-${triple}${ext}`);

mkdirSync(destDir, { recursive: true });
copyFileSync(src, dest);
console.log(`[stage-cowork-helper] staged ${dest}`);
