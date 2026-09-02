// Assert the mock gateway captured a request whose path matches a needle and
// that carries the Gate headers expected for the given auth mode. Exits 0 on a
// match, 1 otherwise (printing what was captured to aid debugging).
//
// Usage: node assert-capture.mjs <capture.jsonl> <path-needle> <mode> [context]
//   mode = "api-key" -> requires x-gate-api-key + x-gate-upstream-url
//   mode = "oauth"   -> requires x-gate-authorization + x-gate-org-id +
//                       x-gate-upstream-url (and NO x-gate-api-key)
//   context = "standard" -> context-1m must be absent
//   context = "1m"       -> context-1m must be present
import fs from 'node:fs';

const [, , logPath, needle, mode = 'api-key', context = ''] = process.argv;
const lines = fs.existsSync(logPath)
  ? fs.readFileSync(logPath, 'utf8').split('\n').filter(Boolean)
  : [];

function ok(headers) {
  if (mode === 'oauth') {
    return (
      headers['x-gate-authorization'] &&
      headers['x-gate-org-id'] &&
      headers['x-gate-upstream-url'] &&
      !headers['x-gate-api-key']
    );
  }
  return headers['x-gate-api-key'] && headers['x-gate-upstream-url'];
}

const match = lines
  .map((l) => JSON.parse(l))
  .find((e) => (e.path || '').includes(needle) && ok(e.headers));

if (!match) {
  console.error(`no captured request matched "${needle}" with the ${mode} Gate headers`);
  console.error(`captured ${lines.length} request(s):`);
  console.error(lines.join('\n') || '  (none)');
  process.exit(1);
}

const beta = match.headers['anthropic-beta'] || '';
if (context === 'standard' && beta.includes('context-1m-2025-08-07')) {
  console.error('standard Claude model unexpectedly sent the 1M beta: ' + beta);
  process.exit(1);
}
if (context === '1m' && !beta.includes('context-1m-2025-08-07')) {
  console.error('Claude (1M) did not send the 1M beta: ' + (beta || '(absent)'));
  process.exit(1);
}

console.error(
  `matched ${match.method} ${match.path} ` +
    `x-gate-upstream-url=${match.headers['x-gate-upstream-url']}` +
    (mode === 'oauth' ? ` x-gate-org-id=${match.headers['x-gate-org-id']}` : ''),
);
