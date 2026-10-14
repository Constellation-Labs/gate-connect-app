// The signed-in state, read from outside the app (AG-905): the app's own record
// of the sign-in, and the requests the mock gateway received.
//
//   node assert-signin.mjs <account.json> <capture.jsonl> <key> <gateway-host>
//
// account.json must carry the key's first 12 characters, never the key, beside
// the gateway it was saved against. The mock (and only the mock, since the host
// resolves to 127.0.0.1) must have received requests carrying the key.
import fs from 'node:fs';

const [, , accountPath, capturePath, key, host] = process.argv;

const raw = fs.readFileSync(accountPath, 'utf8');
const acct = JSON.parse(raw);
console.log(JSON.stringify(acct, null, 2));
const problems = [];
if (acct.api_key_prefix !== key.slice(0, 12)) problems.push(`api_key_prefix is ${JSON.stringify(acct.api_key_prefix)}`);
if (!String(acct.gateway_base_url || '').includes(host)) problems.push(`gateway_base_url is ${JSON.stringify(acct.gateway_base_url)}`);
if (raw.includes(key)) problems.push('the key itself is on disk');

// The dashboard's reads follow the window's move to "Name this device" rather
// than precede it, so give them a moment to land.
function captured() {
  return fs
    .readFileSync(capturePath, 'utf8')
    .split('\n')
    .filter((l) => l.trim())
    .map((l) => JSON.parse(l));
}
let reqs = captured();
const until = Date.now() + 30_000;
const keyedPaths = () => reqs.filter((r) => r.headers['x-gate-api-key'] === key).map((r) => r.path);
while (keyedPaths().length === 0 && Date.now() < until) {
  await new Promise((r) => setTimeout(r, 500));
  reqs = captured();
}
const keyed = keyedPaths();
console.log(`${reqs.length} requests reached the mock gateway, ${keyed.length} with the key: ${JSON.stringify([...new Set(keyed)].sort())}`);
if (keyed.length === 0) {
  problems.push('no request reached the mock gateway with the key');
  console.log(`unkeyed: ${JSON.stringify(reqs.map((r) => `${r.method} ${r.path}`))}`);
}

if (problems.length) {
  for (const p of problems) console.log('FAIL: ' + p);
  process.exit(1);
}
