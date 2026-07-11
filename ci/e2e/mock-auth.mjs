// Minimal plain-HTTP mock for the OAuth phase of the real-tools e2e. Stands in
// for Cognito's token endpoint and the gateway's org-list endpoint so
// `gate-connect login --oauth` can complete headlessly:
//
//   POST /oauth2/token  -> a canned access/refresh token bundle
//   GET  /v1/me/orgs    -> a single org (so login auto-selects it)
//
// Plain HTTP (not TLS) on purpose: gate-connect's token/org reqwest clients use
// their default roots, so a self-signed cert would fail; the seams
// GATE_CONNECT_TEST_TOKEN_ENDPOINT / GATE_CONNECT_TEST_ORGS_ENDPOINT point them
// here over http instead. The relay->gateway hop stays HTTPS (mock-gateway.mjs).
//
// Env: MOCK_AUTH_PORT.
import http from 'node:http';

const port = Number(process.env.MOCK_AUTH_PORT || 8455);

const server = http.createServer((req, res) => {
  const url = req.url || '';
  req.on('data', () => {});
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
    res.writeHead(404, { 'content-type': 'application/json' });
    res.end('{"error":"not found"}');
  });
});

server.listen(port, '127.0.0.1', () => {
  console.log(`mock auth listening on http://127.0.0.1:${port}`);
});
