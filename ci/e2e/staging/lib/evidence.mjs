// The evidence check (AG-907 AC1, AC2, AC3, AC4): after a marked request is
// sent, ask the backend for it until it shows up or 5 minutes pass, then judge
// what the backend recorded against what the suite expected.
//
// A record, as both backends hand it over (backends/normalize.mjs):
//
//   { marker, requestId, createdAt, orgId, userId, clientTool, machineId,
//     harness, verdict: { action } | null }
//
// `clientTool` is the source (the `x-gate-client` slug Gate Connect stamps),
// `harness` is the gateway's own detection of the agent from the request
// (`agent_framework`), and `verdict` is the security decision recorded for it.
import { classify, describe } from './outcome.mjs';

export const DEFAULT_TIMEOUT_MS = 5 * 60 * 1000;
export const DEFAULT_INTERVAL_MS = 5000;
export const VERDICT_ACTIONS = Object.freeze(['allow', 'block', 'flag', 'redact']);

const show = (v) => (v === undefined || v === null || v === '' ? '(missing)' : JSON.stringify(v));

/**
 * Every way `record` differs from `expect`, one sentence each, naming the
 * expected and the actual value (AC3). An `expect` field left out is not
 * checked; the caller decides what a step expects.
 */
export function assertRecord(record, expect) {
  const problems = [];
  const field = (name, label) => {
    if (expect[name] === undefined) return;
    if (record[name] !== expect[name]) {
      problems.push(`${label}: expected ${show(expect[name])}, got ${show(record[name])}`);
    }
  };
  field('orgId', 'org id');
  field('userId', 'user id');
  field('clientTool', 'source (x-gate-client)');
  field('machineId', 'installation (x-gate-install-id)');
  field('harness', 'harness detection');
  const action = record.verdict && record.verdict.action;
  if (!VERDICT_ACTIONS.includes(action)) {
    problems.push(`verdict: expected one of ${VERDICT_ACTIONS.join(', ')}, got ${show(action)}`);
  } else if (expect.verdict !== undefined && action !== expect.verdict) {
    problems.push(`verdict: expected ${show(expect.verdict)}, got ${show(action)}`);
  }
  return problems;
}

const realSleep = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * Poll `backend.getRequestByMarker(marker)` until it returns records, the
 * deadline passes, or the run turns out to be misconfigured.
 *
 *   { status: 'found', records, reads, failures }
 *   { status: 'not-found', reads, failures, lastError }
 *   { status: 'unreachable', reads, failures, lastError, kind }
 *   { status: 'config', error, kind }
 *
 * The deadline runs from `sentAt`, not from the first poll: AC1 is "within 5
 * minutes of sending".
 *
 * Unreachable versus not found (AC4): an outage during the window is retried,
 * because a blip must not fail a run that would have found its marker. When
 * the window closes, the answer is "unreachable" if the last read failed as
 * an outage or no read ever succeeded, and "not found" only when staging was
 * answering at the end and still had nothing. So a gateway that is down reads
 * as down, and one that is up and lost the request reads as a routing failure.
 */
export async function waitForEvidence(backend, marker, opts = {}) {
  const {
    sentAt = new Date(),
    timeoutMs = DEFAULT_TIMEOUT_MS,
    intervalMs = DEFAULT_INTERVAL_MS,
    now = () => Date.now(),
    sleep = realSleep,
  } = opts;
  const deadline = new Date(sentAt).getTime() + timeoutMs;
  let reads = 0;
  let failures = 0;
  let lastError = null;
  let lastKind = null;
  let lastFailed = false;
  for (;;) {
    try {
      const records = await backend.getRequestByMarker(marker);
      reads += 1;
      lastFailed = false;
      if (records.length > 0) return { status: 'found', records, reads, failures };
    } catch (err) {
      const c = classify(err);
      if (c.outcome === 'config-error') return { status: 'config', error: describe(err), kind: c.kind };
      if (c.outcome !== 'staging-unreachable') throw err;
      failures += 1;
      lastFailed = true;
      lastError = describe(err);
      lastKind = c.kind;
    }
    const left = deadline - now();
    if (left <= 0) break;
    await sleep(Math.min(intervalMs, left));
  }
  if (lastFailed || reads === 0) return { status: 'unreachable', reads, failures, lastError, kind: lastKind };
  return { status: 'not-found', reads, failures, lastError };
}

/**
 * One ledger entry, judged: find it, then check every record found for it.
 * Returns { outcome, marker, step, platform, message }.
 */
export async function checkEntry(backend, entry, opts = {}) {
  const base = { marker: entry.marker, step: entry.step, platform: entry.platform };
  const mins = Math.round((opts.timeoutMs ?? DEFAULT_TIMEOUT_MS) / 60000);
  const r = await waitForEvidence(backend, entry.marker, { ...opts, sentAt: entry.sentAt });
  if (r.status === 'config') return { ...base, outcome: 'config-error', message: r.error };
  if (r.status === 'unreachable') {
    return {
      ...base,
      outcome: 'staging-unreachable',
      message:
        `staging unreachable (${r.kind || 'no answer'}): ${r.reads} clean read(s), ${r.failures} failed, ` +
        `last error: ${r.lastError}. This is an outage, not a routing failure.`,
    };
  }
  if (r.status === 'not-found') {
    return {
      ...base,
      outcome: 'routing-failure',
      message: `marker not found within ${mins} min of sending (${r.reads} clean read(s), ${r.failures} failed): the request did not land`,
    };
  }
  const problems = r.records.flatMap((rec, i) =>
    assertRecord(rec, entry.expect || {}).map((p) => (r.records.length > 1 ? `record ${i + 1}: ${p}` : p)),
  );
  if (problems.length) return { ...base, outcome: 'routing-failure', message: problems.join('; ') };
  const rec = r.records[0];
  return {
    ...base,
    outcome: 'pass',
    message:
      `landed as user ${rec.userId}, org ${rec.orgId}, source ${rec.clientTool}, harness ${rec.harness}, ` +
      `verdict ${rec.verdict.action}` + (r.records.length > 1 ? ` (${r.records.length} records)` : ''),
  };
}
