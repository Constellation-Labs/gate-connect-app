// One JSON GET, with a deadline, through the host guard. Errors come out as the
// classes outcome.mjs sorts, so a caller never inspects a status itself.
import { assertTarget } from './host-guard.mjs';
import { BadResponseError, HttpError } from './outcome.mjs';

export async function getJson(url, { backend, headers = {}, timeoutMs = 15000, fetchImpl = fetch } = {}) {
  const u = assertTarget(url, backend);
  const res = await fetchImpl(u, {
    method: 'GET',
    headers: { accept: 'application/json', ...headers },
    signal: AbortSignal.timeout(timeoutMs),
    redirect: 'error',
  });
  const text = await res.text();
  if (!res.ok) throw new HttpError(res.status, u.origin + u.pathname, text);
  try {
    return JSON.parse(text);
  } catch {
    throw new BadResponseError(u.origin + u.pathname, `not JSON: ${text.slice(0, 120)}`);
  }
}
