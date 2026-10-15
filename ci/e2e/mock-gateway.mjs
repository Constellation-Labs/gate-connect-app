// Minimal HTTPS server standing in for the Gate gateway in the real-tools
// e2e job. Records each request it receives (method, path, headers) as one
// JSON line in $CAPTURE_LOG, then answers 200 - so a real AI CLI pointed at
// it via `gate-connect connect` emits exactly one request we can assert on.
//
// Env: MOCK_PORT, MOCK_CERT (PEM), MOCK_KEY (PEM), CAPTURE_LOG (output path).
//
// MOCK_EVIDENCE=1 (AG-907) also records what the gateway would store for each
// request and serves the two staging reads under /__e2e/, for
// ci/e2e/staging/. Unset, nothing below changes. See staging/mock-evidence.mjs.
import fs from 'node:fs';
import https from 'node:https';
import { exitWithPid } from './exit-with-pid.mjs';

exitWithPid();

const evidence =
  process.env.MOCK_EVIDENCE === '1'
    ? (await import('./staging/mock-evidence.mjs')).createEvidenceStore()
    : null;

const port = Number(process.env.MOCK_PORT || 8443);
const log = process.env.CAPTURE_LOG;
const options = {
  cert: fs.readFileSync(process.env.MOCK_CERT),
  key: fs.readFileSync(process.env.MOCK_KEY),
};

const server = https.createServer(options, (req, res) => {
  // The evidence reads are not gateway traffic, so they stay out of the log.
  if (evidence && evidence.handles(req)) {
    evidence.serve(req, res);
    return;
  }
  // Log on header arrival so we capture the request even if the client aborts
  // when it dislikes our canned response.
  fs.appendFileSync(
    log,
    JSON.stringify({ method: req.method, path: req.url, headers: req.headers }) + '\n',
  );
  const chunks = [];
  req.on('data', (c) => {
    if (evidence) chunks.push(c);
  });
  req.on('end', () => {
    if (evidence) evidence.record(req, Buffer.concat(chunks));
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end('{"ok":true}');
  });
});

// Diagnostics: a failed TLS handshake (e.g. the relay's rustls client rejecting
// our cert) never reaches the request handler, so surface it here or the only
// symptom is an opaque 502 on the relay side.
server.on('tlsClientError', (err) => {
  console.log(`tlsClientError: ${err.message}`);
});
server.on('secureConnection', (sock) => {
  console.log(`secureConnection: ${sock.getProtocol()} ${sock.authorized ? 'authorized' : 'unauthorized'}`);
});

server.listen(port, '127.0.0.1', () => {
  console.log(`mock gateway listening on https://127.0.0.1:${port}`);
});
