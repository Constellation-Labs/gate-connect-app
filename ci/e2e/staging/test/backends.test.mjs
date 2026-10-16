// node --test 'ci/e2e/staging/test/*.test.mjs'
import assert from 'node:assert/strict';
import http from 'node:http';
import test from 'node:test';
import { createBackend } from '../backends/index.mjs';
import { missingSecrets } from '../backends/staging.mjs';
import { compareCounts, parseDisplayed, windowFor } from '../lib/counts.mjs';
import { checkEntry } from '../lib/evidence.mjs';
import { assertTarget } from '../lib/host-guard.mjs';
import { ledgerEntry, makeMarker, MARKER_HEADER, MARKER_PATTERN, platformOf } from '../lib/marker.mjs';
import { ConfigError, NotImplementedError, classify } from '../lib/outcome.mjs';
import { createEvidenceStore } from '../mock-evidence.mjs';

const STAGING_ENV = {
  GATE_STAGING_API_KEY: 'sk-gw-test-not-real',
  GATE_STAGING_ORG_ID: 'org-s',
  GATE_STAGING_USER_ID: 'u-s',
};

test('markers fit the pattern the gateway keeps, and are unique per run, platform and step', () => {
  const seen = new Set();
  for (const runId of ['100', '101']) {
    for (const platform of ['macos', 'windows', 'linux']) {
      for (const step of ['chat-allow', 'chat-flag', 'Sign in / API key']) {
        const m = makeMarker({ runId, platform, step, nonce: 'fixed' });
        assert.match(m, MARKER_PATTERN);
        assert.ok(!seen.has(m), m);
        seen.add(m);
      }
    }
  }
  // A retried step gets a new marker.
  const a = makeMarker({ runId: '1', platform: 'linux', step: 's' });
  const b = makeMarker({ runId: '1', platform: 'linux', step: 's' });
  assert.notEqual(a, b);
  // A long step name is cut to the gateway's 24-character label.
  assert.match(makeMarker({ runId: '1', platform: 'macos', step: 'x'.repeat(80) }), MARKER_PATTERN);
});

test('the marker rides on the header the gateway stores as fixture-nonce', () => {
  assert.equal(MARKER_HEADER, 'x-gate-fixture-nonce');
  const e = ledgerEntry({ marker: 'recon-gcls-0123456789ab', runId: 1, attempt: 2, platform: 'linux', step: 's', sentAt: 0 });
  assert.equal(e.header, MARKER_HEADER);
  assert.equal(e.sentAt, '1970-01-01T00:00:00.000Z');
});

test('platform names map to ours', () => {
  assert.equal(platformOf('macOS'), 'macos');
  assert.equal(platformOf('Windows'), 'windows');
  assert.equal(platformOf('Linux'), 'linux');
  assert.equal(platformOf('darwin'), 'macos');
  assert.throws(() => platformOf('aix'));
});

test('AC6: staging may call only the staging hosts, over https', () => {
  assert.equal(assertTarget('https://gateway-staging.constellationgate.ai/health', 'staging').hostname, 'gateway-staging.constellationgate.ai');
  for (const url of [
    'https://gateway.constellationgate.ai',
    'https://api.constellationgate.ai',
    'https://app.constellationgate.ai',
    'https://GATEWAY.constellationgate.ai',
    'https://gateway-staging.constellationgate.ai.evil.example',
    'https://gateway-dev.constellationgate.ai',
    'http://gateway-staging.constellationgate.ai',
    'https://u:p@gateway-staging.constellationgate.ai',
    'https://127.0.0.1:8443',
    'not a url',
  ]) {
    assert.throws(() => assertTarget(url, 'staging'), ConfigError, url);
  }
  assert.throws(() => assertTarget('https://gateway.constellationgate.ai', 'staging'), /production/);
});

test('AC6: the mock may call only loopback', () => {
  assertTarget('https://127.0.0.1:8443', 'mock');
  assertTarget('http://localhost:1', 'mock');
  assert.throws(() => assertTarget('https://gateway-staging.constellationgate.ai', 'mock'), ConfigError);
  assert.throws(() => assertTarget('https://gateway.constellationgate.ai', 'mock'), ConfigError);
});

test('the staging backend refuses to run without the AG-1103 secrets', () => {
  assert.deepEqual(missingSecrets({}), ['GATE_STAGING_API_KEY', 'GATE_STAGING_ORG_ID', 'GATE_STAGING_USER_ID']);
  assert.throws(() => createBackend('staging', {}), /AG-1103 secrets.*GATE_STAGING_API_KEY, GATE_STAGING_ORG_ID, GATE_STAGING_USER_ID are not set/);
  assert.throws(() => createBackend('staging', { ...STAGING_ENV, GATE_STAGING_USER_ID: ' ' }), /GATE_STAGING_USER_ID is not set/);
});

