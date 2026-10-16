// The backend interface the checker talks to (AG-907). Two implementations:
//
//   mock     ./mock.mjs     works today, against ci/e2e/mock-gateway.mjs
//   staging  ./staging.mjs  stubbed until AG-1102 to AG-1105
//
// Every backend has:
//   name, target                         for messages and the summary
//   preflight()                          throws when the backend cannot be asked
//   getRequestByMarker(marker)           -> record[] (normalize.mjs); [] = not there yet
//   getOrgCounts({ orgId, from, to })    -> { orgId, messages, blockedOrFlagged, tokensSavedFraction }
import { ConfigError } from '../lib/outcome.mjs';
import { createMockBackend } from './mock.mjs';
import { createStagingBackend } from './staging.mjs';

export function createBackend(name, env = process.env, opts = {}) {
  if (name === 'mock') return createMockBackend(env, opts);
  if (name === 'staging') return createStagingBackend(env, opts);
  throw new ConfigError(`E2E_BACKEND must be mock or staging, not ${JSON.stringify(name)}`);
}
