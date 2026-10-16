// The staging backend: stubbed until the staging side lands (AG-907).
//
// It refuses to exist without the AG-1103 environment secrets, and every URL
// it would call passes the host guard first (AC6: staging hosts only, https
// only). The two reads throw NotImplementedError naming the subtask they wait
// on, so a run that reaches them before they are filled in fails as a config
// error and says why, rather than reading as a routing failure.
//
// Turning it on is configuration plus the two TODOs below:
//   1. AG-1102: a staging test org and an API key scoped to it.
//   2. AG-1103: GATE_STAGING_API_KEY, GATE_STAGING_ORG_ID and
//      GATE_STAGING_USER_ID as secrets of the `staging-e2e` environment,
//      readable by schedule and v* runs only.
//   3. AG-1104: fill in getRequestByMarker.
//   4. AG-1105: fill in getOrgCounts.
//
// Env:
//   GATE_STAGING_API_KEY      the scoped sk-gw-* key (AG-1102, AG-1103)
//   GATE_STAGING_ORG_ID       the test org's id, the value every request must carry
//   GATE_STAGING_USER_ID      the key owner's user id, likewise
//   GATE_STAGING_GATEWAY_URL  optional; default https://gateway-staging.constellationgate.ai
import { getJson } from '../lib/http.mjs';
import { assertTarget } from '../lib/host-guard.mjs';
import { ConfigError, NotImplementedError } from '../lib/outcome.mjs';
// Used once the TODOs are filled in.
import { normalizeCounts, normalizeRequests } from './normalize.mjs';

export const STAGING_GATEWAY_URL = 'https://gateway-staging.constellationgate.ai';
export const REQUIRED_SECRETS = Object.freeze(['GATE_STAGING_API_KEY', 'GATE_STAGING_ORG_ID', 'GATE_STAGING_USER_ID']);

export function missingSecrets(env = process.env) {
  return REQUIRED_SECRETS.filter((k) => !env[k] || !String(env[k]).trim());
}

export function createStagingBackend(env = process.env, { fetchImpl } = {}) {
  const missing = missingSecrets(env);
  if (missing.length) {
    throw new ConfigError(
      `the staging backend needs the AG-1103 secrets, and ${missing.join(', ')} ${missing.length > 1 ? 'are' : 'is'} not set`,
    );
  }
  const base = (env.GATE_STAGING_GATEWAY_URL || STAGING_GATEWAY_URL).replace(/\/$/, '');
  assertTarget(base, 'staging');
  if (!/^sk-gw-/.test(env.GATE_STAGING_API_KEY)) {
    throw new ConfigError('GATE_STAGING_API_KEY is not an sk-gw-* key');
  }
  const auth = { 'x-gate-api-key': env.GATE_STAGING_API_KEY };
  return {
    name: 'staging',
    target: base,
    orgId: env.GATE_STAGING_ORG_ID,
    userId: env.GATE_STAGING_USER_ID,

    // Not a stub: the gateway's own health route, so an outage is reported
    // as one before the suite runs (AC4).
    async preflight() {
      await getJson(`${base}/health`, { backend: 'staging', fetchImpl });
    },

    // TODO(AG-1104): query one request by its marker. Coded against the
    // response shape in normalize.mjs. When the route exists, this becomes:
    //
    //   const u = `${base}/<route from AG-1104>?marker=${encodeURIComponent(marker)}`;
    //   return normalizeRequests(await getJson(u, { backend: 'staging', headers: auth, fetchImpl }), u);
    //
    // The marker is stored in gateway_requests.custom_properties['fixture-nonce']
    // (see ../lib/marker.mjs). The read must be scoped to the test org.
    async getRequestByMarker(marker) {
      void marker;
      void auth;
      throw new NotImplementedError('AG-1104', 'the staging read of one request by its marker');
    },

    // TODO(AG-1105): an org's counts over [from, to]. Coded against the
    // response shape in normalize.mjs. When the route exists:
    //
    //   const u = `${base}/<route from AG-1105>?` + new URLSearchParams({ orgId, from, to });
    //   return normalizeCounts(await getJson(u, { backend: 'staging', headers: auth, fetchImpl }), u);
    //
    // /v1/me/activity carries the same three figures, but only for its fixed
    // 24h window, and it is the read Connect itself renders from; an
    // independent read is the stronger check.
    async getOrgCounts({ orgId, from, to }) {
      void orgId;
      void from;
      void to;
      throw new NotImplementedError('AG-1105', "the staging read of an org's counts over a window");
    },
  };
}