test('the staging backend refuses a production gateway URL', () => {
  assert.throws(() => createBackend('staging', { ...STAGING_ENV, GATE_STAGING_GATEWAY_URL: 'https://gateway.constellationgate.ai' }), /production/);
});

test('the staging reads are stubs that name their subtask, and touch no network', async () => {
  let fetched = 0;
  const b = createBackend('staging', STAGING_ENV, { fetchImpl: async () => { fetched += 1; throw new Error('no'); } });
  await assert.rejects(b.getRequestByMarker('m'), (e) => e instanceof NotImplementedError && e.subtask === 'AG-1104');
  await assert.rejects(b.getOrgCounts({ orgId: 'o', from: 'a', to: 'b' }), (e) => e instanceof NotImplementedError && e.subtask === 'AG-1105');
  assert.equal(fetched, 0);
  assert.equal(classify(new NotImplementedError('AG-1104', 'x')).outcome, 'config-error');
});

test('an unknown backend is a config error', () => {
  assert.throws(() => createBackend('prod', {}), ConfigError);
});

test('Connect window: from the top of the hour 23 hours back, to the read', () => {
  assert.deepEqual(windowFor('2026-10-14T12:34:56.000Z'), { from: '2026-10-13T13:00:00.000Z', to: '2026-10-14T12:34:56.000Z' });
});

test('the tiles are read from a page dump, whatever the case and separators', () => {
  const dump = `=== window abc\n${JSON.stringify({ title: 'Gate Connect', text: 'Overview\nMESSAGES\n1,234\nBLOCKED/FLAGGED/REDACTED\n5\nTOKENS SAVED\n12%\nPolicies' })}\n=== window def\n${JSON.stringify({ text: 'other' })}\n`;
  assert.deepEqual(parseDisplayed(dump, 'T'), { readAt: 'T', messages: 1234, blockedFlagged: 5, tokensSavedPercent: 12 });
  assert.deepEqual(parseDisplayed('Messages\n0\nBlocked/Flagged/Redacted\n0\nTokens saved\nN/A', 'T').tokensSavedPercent, null);
  assert.throws(() => parseDisplayed('Messages\n3', 'T'), /"Blocked\/Flagged\/Redacted" tile is not on the page/);
  assert.throws(() => parseDisplayed('Messages\nloading\nBlocked/Flagged/Redacted\n0\nTokens saved\n1%', 'T'), /not a number or N\/A/);
});

test('the tiles are read from JSON', () => {
  const j = JSON.stringify({ messages: 3, blockedFlagged: 2, tokensSavedPercent: null, readAt: '2026-10-14T00:00:00Z' });
  assert.deepEqual(parseDisplayed(j), { messages: 3, blockedFlagged: 2, tokensSavedPercent: null, readAt: '2026-10-14T00:00:00Z' });
  assert.throws(() => parseDisplayed('{"messages":3}'), /no blockedFlagged/);
});

test('AC7: a mismatch names both values; a match is empty', () => {
  const backend = { messages: 3, blockedOrFlagged: 2, tokensSavedFraction: 0.2 };
  assert.deepEqual(compareCounts({ messages: 3, blockedFlagged: 2, tokensSavedPercent: 20 }, backend), []);
  assert.deepEqual(compareCounts({ messages: 4, blockedFlagged: 2, tokensSavedPercent: null }, backend), [
    'Messages: Connect displays 4, the backend counts 3',
    'Tokens saved: Connect displays N/A, the backend counts 20',
  ]);
  // The percentage is compared as Connect rounds it.
  assert.deepEqual(compareCounts({ messages: 0, blockedFlagged: 0, tokensSavedPercent: 13 }, { messages: 0, blockedOrFlagged: 0, tokensSavedFraction: 0.125 }), []);
});

// The mock gateway's evidence half and the mock backend, end to end over HTTP.
async function withMock(fn, env = {}) {
  const store = createEvidenceStore({ env });
  const server = http.createServer((req, res) => {
    if (store.handles(req)) return store.serve(req, res);
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      store.record(req, Buffer.concat(chunks));
      res.end('{}');
    });
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const base = `http://127.0.0.1:${server.address().port}`;
  try {
    await fn(base, createBackend('mock', { MOCK_EVIDENCE_URL: base }));
  } finally {
    server.close();
  }
}

