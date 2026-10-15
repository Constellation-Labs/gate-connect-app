// The evidence half of the mock gateway (AG-907). Loaded by
// ci/e2e/mock-gateway.mjs only when MOCK_EVIDENCE=1, so every existing caller
// of the mock is unchanged.
//
// It plays the two parts of the real gateway the staging check reads:
//
//   record(req, body)  on every proxied request, what the gateway would write
//                      to gateway_requests: org and user from the credential,
//                      source from x-gate-client, installation from
//                      x-gate-install-id, harness detection from the request,
//                      a security verdict, and the marker from
//                      x-gate-fixture-nonce (dropped unless it fits the
//                      gateway's pattern, as the gateway drops it).
//   serve(req, res)    the two reads the staging side will provide, in the
//                      shapes backends/normalize.mjs documents:
//                        GET /__e2e/requests?marker=M   (AG-1104's stand-in)
//                        GET /__e2e/counts?orgId&from&to (AG-1105's stand-in)
//                        GET /__e2e/health
//
// The simulation is deliberately simple; what it must get right is the
// contract, not the gateway's logic:
//   - credentials: MOCK_EVIDENCE_KEYS, JSON { "<sk-gw key or bearer token>":
//     { "userId", "orgId" } }. An OAuth request takes its org from
//     x-gate-org-id, as the gateway does. An unknown credential records no org
//     and no user, which is how a mis-attributed request looks (AC3).
//   - harness: a few User-Agent needles, else "direct-api".
//   - verdict: "block" or "flag" when the body carries E2E-VERDICT-BLOCK or
//     E2E-VERDICT-FLAG, else "allow".
//   - token savings: a body field `e2e_mock_usage: { prompt, saved }`, standing
//     in for the compression the real gateway measures.
//   - MOCK_EVIDENCE_DELAY_MS: a record becomes visible to the reads only after
//     this long, standing in for staging's ingestion lag.
import { MARKER_HEADER, MARKER_PATTERN } from './lib/marker.mjs';

const DEFAULT_KEYS = {
  'sk-gw-e2e-00000000000000000000000000000000': { userId: 'u-e2e', orgId: 'org-e2e-1' },
  'at-e2e': { userId: 'u-e2e', orgId: null },
};

const HARNESS = [
  [/claude-cli|claude-code/i, 'claude-code'],
  [/codex/i, 'codex'],
  [/opencode/i, 'opencode'],
  [/openclaw/i, 'openclaw'],
  [/hermes/i, 'hermes'],
];

export function createEvidenceStore({ env = process.env, now = () => Date.now() } = {}) {
  const keys = env.MOCK_EVIDENCE_KEYS ? JSON.parse(env.MOCK_EVIDENCE_KEYS) : DEFAULT_KEYS;
  const delayMs = Number(env.MOCK_EVIDENCE_DELAY_MS || 0);
  const records = [];
  let seq = 0;

  function record(req, body) {
    const h = req.headers;
    const bearer = String(h['x-gate-authorization'] || '').replace(/^Bearer\s+/i, '');
    const apiKey = h['x-gate-api-key'];
    const who = (apiKey ? keys[apiKey] : bearer ? keys[bearer] : null) || null;
    const orgId = apiKey ? (who && who.orgId) || null : bearer && who ? h['x-gate-org-id'] || null : null;
    const ua = String(h['user-agent'] || '');
    const text = body.toString('utf8');
    let usage = null;
    try {
      const j = JSON.parse(text);
      if (j && j.e2e_mock_usage) usage = j.e2e_mock_usage;
    } catch {}
    const marker = typeof h[MARKER_HEADER] === 'string' && MARKER_PATTERN.test(h[MARKER_HEADER]) ? h[MARKER_HEADER] : null;
    const at = now();
    records.push({
      visibleAt: at + delayMs,
      createdAt: new Date(at).toISOString(),
      requestId: `mock-${++seq}`,
      marker,
      orgId,
      userId: (who && who.userId) || null,
      clientTool: typeof h['x-gate-client'] === 'string' ? h['x-gate-client'] : null,
      machineId: typeof h['x-gate-install-id'] === 'string' ? h['x-gate-install-id'] : null,
      agentFramework: (HARNESS.find(([re]) => re.test(ua)) || [null, 'direct-api'])[1],
      verdict: { action: text.includes('E2E-VERDICT-BLOCK') ? 'block' : text.includes('E2E-VERDICT-FLAG') ? 'flag' : 'allow' },
      usage,
    });
  }

  const json = (res, status, value) => {
    res.writeHead(status, { 'content-type': 'application/json' });
    res.end(JSON.stringify(value));
  };

  function handles(req) {
    return (req.url || '').startsWith('/__e2e/');
  }

  function serve(req, res) {
    const u = new URL(req.url, 'https://mock.invalid');
    const visible = records.filter((r) => r.visibleAt <= now());
    if (u.pathname === '/__e2e/health') return json(res, 200, { ok: true, records: records.length });
    if (u.pathname === '/__e2e/requests') {
      const marker = u.searchParams.get('marker');
      if (!marker) return json(res, 400, { error: 'marker is required' });
      const requests = visible
        .filter((r) => r.marker === marker)
        .map(({ requestId, createdAt, orgId, userId, clientTool, machineId, agentFramework, verdict }) => ({
          requestId,
          createdAt,
          orgId,
          userId,
          clientTool,
          machineId,
          agentFramework,
          verdict,
        }));
      return json(res, 200, { requests });
    }
    if (u.pathname === '/__e2e/counts') {
      const orgId = u.searchParams.get('orgId');
      const from = Date.parse(u.searchParams.get('from'));
      const to = Date.parse(u.searchParams.get('to'));
      if (!orgId || Number.isNaN(from) || Number.isNaN(to)) return json(res, 400, { error: 'orgId, from and to are required' });
      const inWindow = visible.filter((r) => {
        const t = Date.parse(r.createdAt);
        return r.orgId === orgId && t >= from && t <= to;
      });
      let saved = 0;
      let sent = 0;
      for (const r of inWindow) {
        if (r.usage && r.usage.prompt > 0) {
          saved += Math.max(0, r.usage.saved || 0);
          sent += r.usage.prompt;
        }
      }
      return json(res, 200, {
        orgId,
        window: { from: new Date(from).toISOString(), to: new Date(to).toISOString() },
        messages: inWindow.length,
        blockedOrFlagged: inWindow.filter((r) => ['block', 'flag', 'redact'].includes(r.verdict.action)).length,
        tokensSaved: saved + sent > 0 ? { fraction: saved / (saved + sent) } : null,
      });
    }
    return json(res, 404, { error: 'not found' });
  }

  return { record, handles, serve, records };
}
