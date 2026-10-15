// The run marker (AG-907): one value per run, platform and step, carried on
// every request the suite sends, so staging can be asked for exactly that
// request afterwards.
//
// Carrier: the `x-gate-fixture-nonce` request header, in the exact shape the
// gateway already accepts for it. Chosen because it is the one caller header
// the gateway both stores and keeps:
//
//   - the gateway copies every caller `x-gate-<name>` header into
//     `gateway_requests.custom_properties[<name>]` (gate
//     apps/gateway-proxy/src/utils/proxy-helpers.ts, extractCustomHeaders), but
//   - dashboard-api's content erasure (AG-1018) wipes every custom property
//     not on an allowlist, and that allowlist is the gateway-written keys plus
//     `fixture-nonce` alone (apps/dashboard-api/src/audit/retention/
//     row-local-cleartext.ts). A new header name would be stored and then
//     erased; this one survives with no gateway change.
//   - the gateway drops a `fixture-nonce` value that does not match
//     /^recon-[a-z0-9]{1,24}-[0-9a-f]{12}$/ (proxy-helpers.ts,
//     FIXTURE_NONCE_PATTERN), so free text cannot ride on it and our value
//     must fit it exactly. `makeMarker` guarantees that.
//
// Shape: recon-gc<p><step>-<12 hex>
//   p     platform: m (macOS), w (Windows), l (Linux)
//   step  the suite step, lowercase alphanumerics, cut to fit the 24-char label
//   hex   the first 12 hex of sha256(run id, attempt, platform, step, nonce),
//         so a rerun or a retried step never reuses a marker
//
// The marker alone does not spell the run id: the label has no room for it.
// The ledger (one JSON line per marker, written by whoever sends the request)
// maps each marker back to its run, attempt, platform and step.
//
// On Gate Connect's side the header must reach the gateway. The relay keeps
// caller `x-gate-*` headers on its rewrite arm (gate-connect-app
// crates/core/src/proxy/relay.rs, rewrite_headers), but strips every one on the
// Gate-served models route (same file, the `x-gate-*` filter before
// rewrite_headers on that arm). So a marked request must be one the relay
// forwards, not one Gate serves from its own model set.
import crypto from 'node:crypto';

export const MARKER_HEADER = 'x-gate-fixture-nonce';
/** The key the gateway stores it under in `custom_properties`. */
export const MARKER_PROPERTY = 'fixture-nonce';
/** Copied from the gateway; a marker that fails it is silently not stored. */
export const MARKER_PATTERN = /^recon-[a-z0-9]{1,24}-[0-9a-f]{12}$/;

const PLATFORM_CODES = { macos: 'm', windows: 'w', linux: 'l' };

/** `macOS`, `Windows`, `Linux` (RUNNER_OS) or the Node platform name, to ours. */
export function platformOf(name = process.platform) {
  const n = String(name).toLowerCase();
  if (n === 'darwin' || n === 'macos') return 'macos';
  if (n === 'win32' || n === 'windows') return 'windows';
  if (n === 'linux') return 'linux';
  throw new Error(`unknown platform ${JSON.stringify(name)}`);
}

export function makeMarker({ runId, attempt = '1', platform, step, nonce = crypto.randomBytes(8).toString('hex') }) {
  if (!runId) throw new Error('makeMarker: runId is required');
  const p = PLATFORM_CODES[platform];
  if (!p) throw new Error(`makeMarker: platform must be one of ${Object.keys(PLATFORM_CODES).join(', ')}`);
  const stepSlug = String(step || '').toLowerCase().replace(/[^a-z0-9]/g, '');
  if (!stepSlug) throw new Error('makeMarker: step must contain at least one letter or digit');
  // 24 is the gateway's limit on the label; 'gc' and the platform take 3.
  const label = `gc${p}${stepSlug}`.slice(0, 24);
  const hex = crypto
    .createHash('sha256')
    .update([runId, attempt, platform, step, nonce].join('\u0000'))
    .digest('hex')
    .slice(0, 12);
  const marker = `recon-${label}-${hex}`;
  if (!MARKER_PATTERN.test(marker)) throw new Error(`makeMarker built ${marker}, which the gateway would drop`);
  return marker;
}

/** One ledger line: everything needed to find and judge the request later. */
export function ledgerEntry({ marker, runId, attempt, platform, step, sentAt = new Date(), expect = {} }) {
  return {
    marker,
    header: MARKER_HEADER,
    runId: String(runId),
    attempt: String(attempt),
    platform,
    step,
    sentAt: new Date(sentAt).toISOString(),
    expect,
  };
}
