// node --test ci/e2e/staging/test/
import assert from 'node:assert/strict';
import test from 'node:test';
import { assertRecord, checkEntry, waitForEvidence } from '../lib/evidence.mjs';
import { HttpError, NotImplementedError } from '../lib/outcome.mjs';

const good = {
  requestId: 'r1',
  createdAt: '2026-10-14T12:00:00.000Z',
  orgId: 'org-1',
  userId: 'u-1',
  clientTool: 'claude-code',
  machineId: 'install-1',
  harness: 'claude-code',
  verdict: { action: 'allow' },
};
const expect = { orgId: 'org-1', userId: 'u-1', clientTool: 'claude-code', harness: 'claude-code' };

// A clock the poll loop advances itself, so 5 minutes take no time.
function fakeClock(start = Date.parse('2026-10-14T12:00:00Z')) {
  let t = start;
  return { now: () => t, sleep: async (ms) => { t += ms; }, start };
}
const backendOf = (...answers) => {
  let i = 0;
  const calls = [];
  return {
    calls,
    async getRequestByMarker(m) {
      calls.push(m);
      const a = answers[Math.min(i++, answers.length - 1)];
      if (a instanceof Error) throw a;
      return a;
    },
  };
};
const refused = () => new TypeError('fetch failed', { cause: Object.assign(new Error('connect ECONNREFUSED'), { code: 'ECONNREFUSED' }) });

test('a matching record passes (AC2)', () => {
  assert.deepEqual(assertRecord(good, expect), []);
});

test('a missing org id fails with the expected and the actual value (AC3)', () => {
  const problems = assertRecord({ ...good, orgId: null }, expect);
  assert.deepEqual(problems, ['org id: expected "org-1", got (missing)']);
});

test('a wrong org, user, source or harness names both values', () => {
  const p = assertRecord({ ...good, orgId: 'org-2', userId: 'u-2', clientTool: 'codex', harness: 'direct-api' }, expect);
  assert.deepEqual(p, [
    'org id: expected "org-1", got "org-2"',
    'user id: expected "u-1", got "u-2"',
    'source (x-gate-client): expected "claude-code", got "codex"',
    'harness detection: expected "claude-code", got "direct-api"',
  ]);
});

test('a record with no verdict fails', () => {
  assert.match(assertRecord({ ...good, verdict: null }, expect)[0], /^verdict: expected one of allow, block, flag, redact, got \(missing\)$/);
  assert.match(assertRecord({ ...good, verdict: { action: 'maybe' } }, expect)[0], /got "maybe"/);
});

test('an expected verdict is checked when given', () => {
  assert.deepEqual(assertRecord(good, { ...expect, verdict: 'flag' }), ['verdict: expected "flag", got "allow"']);
});

test('polls until the marker shows up', async () => {
  const c = fakeClock();
  const b = backendOf([], [], [good]);
  const r = await waitForEvidence(b, 'm', { sentAt: c.start, now: c.now, sleep: c.sleep, intervalMs: 5000 });
  assert.equal(r.status, 'found');
  assert.equal(b.calls.length, 3);
});

test('gives up 5 minutes after sending, not after the first poll (AC1)', async () => {
  const c = fakeClock();
  const b = backendOf([]);
  // Sent two minutes before the first poll: three minutes are left.
  const r = await waitForEvidence(b, 'm', { sentAt: c.start - 120000, now: c.now, sleep: c.sleep, intervalMs: 10000 });
  assert.equal(r.status, 'not-found');
  assert.equal(c.now() - c.start, 180000);
  assert.equal(r.reads, 19);
});

test('an outage during the window is retried and can still pass', async () => {
  const c = fakeClock();
  const r = await waitForEvidence(backendOf(refused(), new HttpError(503, 'u'), [good]), 'm', {
    sentAt: c.start, now: c.now, sleep: c.sleep,
  });
  assert.equal(r.status, 'found');
  assert.equal(r.failures, 2);
});

test('staging down for the whole window is unreachable, not a routing failure (AC4)', async () => {
  const c = fakeClock();
  const r = await waitForEvidence(backendOf(refused()), 'm', { sentAt: c.start, now: c.now, sleep: c.sleep });
  assert.equal(r.status, 'unreachable');
  assert.equal(r.kind, 'connect');
  assert.equal(r.reads, 0);
});

test('staging answering at the end with nothing is a routing failure', async () => {
  const c = fakeClock();
  const r = await waitForEvidence(backendOf(refused(), refused(), []), 'm', { sentAt: c.start, now: c.now, sleep: c.sleep });
  assert.equal(r.status, 'not-found');
});

test('staging failing at the end is unreachable even after clean reads', async () => {
  const c = fakeClock();
  const answers = Array.from({ length: 30 }, () => []);
  answers.push(new HttpError(502, 'u'));
  const r = await waitForEvidence(backendOf(...answers), 'm', { sentAt: c.start, now: c.now, sleep: c.sleep, intervalMs: 5000 });
  assert.equal(r.status, 'unreachable');
  assert.equal(r.kind, 'http-5xx');
});

test('a config error stops at once', async () => {
  const c = fakeClock();
  const b = backendOf(new NotImplementedError('AG-1104', 'the marker read'));
  const r = await waitForEvidence(b, 'm', { sentAt: c.start, now: c.now, sleep: c.sleep });
  assert.equal(r.status, 'config');
  assert.equal(b.calls.length, 1);
  assert.match(r.error, /AG-1104/);
});

test('a checker bug is thrown, never reported as an outcome', async () => {
  const c = fakeClock();
  await assert.rejects(
    waitForEvidence(backendOf(new TypeError('boom')), 'm', { sentAt: c.start, now: c.now, sleep: c.sleep }),
    /boom/,
  );
});

test('checkEntry turns each status into its outcome and message', async () => {
  const c = fakeClock();
  const entry = { marker: 'm', step: 's', platform: 'linux', sentAt: new Date(c.start).toISOString(), expect };
  const o = { now: c.now, sleep: c.sleep };
  assert.equal((await checkEntry(backendOf([good]), entry, o)).outcome, 'pass');
  const miss = await checkEntry(backendOf([{ ...good, orgId: null }]), entry, o);
  assert.equal(miss.outcome, 'routing-failure');
  assert.match(miss.message, /org id: expected "org-1", got \(missing\)/);
  const c2 = fakeClock();
  const down = await checkEntry(backendOf(refused()), { ...entry, sentAt: new Date(c2.start).toISOString() }, { now: c2.now, sleep: c2.sleep });
  assert.equal(down.outcome, 'staging-unreachable');
  assert.match(down.message, /outage, not a routing failure/);
  const c3 = fakeClock();
  const lost = await checkEntry(backendOf([]), { ...entry, sentAt: new Date(c3.start).toISOString() }, { now: c3.now, sleep: c3.sleep });
  assert.equal(lost.outcome, 'routing-failure');
  assert.match(lost.message, /not found within 5 min/);
});

test('every record for a marker is checked', async () => {
  const c = fakeClock();
  const entry = { marker: 'm', step: 's', platform: 'linux', sentAt: new Date(c.start).toISOString(), expect };
  const r = await checkEntry(backendOf([good, { ...good, clientTool: null }]), entry, { now: c.now, sleep: c.sleep });
  assert.equal(r.outcome, 'routing-failure');
  assert.match(r.message, /^record 2: source/);
});
