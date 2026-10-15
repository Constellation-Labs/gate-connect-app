// AC6: nothing in this suite may touch a production tenant. Every URL a backend
// calls goes through `assertTarget` first, which allows exactly the hosts named
// here for that backend and refuses everything else, before any socket opens.
//
// An allowlist, not a production blocklist: a new production hostname must not
// become reachable because nobody added it here.
import { ConfigError } from './outcome.mjs';

/** The staging gateway (gate-connect-app src/lib/config.ts lists it as
 *  "Staging"), and dashboard-api's staging host, where AG-1104 or AG-1105 may
 *  put their reads. Nothing else. */
export const STAGING_HOSTS = Object.freeze(['gateway-staging.constellationgate.ai', 'api-staging.constellationgate.ai']);

/** The mock backend runs on the runner's own loopback. */
export const MOCK_HOSTS = Object.freeze(['127.0.0.1', 'localhost', '[::1]']);

const ALLOWED = { staging: STAGING_HOSTS, mock: MOCK_HOSTS };

/**
 * Return the parsed URL when `url` is one `backend` may call, and throw a
 * ConfigError naming the host otherwise. Staging must be https, so a
 * downgrade cannot be configured in by accident.
 */
export function assertTarget(url, backend) {
  const allowed = ALLOWED[backend];
  if (!allowed) throw new ConfigError(`unknown backend ${JSON.stringify(backend)}`);
  let u;
  try {
    u = new URL(url);
  } catch {
    throw new ConfigError(`${backend} target ${JSON.stringify(url)} is not a URL`);
  }
  const host = u.hostname.toLowerCase();
  if (!allowed.includes(host)) {
    const prod = backend === 'staging' && /(^|\.)constellationgate\.ai$/.test(host);
    throw new ConfigError(
      `refusing ${u.origin}: the ${backend} backend may only call ${allowed.join(', ')}` +
        (prod ? ' (this looks like a production host; AC6 forbids touching a production tenant)' : ''),
    );
  }
  if (backend === 'staging' && u.protocol !== 'https:') {
    throw new ConfigError(`refusing ${u.origin}: staging is https only`);
  }
  if (u.username || u.password) throw new ConfigError(`refusing ${u.host}: credentials in the URL`);
  return u;
}
