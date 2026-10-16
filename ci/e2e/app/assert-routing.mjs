// Routing state, read from outside the app (AG-905): the app's own record of
// whether routing is on, and what a connected tool's config says.
//
//   node assert-routing.mjs intent <routing-intent.json> on|off [secs]
//   node assert-routing.mjs config <codex config.toml> routed|direct [secs]
//
// Each polls for up to [secs] (default 0, one read), since the app writes
// these a moment after the window moves on.
//
// intent: `enabled` in the app's routing-intent.json, which routing::enable
// and disable keep current.
//
// config: Codex's `[model_providers.gate] base_url`. Routed is the loopback
// relay (http://127.0.0.1:<port>/...); direct is anything else, including no
// config at all. Either way the file must carry no Gate key (sk-gw-...): the
// relay injects the credential, so a config never holds it.
import fs from 'node:fs';

const [, , kind, file, want, secs = '0'] = process.argv;

function read() {
  try {
    return fs.readFileSync(file, 'utf8');
  } catch (e) {
    if (e.code === 'ENOENT') return null;
    throw e;
  }
}

function intentState() {
  const raw = read();
  if (raw === null) return { ok: want === 'off', seen: 'no intent file' };
  let enabled;
  try {
    enabled = JSON.parse(raw).enabled === true;
  } catch {
    return { ok: false, seen: `unparseable: ${raw.slice(0, 200)}` };
  }
  return { ok: enabled === (want === 'on'), seen: `enabled=${enabled}` };
}

// The `base_url` inside `[model_providers.gate]`, or null.
function gateBaseUrl(toml) {
  const block = /^\[model_providers\.gate\]\s*$([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(toml);
  if (!block) return null;
  return /^\s*base_url\s*=\s*"([^"]*)"/m.exec(block[1])?.[1] ?? null;
}

function configState() {
  const raw = read();
  if (raw === null) return { ok: want === 'direct', seen: 'no config.toml' };
  if (/sk-gw-/.test(raw)) return { ok: false, fatal: true, seen: 'a Gate key (sk-gw-...) is in the config' };
  const url = gateBaseUrl(raw);
  const provider = /^\s*model_provider\s*=\s*"([^"]*)"/m.exec(raw)?.[1] ?? null;
  const routed = provider === 'gate' && /^http:\/\/127\.0\.0\.1:\d+\//.test(url || '');
  return { ok: routed === (want === 'routed'), seen: `model_provider=${provider} gate base_url=${url}`, raw };
}

const check = { intent: intentState, config: configState }[kind];
if (!check || !file || !want) {
  console.log('usage: assert-routing.mjs intent <file> on|off [secs] | config <file> routed|direct [secs]');
  process.exit(2);
}

const until = Date.now() + Number(secs) * 1000;
let r = check();
while (!r.ok && !r.fatal && Date.now() < until) {
  await new Promise((res) => setTimeout(res, 500));
  r = check();
}
if (!r.ok) {
  console.log(`FAIL: ${kind} is not ${want} after ${secs}s: ${r.seen}`);
  if (r.raw && !r.fatal) console.log(r.raw);
  process.exit(1);
}
console.log(`${kind} ${want}: ${r.seen}`);
