// node --test ci/e2e/staging/test/
import assert from 'node:assert/strict';
import test from 'node:test';
import { BadResponseError, ConfigError, EXIT, HttpError, NotImplementedError, classify, describe, worst } from '../lib/outcome.mjs';

const sysErr = (code, message = code) => Object.assign(new Error(message), { code });
// The shape Node's fetch throws: a TypeError with the socket error on `cause`.
const fetchFailed = (cause) => new TypeError('fetch failed', { cause });

test('each outcome has its own exit code', () => {
  assert.deepEqual(EXIT, { pass: 0, 'routing-failure': 1, 'config-error': 2, 'staging-unreachable': 3 });
  assert.equal(new Set(Object.values(EXIT)).size, 4);
});

test('DNS, TLS, connect and timeout failures are staging unreachable (AC4)', () => {
  const cases = [
    [fetchFailed(sysErr('ENOTFOUND', 'getaddrinfo ENOTFOUND gateway-staging.constellationgate.ai')), 'dns'],
    [fetchFailed(sysErr('EAI_AGAIN')), 'dns'],
    [fetchFailed(sysErr('CERT_HAS_EXPIRED')), 'tls'],
    [fetchFailed(sysErr('UNABLE_TO_VERIFY_LEAF_SIGNATURE')), 'tls'],
    [fetchFailed(sysErr('ERR_TLS_CERT_ALTNAME_INVALID')), 'tls'],
    [fetchFailed(sysErr('ERR_SSL_WRONG_VERSION_NUMBER')), 'tls'],
    [fetchFailed(sysErr('ECONNREFUSED')), 'connect'],
    [fetchFailed(sysErr('ECONNRESET')), 'connect'],
    [fetchFailed(sysErr('UND_ERR_SOCKET')), 'connect'],
    [fetchFailed(sysErr('UND_ERR_CONNECT_TIMEOUT')), 'timeout'],
    [fetchFailed(sysErr('ETIMEDOUT')), 'timeout'],
    [Object.assign(new Error('The operation was aborted due to timeout'), { name: 'TimeoutError' }), 'timeout'],
  ];
  for (const [err, kind] of cases) {
    assert.deepEqual(classify(err), { outcome: 'staging-unreachable', kind }, describe(err));
  }
});

test('a Happy Eyeballs AggregateError is read through its members', () => {
  const agg = new AggregateError([sysErr('ECONNREFUSED'), sysErr('EHOSTUNREACH')], 'all failed');
  assert.deepEqual(classify(fetchFailed(agg)), { outcome: 'staging-unreachable', kind: 'connect' });
});

test('5xx is an outage; 4xx is our configuration', () => {
  for (const s of [500, 502, 503, 504]) {
    assert.deepEqual(classify(new HttpError(s, 'https://x')), { outcome: 'staging-unreachable', kind: 'http-5xx' });
  }
  for (const s of [400, 401, 403, 404]) {
    assert.deepEqual(classify(new HttpError(s, 'https://x')), { outcome: 'config-error', kind: 'http-4xx' });
  }
});

test('an unreadable 2xx is treated as an outage, not a routing failure', () => {
  assert.equal(classify(new BadResponseError('https://x', 'not JSON')).outcome, 'staging-unreachable');
});

test('config and not-implemented are config errors, and name their subtask', () => {
  assert.deepEqual(classify(new ConfigError('no key')), { outcome: 'config-error', kind: 'config' });
  const ni = new NotImplementedError('AG-1104', 'the marker read');
  assert.deepEqual(classify(ni), { outcome: 'config-error', kind: 'not-implemented' });
  assert.match(ni.message, /AG-1104/);
});

test('an unrecognised error is unknown, never guessed to be an outage', () => {
  assert.deepEqual(classify(new TypeError('x is not a function')), { outcome: 'unknown', kind: 'unknown' });
  assert.deepEqual(classify(fetchFailed(sysErr('EWEIRD'))), { outcome: 'unknown', kind: 'unknown' });
});

test('worst picks the most actionable outcome', () => {
  assert.equal(worst(['pass', 'pass']), 'pass');
  assert.equal(worst(['pass', 'staging-unreachable']), 'staging-unreachable');
  // A real routing failure is never hidden behind an outage elsewhere.
  assert.equal(worst(['staging-unreachable', 'routing-failure']), 'routing-failure');
  assert.equal(worst(['routing-failure', 'config-error']), 'config-error');
  assert.equal(worst([]), 'config-error');
});

test('describe reaches the root cause', () => {
  const m = describe(fetchFailed(sysErr('ENOTFOUND', 'getaddrinfo ENOTFOUND h')));
  assert.match(m, /fetch failed: getaddrinfo ENOTFOUND h \(ENOTFOUND\)/);
});
