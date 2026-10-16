// The staging evidence CLI (AG-907). One command per call, like the app
// drivers, so the shell scripts read step by step.
//
//   node check.mjs mark <step>
//       Mint a marker for this run, platform and step, append it to the ledger
//       and print it. For a driver that attaches the header itself.
//   node check.mjs send <step> <url> [verdict]
//       Mint a marker and send one marked chat request to <url> (the relay),
//       as Claude Code would. verdict = allow (default), flag or block decides
//       what the mock's security stand-in records, and what is expected.
//   node check.mjs evidence
//       For every ledger entry, poll the backend for its marker and assert
//       user, org, source, harness detection and a verdict (AC1 to AC4).
//   node check.mjs counts <displayed-file|ledger>
//       Compare what Connect displays for its window with the backend's
//       counts for the same org and window (AC7).
//   node check.mjs assert-host <url> <backend>
//       Exit 0 when <backend> may call <url>, 2 otherwise (AC6).
//   node check.mjs preflight
//       Ask the backend whether it can be asked at all.
//
// Exit codes are outcome.mjs's: 0 pass, 1 routing failure, 2 config error,
// 3 staging unreachable. Results are appended to $GITHUB_STEP_SUMMARY when set.
//
// Env:
//   E2E_BACKEND        mock | staging
//   E2E_LEDGER         the ledger file (JSON lines)
//   E2E_RESULT         where `evidence` and `counts` write their JSON result
//   E2E_EXPECT_ORG_ID, E2E_EXPECT_USER_ID   what every request must carry;
//                      the staging backend takes them from its secrets
//   E2E_EXPECT_SOURCE  the x-gate-client slug (default claude-code)
//   E2E_EXPECT_HARNESS the gateway's harness detection (default claude-code)
//   E2E_TIMEOUT_MS, E2E_INTERVAL_MS   polling (default 5 min, 5 s)
//   GITHUB_RUN_ID, GITHUB_RUN_ATTEMPT, RUNNER_OS   the marker's inputs
import fs from 'node:fs';
import { createBackend } from './backends/index.mjs';
import { compareCounts, parseDisplayed, windowFor } from './lib/counts.mjs';
import { checkEntry, DEFAULT_INTERVAL_MS, DEFAULT_TIMEOUT_MS } from './lib/evidence.mjs';
import { assertTarget } from './lib/host-guard.mjs';
import { ledgerEntry, makeMarker, MARKER_HEADER, platformOf } from './lib/marker.mjs';
import { ConfigError, EXIT, classify, describe, worst } from './lib/outcome.mjs';

const env = process.env;
const say = (line) => console.log(line);
function summary(lines) {
  if (env.GITHUB_STEP_SUMMARY) fs.appendFileSync(env.GITHUB_STEP_SUMMARY, lines.join('\n') + '\n\n');
}
function finish(outcome, message) {
  say(`${outcome}: ${message}`);
  process.exit(EXIT[outcome]);
}

function runInfo() {
  return {
    runId: env.GITHUB_RUN_ID || `local-${process.pid}-${Date.now()}`,
    attempt: env.GITHUB_RUN_ATTEMPT || '1',
    platform: platformOf(env.RUNNER_OS || process.platform),
  };
}

function ledgerPath() {
  if (!env.E2E_LEDGER) throw new ConfigError('E2E_LEDGER must name the ledger file');
  return env.E2E_LEDGER;
}

function readLedger() {
  const p = ledgerPath();
  if (!fs.existsSync(p)) return [];
  return fs
    .readFileSync(p, 'utf8')
    .split('\n')
    .filter((l) => l.trim())
    .map((l) => JSON.parse(l));
}

function expectations(backend, extra = {}) {
  return {
    orgId: backend.orgId || env.E2E_EXPECT_ORG_ID,
    userId: backend.userId || env.E2E_EXPECT_USER_ID,
    clientTool: env.E2E_EXPECT_SOURCE || 'claude-code',
    harness: env.E2E_EXPECT_HARNESS || 'claude-code',
    ...extra,
  };
}

function mark(backend, step, extra) {
  const info = runInfo();
  const marker = makeMarker({ ...info, step });
  const entry = ledgerEntry({ marker, ...info, step, expect: expectations(backend, extra) });
  fs.appendFileSync(ledgerPath(), JSON.stringify(entry) + '\n');
  return entry;
}

const BODY_TAG = { flag: 'E2E-VERDICT-FLAG', block: 'E2E-VERDICT-BLOCK' };

async function send(backend, step, url, verdict = 'allow') {
  if (!['allow', 'flag', 'block'].includes(verdict)) throw new ConfigError(`verdict must be allow, flag or block`);
  // The relay is on loopback; whatever it forwards to was checked when the
  // account was signed in (assert-host in the script).
  assertTarget(url, 'mock');
  // The mock's security stand-in decides from the body. Against staging the
  // verdict comes from the test org's policies, so it is not pinned.
  const extra = backend.name === 'mock' ? { verdict } : {};
  const entry = mark(backend, step, extra);
  const body = {
    model: 'claude-sonnet-4-5',
    max_tokens: 8,
    messages: [{ role: 'user', content: `ping ${entry.marker} ${BODY_TAG[verdict] || ''}`.trim() }],
    e2e_mock_usage: backend.name === 'mock' ? { prompt: 100, saved: 25 } : undefined,
  };
  const res = await fetch(url, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      'anthropic-version': '2023-06-01',
      'user-agent': 'claude-cli/2.1.286 (external, cli)',
      'x-api-key': 'sk-ant-e2e-not-a-key',
      [MARKER_HEADER]: entry.marker,
    },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(30000),
  });
  const text = await res.text();
  say(`sent ${step} (${entry.marker}): HTTP ${res.status} ${text.slice(0, 200)}`);
  if (!res.ok) throw new ConfigError(`the relay answered ${res.status} for ${step}; the request never reached the gateway`);
  return entry;
}

