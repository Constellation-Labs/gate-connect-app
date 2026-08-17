// Minimal plain-HTTP mock for the OAuth phase of the real-tools e2e. Stands in
// for Cognito's token endpoint, the gateway's org-list endpoint, and
// dashboard-api's audit emit endpoint so `gate-connect login --oauth` and the
// audit instrumentation can complete headlessly:
//
//   POST /oauth2/token  -> a canned access/refresh token bundle
//   GET  /v1/me/orgs    -> a single org (so login auto-selects it)
//   POST /audit/emit    -> 201 accepted, request logged to MOCK_AUDIT_LOG
//
// Plain HTTP (not TLS) on purpose: gate-connect's token/org/audit reqwest
// clients use their default roots, so a self-signed cert would fail; the seams
// GATE_CONNECT_TEST_TOKEN_ENDPOINT / GATE_CONNECT_TEST_ORGS_ENDPOINT /
// GATE_CONNECT_TEST_AUDIT_ENDPOINT point them here over http instead. The
// relay->gateway hop stays HTTPS (mock-gateway.mjs).
//
// Env: MOCK_AUTH_PORT, MOCK_AUDIT_LOG.
import http from 'node:http';
import fs from 'node:fs';

const port = Number(process.env.MOCK_AUTH_PORT || 8455);
const auditLog = process.env.MOCK_AUDIT_LOG || '';

const server = http.createServer((req, res) => {
  const url = req.url || '';
  const chunks = [];
  req.on('data', (c) => chunks.push(c));
  req.on('end', () => {
    if (req.method === 'POST' && url.startsWith('/oauth2/token')) {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(
        JSON.stringify({
          access_token: 'at-e2e',
          refresh_token: 'rt-e2e',
          expires_in: 3600,
          token_type: 'Bearer',
        }),
      );
      return;
    }
    if (req.method === 'GET' && url.startsWith('/v1/me/orgs')) {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(
        JSON.stringify({
          user: { id: 'u-e2e', email: 'e2e@example.test' },
          orgs: [{ orgId: 'org-e2e-1', name: 'E2E Org', slug: 'e2e-org', role: 'owner' }],
        }),
      );
      return;
    }
    if (req.method === 'POST' && url.startsWith('/audit/emit')) {
      // One JSONL line per event: the auth/org headers plus the body, so the
      // suite can assert what the instrumentation actually sent.
      if (auditLog) {
        let body = {};
        try {
          body = JSON.parse(Buffer.concat(chunks).toString('utf8'));
        } catch {}
        const line = JSON.stringify({
          authorization: req.headers['authorization'] ?? null,
          orgHeader: req.headers['x-org-id'] ?? null,
          body,
        });
        fs.appendFileSync(auditLog, line + '\n');
      }
      res.writeHead(201, { 'content-type': 'application/json' });
      res.end('{"message":"Event accepted for processing"}');
      return;
    }
    res.writeHead(404, { 'content-type': 'application/json' });
    res.end('{"error":"not found"}');
  });
});

server.listen(port, '127.0.0.1', () => {
  console.log(`mock auth listening on http://127.0.0.1:${port}`);
});
