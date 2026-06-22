// Assert the mock gateway captured a request whose path matches a needle and
// that carries both Gate headers. Exits 0 on a match, 1 otherwise (printing
// what was captured to aid debugging a failed e2e run).
//
// Usage: node assert-capture.mjs <capture.jsonl> <path-needle>
import fs from 'node:fs';

const [, , logPath, needle] = process.argv;
const lines = fs.existsSync(logPath)
  ? fs.readFileSync(logPath, 'utf8').split('\n').filter(Boolean)
  : [];

const match = lines
  .map((l) => JSON.parse(l))
  .find(
    (e) =>
      (e.path || '').includes(needle) &&
      e.headers['x-gate-api-key'] &&
      e.headers['x-gate-upstream-url'],
  );

if (!match) {
  console.error(`no captured request matched "${needle}" with both Gate headers`);
  console.error(`captured ${lines.length} request(s):`);
  console.error(lines.join('\n') || '  (none)');
  process.exit(1);
}

console.error(
  `matched ${match.method} ${match.path} ` +
    `x-gate-upstream-url=${match.headers['x-gate-upstream-url']}`,
);
