// The one shape both backends hand to the checker, and the reads that turn a
// backend's JSON into it. The staging reads (AG-1104, AG-1105) are coded
// against the same JSON the mock serves, so once staging answers in that shape
// these run unchanged; if it answers in another, only these two functions move.
//
// AG-1104, one request by its marker. Proposed response:
//
//   { "requests": [ {
//       "requestId":      "<gateway_requests.request_id>",
//       "createdAt":      "<ISO 8601>",
//       "orgId":          "<gateway_requests.org_id>",
//       "userId":         "<gateway_requests.user_id>",
//       "clientTool":     "<gateway_requests.client_tool>",   (x-gate-client)
//       "machineId":      "<gateway_requests.machine_id>",    (x-gate-install-id)
//       "agentFramework": "<gateway_requests.agent_framework>", (harness detection)
//       "verdict":        { "action": "allow"|"block"|"flag"|"redact" } | null
//   } ] }
//
//   An empty list means "not there (yet)", and the checker keeps polling.
//
// AG-1105, an org's counts over a window. Proposed response:
//
//   { "orgId": "...", "window": { "from": "<ISO>", "to": "<ISO>" },
//     "messages": <int>,            (activity.repository countRequests)
//     "blockedOrFlagged": <int>,    (countEnforcementActions: block, flag, redact)
//     "tokensSaved": { "fraction": <0..1> } | null }   (tokensSaved)
import { BadResponseError } from '../lib/outcome.mjs';

const str = (v) => (typeof v === 'string' && v.length > 0 ? v : null);

export function normalizeRequests(json, url) {
  if (!json || !Array.isArray(json.requests)) throw new BadResponseError(url, 'no "requests" array');
  return json.requests.map((r) => ({
    requestId: str(r.requestId),
    createdAt: str(r.createdAt),
    orgId: str(r.orgId),
    userId: str(r.userId),
    clientTool: str(r.clientTool),
    machineId: str(r.machineId),
    harness: str(r.agentFramework),
    verdict: r.verdict && typeof r.verdict === 'object' ? { action: str(r.verdict.action) } : null,
  }));
}

export function normalizeCounts(json, url) {
  const int = (v, name) => {
    if (!Number.isInteger(v) || v < 0) throw new BadResponseError(url, `"${name}" is ${JSON.stringify(v)}`);
    return v;
  };
  if (!json || typeof json !== 'object') throw new BadResponseError(url, 'not an object');
  const saved = json.tokensSaved;
  let fraction = null;
  if (saved !== null && saved !== undefined) {
    if (typeof saved.fraction !== 'number' || saved.fraction < 0 || saved.fraction > 1) {
      throw new BadResponseError(url, `"tokensSaved.fraction" is ${JSON.stringify(saved.fraction)}`);
    }
    fraction = saved.fraction;
  }
  return {
    orgId: str(json.orgId),
    messages: int(json.messages, 'messages'),
    blockedOrFlagged: int(json.blockedOrFlagged, 'blockedOrFlagged'),
    tokensSavedFraction: fraction,
  };
}
