// Outcomes of a staging evidence check (AG-907), and the rule that sorts an
// error into one of them.
//
// Four outcomes, each with its own exit code, because each sends whoever reads
// it somewhere different:
//
//   pass                 0  every request landed as the right user, org and source
//   routing-failure      1  staging answered, and what it holds is wrong or missing:
//                           a Gate Connect (or gateway) break
//   config-error         2  the run itself is misconfigured: missing secrets, a
//                           refused host, a read that is not implemented yet, a
//                           credential staging rejects. Nothing was proven either way
//   staging-unreachable  3  staging could not be asked (DNS, TLS, connect, timeout,
//                           5xx): an outage, not a Gate Connect break (AC4)
//
// When several requests disagree, the exit code is the most actionable one:
// config, then routing, then unreachable. A real routing failure is never
// hidden behind an outage on another request.

export const EXIT = Object.freeze({
  pass: 0,
  'routing-failure': 1,
  'config-error': 2,
  'staging-unreachable': 3,
});

const PRIORITY = ['config-error', 'routing-failure', 'staging-unreachable', 'pass'];

/** The outcome a set of per-request outcomes adds up to. Empty is a config
 *  error: a run that checked nothing proved nothing. */
export function worst(outcomes) {
  if (outcomes.length === 0) return 'config-error';
  for (const o of PRIORITY) if (outcomes.includes(o)) return o;
  throw new Error(`unknown outcome in ${JSON.stringify(outcomes)}`);
}

/** The run is misconfigured. Never retried. */
export class ConfigError extends Error {
  constructor(message) {
    super(message);
    this.name = 'ConfigError';
  }
}

/** A read the staging side has not provided yet (AG-1104, AG-1105). A config
 *  error by kind, named apart so the message says which subtask it waits on. */
export class NotImplementedError extends ConfigError {
  constructor(subtask, what) {
    super(`${what} is not implemented yet: it waits on ${subtask}`);
    this.name = 'NotImplementedError';
    this.subtask = subtask;
  }
}

/** A non-2xx answer. Classified by status: 5xx is an outage, 4xx is ours. */
export class HttpError extends Error {
  constructor(status, url, body) {
    super(`HTTP ${status} from ${url}${body ? `: ${String(body).slice(0, 300)}` : ''}`);
    this.name = 'HttpError';
    this.status = status;
    this.url = url;
  }
}

/** A 2xx whose body is not the JSON the contract promises. */
export class BadResponseError extends Error {
  constructor(url, detail) {
    super(`unreadable response from ${url}: ${detail}`);
    this.name = 'BadResponseError';
    this.url = url;
  }
}

const DNS = new Set(['ENOTFOUND', 'EAI_AGAIN', 'EAI_NONAME', 'EAI_FAIL', 'EAI_NODATA']);
const CONNECT = new Set([
  'ECONNREFUSED',
  'ECONNRESET',
  'EHOSTUNREACH',
  'ENETUNREACH',
  'ENETDOWN',
  'EPIPE',
  'ECONNABORTED',
  'UND_ERR_SOCKET',
  'UND_ERR_CLOSED',
]);
const TIMEOUT = new Set([
  'ETIMEDOUT',
  'ESOCKETTIMEDOUT',
  'UND_ERR_CONNECT_TIMEOUT',
  'UND_ERR_HEADERS_TIMEOUT',
  'UND_ERR_BODY_TIMEOUT',
  'ABORT_ERR',
]);
// OpenSSL's verify codes and Node's own TLS codes.
const TLS_CODES = new Set([
  'CERT_HAS_EXPIRED',
  'CERT_NOT_YET_VALID',
  'CERT_REVOKED',
  'CERT_UNTRUSTED',
  'CERT_REJECTED',
  'DEPTH_ZERO_SELF_SIGNED_CERT',
  'SELF_SIGNED_CERT_IN_CHAIN',
  'UNABLE_TO_VERIFY_LEAF_SIGNATURE',
  'UNABLE_TO_GET_ISSUER_CERT',
  'UNABLE_TO_GET_ISSUER_CERT_LOCALLY',
  'HOSTNAME_MISMATCH',
]);
const isTls = (code) => TLS_CODES.has(code) || /^ERR_(TLS|SSL)_/.test(code) || /^ERR_OSSL_/.test(code);

function* chain(err) {
  const seen = new Set();
  for (let e = err; e && typeof e === 'object' && !seen.has(e); e = e.cause) {
    seen.add(e);
    yield e;
    // AggregateError (Happy Eyeballs tries v4 and v6): any member counts.
    if (Array.isArray(e.errors)) for (const m of e.errors) yield* chain(m);
  }
}

/**
 * Sort an error from a staging call into an outcome.
 *
 *   { outcome: 'staging-unreachable', kind: 'dns'|'tls'|'connect'|'timeout'|'http-5xx'|'bad-response' }
 *   { outcome: 'config-error', kind: 'config'|'not-implemented'|'http-4xx' }
 *   { outcome: 'unknown', kind: 'unknown' }  a bug in this code, never an outage
 *
 * Node's fetch wraps the socket error as `TypeError('fetch failed')` with the
 * real one on `cause`, so the whole cause chain is read, not only the top.
 * Anything not recognised is `unknown` on purpose: guessing "unreachable" would
 * let a broken checker read as a gateway outage, which is the misreading AC4
 * exists to prevent in the other direction.
 */
export function classify(err) {
  for (const e of chain(err)) {
    if (e instanceof NotImplementedError) return { outcome: 'config-error', kind: 'not-implemented' };
    if (e instanceof ConfigError) return { outcome: 'config-error', kind: 'config' };
    if (e instanceof HttpError) {
      return e.status >= 500
        ? { outcome: 'staging-unreachable', kind: 'http-5xx' }
        : { outcome: 'config-error', kind: 'http-4xx' };
    }
    if (e instanceof BadResponseError) return { outcome: 'staging-unreachable', kind: 'bad-response' };
    if (e.name === 'TimeoutError' || e.name === 'AbortError') return { outcome: 'staging-unreachable', kind: 'timeout' };
    const code = typeof e.code === 'string' ? e.code : '';
    if (DNS.has(code)) return { outcome: 'staging-unreachable', kind: 'dns' };
    if (CONNECT.has(code)) return { outcome: 'staging-unreachable', kind: 'connect' };
    if (TIMEOUT.has(code)) return { outcome: 'staging-unreachable', kind: 'timeout' };
    if (code && isTls(code)) return { outcome: 'staging-unreachable', kind: 'tls' };
  }
  return { outcome: 'unknown', kind: 'unknown' };
}

/** The deepest message in the cause chain, which is the one that says why. */
export function describe(err) {
  let last = err;
  for (const e of chain(err)) last = e;
  const code = last && last.code ? ` (${last.code})` : '';
  const top = err && err.message ? err.message : String(err);
  const deep = last && last !== err && last.message ? `: ${last.message}${code}` : code;
  return top + deep;
}
