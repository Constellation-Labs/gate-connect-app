// The counts comparison (AG-907 AC7): what Connect's Overview displays for its
// window, against what the backend counts for the same org and window.
//
// Connect shows three tiles (src/components/gc/metrics.tsx, StatTiles):
//   Messages                  requestsRouted, toLocaleString()
//   Blocked/Flagged/Redacted  blockedOrFlagged
//   Tokens saved              round(tokensSaved.fraction * 100) + '%'
// and `N/A` for a counter with no reading. Its window is the gateway's
// /v1/me/activity window: from the top of the hour 23 hours before the read,
// to the read itself (gate apps/gateway-proxy/src/activity/activity.controller.ts).
//
// Displayed values come in one of two forms:
//   - JSON { messages, blockedFlagged, tokensSavedPercent, readAt }, numbers or
//     null for N/A, which a driver hook writes after reading the pane;
//   - a driver page dump (webdriver.mjs `dump`, or any plain text), from which
//     the tiles are read by their labels.
import { ConfigError } from './outcome.mjs';

const TILES = [
  ['messages', 'Messages'],
  ['blockedFlagged', 'Blocked/Flagged/Redacted'],
  ['tokensSavedPercent', 'Tokens saved'],
];

/** Connect's window for a read at `readAt`. */
export function windowFor(readAt) {
  const to = new Date(readAt);
  if (Number.isNaN(to.getTime())) throw new ConfigError(`readAt ${JSON.stringify(readAt)} is not a time`);
  const hour = new Date(to);
  hour.setUTCMinutes(0, 0, 0);
  const from = new Date(hour.getTime() - 23 * 3600 * 1000);
  return { from: from.toISOString(), to: to.toISOString() };
}

function readFigure(text) {
  const t = String(text).trim();
  if (/^n\/?a$/i.test(t)) return null;
  const digits = t.replace(/[%\s,.  ']/g, '');
  if (!/^\d+$/.test(digits)) return undefined;
  return Number(digits);
}

/** The page text out of a webdriver.mjs dump, or the input as is. */
function pageText(raw) {
  if (!raw.includes('=== window')) return raw;
  const texts = [];
  for (const block of raw.split(/^=== window .*$/m)) {
    const json = block.trim();
    if (!json) continue;
    try {
      const page = JSON.parse(json);
      if (page && typeof page.text === 'string') texts.push(page.text);
    } catch {
      texts.push(json);
    }
  }
  return texts.join('\n');
}

/**
 * Read the three tiles out of page text: each label (matched without regard to
 * case, since the tile label is CSS-uppercased and innerText reports it so),
 * then the next non-empty line as its figure. A tile not on the page, or a
 * figure that is not a number or N/A, is a config error: the read failed, and
 * comparing it would prove nothing.
 */
export function parseDisplayed(raw, readAt) {
  const trimmed = String(raw).trim();
  if (trimmed.startsWith('{')) {
    const j = JSON.parse(trimmed);
    for (const [key] of TILES) {
      if (!(key in j)) throw new ConfigError(`displayed JSON has no ${key}`);
      if (j[key] !== null && typeof j[key] !== 'number') throw new ConfigError(`displayed ${key} is ${JSON.stringify(j[key])}`);
    }
    return { messages: j.messages, blockedFlagged: j.blockedFlagged, tokensSavedPercent: j.tokensSavedPercent, readAt: j.readAt ?? readAt };
  }
  const lines = pageText(trimmed).split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
  const out = { readAt };
  for (const [key, label] of TILES) {
    const i = lines.findIndex((l) => l.toLowerCase() === label.toLowerCase());
    if (i < 0 || i + 1 >= lines.length) throw new ConfigError(`the "${label}" tile is not on the page`);
    const v = readFigure(lines[i + 1]);
    if (v === undefined) throw new ConfigError(`the "${label}" tile reads ${JSON.stringify(lines[i + 1])}, not a number or N/A`);
    out[key] = v;
  }
  return out;
}

/** What Connect would print for a backend reading, in the tiles' own terms. */
export function asDisplayed(counts) {
  return {
    messages: counts.messages,
    blockedFlagged: counts.blockedOrFlagged,
    tokensSavedPercent: counts.tokensSavedFraction === null ? null : Math.round(counts.tokensSavedFraction * 100),
  };
}

/** Every tile that disagrees, with both values (AC7). */
export function compareCounts(displayed, counts) {
  const want = asDisplayed(counts);
  const show = (v) => (v === null ? 'N/A' : String(v));
  const problems = [];
  for (const [key, label] of TILES) {
    if (displayed[key] !== want[key]) {
      problems.push(`${label}: Connect displays ${show(displayed[key])}, the backend counts ${show(want[key])}`);
    }
  }
  return problems;
}
