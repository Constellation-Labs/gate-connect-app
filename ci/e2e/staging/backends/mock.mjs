// The mock backend: reads the evidence the mock gateway recorded
// (ci/e2e/mock-gateway.mjs with MOCK_EVIDENCE=1, see ../mock-evidence.mjs).
// Loopback only, by the host guard.
//
// Env: MOCK_EVIDENCE_URL (default https://127.0.0.1:8443). The mock's
// self-signed CA reaches Node's fetch through NODE_EXTRA_CA_CERTS.
import { getJson } from '../lib/http.mjs';
import { assertTarget } from '../lib/host-guard.mjs';
import { normalizeCounts, normalizeRequests } from './normalize.mjs';

export function createMockBackend(env = process.env, { fetchImpl } = {}) {
  const base = env.MOCK_EVIDENCE_URL || 'https://127.0.0.1:8443';
  assertTarget(base, 'mock');
  const url = (path, q) => `${base.replace(/\/$/, '')}${path}?${new URLSearchParams(q)}`;
  return {
    name: 'mock',
    target: base,
    async preflight() {
      await getJson(url('/__e2e/health', {}), { backend: 'mock', fetchImpl });
    },
    async getRequestByMarker(marker) {
      const u = url('/__e2e/requests', { marker });
      return normalizeRequests(await getJson(u, { backend: 'mock', fetchImpl }), u);
    },
    async getOrgCounts({ orgId, from, to }) {
      const u = url('/__e2e/counts', { orgId, from, to });
      return normalizeCounts(await getJson(u, { backend: 'mock', fetchImpl }), u);
    },
  };
}