async function evidence(backend) {
  const entries = readLedger();
  if (entries.length === 0) throw new ConfigError(`the ledger ${ledgerPath()} is empty: nothing was sent, so nothing can be checked`);
  const opts = {
    timeoutMs: Number(env.E2E_TIMEOUT_MS || DEFAULT_TIMEOUT_MS),
    intervalMs: Number(env.E2E_INTERVAL_MS || DEFAULT_INTERVAL_MS),
  };
  const results = await Promise.all(entries.map((e) => checkEntry(backend, e, opts)));
  for (const r of results) say(`${r.outcome.padEnd(19)} ${r.platform}/${r.step} ${r.marker}: ${r.message}`);
  const outcome = worst(results.map((r) => r.outcome));
  if (env.E2E_RESULT) fs.writeFileSync(env.E2E_RESULT, JSON.stringify({ check: 'evidence', backend: backend.name, outcome, results }, null, 2));
  summary([
    `### Evidence (${backend.name} backend, ${runInfo().platform}): ${outcome}`,
    '',
    '| step | marker | outcome | detail |',
    '|---|---|---|---|',
    ...results.map((r) => `| ${r.step} | \`${r.marker}\` | ${r.outcome} | ${r.message.replace(/\|/g, '\\|')} |`),
  ]);
  return { outcome, message: `${results.filter((r) => r.outcome === 'pass').length}/${results.length} request(s) landed as expected` };
}

/** The ledger's own totals, in the tiles' terms. The stand-in for Connect's
 *  display until the installed-app driver reads the Overview pane. */
function ledgerDisplayed(entries, readAt) {
  const mine = entries.filter((e) => e.expect && e.expect.orgId);
  return {
    messages: mine.length,
    blockedFlagged: mine.filter((e) => ['block', 'flag', 'redact'].includes(e.expect.verdict)).length,
    tokensSavedPercent: mine.length ? 20 : null,
    readAt,
  };
}

async function counts(backend, source) {
  const readAt = new Date().toISOString();
  let displayed;
  let from;
  if (source === 'ledger') {
    // The ledger can only stand in for the mock, whose usage figures the
    // suite itself sent. Against staging the displayed side must be Connect's.
    if (backend.name !== 'mock') throw new ConfigError('counts against staging need what Connect displays, not the ledger');
    const entries = readLedger();
    displayed = ledgerDisplayed(entries, readAt);
    from = 'the ledger (stand-in for Connect until the driver reads the Overview pane)';
  } else {
    displayed = parseDisplayed(fs.readFileSync(source, 'utf8'), readAt);
    from = source;
  }
  const window = windowFor(displayed.readAt);
  const orgId = backend.orgId || env.E2E_EXPECT_ORG_ID;
  if (!orgId) throw new ConfigError('no org id to count for (E2E_EXPECT_ORG_ID, or the staging secrets)');
  const got = await backend.getOrgCounts({ orgId, ...window });
  const problems = compareCounts(displayed, got);
  const outcome = problems.length ? 'routing-failure' : 'pass';
  const message = problems.length
    ? problems.join('; ')
    : `Messages ${displayed.messages}, Blocked/Flagged/Redacted ${displayed.blockedFlagged}, Tokens saved ${displayed.tokensSavedPercent ?? 'N/A'}% agree`;
  if (env.E2E_RESULT) fs.writeFileSync(env.E2E_RESULT, JSON.stringify({ check: 'counts', backend: backend.name, outcome, window, displayed, backendCounts: got, problems }, null, 2));
  summary([
    `### Counts (${backend.name} backend, ${runInfo().platform}): ${outcome}`,
    '',
    `Org \`${orgId}\`, window ${window.from} to ${window.to}. Displayed values from ${from}.`,
    '',
    message,
  ]);
  return { outcome, message };
}

async function main([cmd, ...args]) {
  if (cmd === 'assert-host') {
    assertTarget(args[0], args[1]);
    return { outcome: 'pass', message: `${args[1]} may call ${new URL(args[0]).origin}` };
  }
  const backend = createBackend(env.E2E_BACKEND || 'mock');
  switch (cmd) {
    case 'preflight':
      await backend.preflight();
      return { outcome: 'pass', message: `${backend.name} backend at ${backend.target} answers` };
    case 'mark':
      say(mark(backend, args[0]).marker);
      process.exit(0);
      break;
    case 'send':
      await send(backend, args[0], args[1], args[2]);
      return { outcome: 'pass', message: `sent ${args[0]}` };
    case 'evidence':
      return evidence(backend);
    case 'counts':
      return counts(backend, args[0] || 'ledger');
    default:
      throw new ConfigError('usage: check.mjs mark|send|evidence|counts|assert-host|preflight ...');
  }
}

main(process.argv.slice(2)).then(
  ({ outcome, message }) => finish(outcome, message),
  (err) => {
    const c = classify(err);
    if (c.outcome === 'unknown') {
      console.error(err);
      finish('config-error', `checker error, not a gateway result: ${describe(err)}`);
    }
    const message =
      c.outcome === 'staging-unreachable'
        ? `staging unreachable (${c.kind}): ${describe(err)}. This is an outage, not a routing failure.`
        : describe(err);
    summary([`### ${c.outcome}`, '', message]);
    finish(c.outcome, message);
  },
);