const relayed = (base, marker, extra = {}, body = 'ping') =>
  fetch(`${base}/v1/messages`, {
    method: 'POST',
    headers: {
      'x-gate-api-key': 'sk-gw-e2e-00000000000000000000000000000000',
      'x-gate-client': 'claude-code',
      'x-gate-install-id': 'install-1',
      'user-agent': 'claude-cli/2.1.286 (external, cli)',
      [MARKER_HEADER]: marker,
      ...extra,
    },
    body: JSON.stringify({ messages: [{ role: 'user', content: body }], e2e_mock_usage: { prompt: 100, saved: 25 } }),
  });

test('mock: a relayed request is found by its marker and passes', async () => {
  await withMock(async (base, backend) => {
    await backend.preflight();
    const marker = makeMarker({ runId: '1', platform: 'linux', step: 'a' });
    await relayed(base, marker);
    const entry = { marker, step: 'a', platform: 'linux', sentAt: new Date().toISOString(), expect: { orgId: 'org-e2e-1', userId: 'u-e2e', clientTool: 'claude-code', harness: 'claude-code', machineId: 'install-1' } };
    const r = await checkEntry(backend, entry, { timeoutMs: 2000, intervalMs: 50 });
    assert.equal(r.outcome, 'pass', r.message);
  });
});

test('mock: an unknown key lands with no org, and fails as AC3 says', async () => {
  await withMock(async (base, backend) => {
    const marker = makeMarker({ runId: '1', platform: 'linux', step: 'b' });
    await relayed(base, marker, { 'x-gate-api-key': 'sk-gw-someone-else' });
    const entry = { marker, step: 'b', platform: 'linux', sentAt: new Date().toISOString(), expect: { orgId: 'org-e2e-1' } };
    const r = await checkEntry(backend, entry, { timeoutMs: 2000, intervalMs: 50 });
    assert.equal(r.outcome, 'routing-failure');
    assert.match(r.message, /org id: expected "org-e2e-1", got \(missing\)/);
  });
});

test('mock: a marker outside the gateway pattern is not stored, so it is never found', async () => {
  await withMock(async (base, backend) => {
    await relayed(base, 'free text');
    assert.deepEqual(await backend.getRequestByMarker('free text'), []);
  });
});

test('mock: records appear only after the ingestion lag', async () => {
  await withMock(async (base, backend) => {
    const marker = makeMarker({ runId: '1', platform: 'linux', step: 'c' });
    await relayed(base, marker);
    assert.deepEqual(await backend.getRequestByMarker(marker), []);
    const r = await checkEntry(backend, { marker, step: 'c', platform: 'linux', sentAt: new Date().toISOString(), expect: {} }, { timeoutMs: 3000, intervalMs: 50 });
    assert.equal(r.outcome, 'pass');
  }, { MOCK_EVIDENCE_DELAY_MS: '300' });
});

test('mock: counts cover the org and window, with verdicts and savings', async () => {
  await withMock(async (base, backend) => {
    const t0 = new Date(Date.now() - 1000).toISOString();
    await relayed(base, makeMarker({ runId: '1', platform: 'linux', step: 'd1' }));
    await relayed(base, makeMarker({ runId: '1', platform: 'linux', step: 'd2' }), {}, 'E2E-VERDICT-FLAG');
    await relayed(base, makeMarker({ runId: '1', platform: 'linux', step: 'd3' }), {}, 'E2E-VERDICT-BLOCK');
    await relayed(base, makeMarker({ runId: '1', platform: 'linux', step: 'd4' }), { 'x-gate-api-key': 'sk-gw-other' });
    const to = new Date(Date.now() + 1000).toISOString();
    const c = await backend.getOrgCounts({ orgId: 'org-e2e-1', from: t0, to });
    assert.deepEqual(c, { orgId: 'org-e2e-1', messages: 3, blockedOrFlagged: 2, tokensSavedFraction: 0.2 });
    const none = await backend.getOrgCounts({ orgId: 'org-e2e-1', from: to, to });
    assert.deepEqual(none, { orgId: 'org-e2e-1', messages: 0, blockedOrFlagged: 0, tokensSavedFraction: null });
  });
});

test('mock: a dead mock is staging unreachable, not a routing failure', async () => {
  // A port that was just free and is closed again: nothing listens there.
  const probe = http.createServer();
  await new Promise((r) => probe.listen(0, '127.0.0.1', r));
  const port = probe.address().port;
  await new Promise((r) => probe.close(r));
  const backend = createBackend('mock', { MOCK_EVIDENCE_URL: `http://127.0.0.1:${port}` });
  const r = await checkEntry(backend, { marker: 'm', step: 'e', platform: 'linux', sentAt: new Date().toISOString(), expect: {} }, { timeoutMs: 300, intervalMs: 100 });
  assert.equal(r.outcome, 'staging-unreachable');
});
