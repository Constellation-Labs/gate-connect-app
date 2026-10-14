// WebDriver driver for the installed Gate Connect bundle on Windows and Linux
// (AG-905), the counterpart of macos-ax.js.
//
// It talks plain W3C WebDriver over HTTP to tauri-driver, which launches the
// installed binary under the platform's native driver: msedgedriver (WebView2)
// on Windows, WebKitWebDriver (WebKitGTK) on Linux. No test hook in the app:
// tauri-driver turns automation on from the environment it launches the app
// with, which the release build honours as it is.
//
// One command per process, like macos-ax.js, so the shell script reads the
// same step by step. The session lives in $WD_SESSION (a JSON file), which
// `start` writes and every other command reads.
//
//   node webdriver.mjs <command> [args...]
//
//   start <app> [secs]          open a session that launches <app>
//   windows                     list the window handles with title and URL
//   dump                        print every window's text, buttons and fields
//   wait-text <secs> <text>     wait for a window whose page contains <text>
//   click <secs> <text>         click the button (or link) whose text is <text>
//   type <secs> <label> <text>  type <text> into the field labelled <label>
//   screenshot <file.png>       save the current window's page as a PNG
//   quit                        end the session (the driver closes the app)
//
// Exit 0 on success, 1 on a failed step (with the reason on stdout), 2 on
// usage. Matching is exact on a button's own text and substring elsewhere,
// as in macos-ax.js. Every search walks every window, since the first run
// opens the intro in its own window beside the hidden main one.
//
// Env: WD_URL (default http://127.0.0.1:4444), WD_SESSION (session file).
import fs from 'node:fs';

const WD_URL = process.env.WD_URL || 'http://127.0.0.1:4444';
const SESSION_FILE = process.env.WD_SESSION;
const W3C_ELEMENT = 'element-6066-11e4-a52f-4a6524fdcfa1';

function fail(msg) {
  console.log('FAIL: ' + msg);
  process.exit(1);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function wd(method, path, body) {
  const res = await fetch(WD_URL + path, {
    method,
    headers: { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  // An error the driver answered with, as against no driver to ask: `start`
  // retries the second and fails at once on the first.
  const answered = (msg) => Object.assign(new Error(msg), { answered: true });
  let json;
  try {
    json = JSON.parse(text);
  } catch {
    throw answered(`${method} ${path}: HTTP ${res.status}: ${text.slice(0, 500)}`);
  }
  if (!res.ok || (json.value && json.value.error)) {
    const v = json.value || {};
    throw answered(`${method} ${path}: ${v.error || res.status}: ${v.message || text.slice(0, 500)}`);
  }
  return json.value;
}

function session() {
  if (!SESSION_FILE || !fs.existsSync(SESSION_FILE)) fail('no session: run `start` first (WD_SESSION)');
  return JSON.parse(fs.readFileSync(SESSION_FILE, 'utf8')).id;
}

const s = (path) => `/session/${session()}${path}`;

// Runs in the page. Reads what a match can use: the visible text, and for
// controls their own text, aria-label, placeholder and <label>.
const SCAN = `
  const visible = (el) => {
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && cs.visibility !== 'hidden' && cs.display !== 'none';
  };
  const controls = (sel) => Array.from(document.querySelectorAll(sel)).filter(visible);
  const labelOf = (el) => [
    el.getAttribute('aria-label') || '',
    el.getAttribute('placeholder') || '',
    ...Array.from(el.labels || []).map((l) => l.textContent || ''),
  ].map((t) => t.trim()).filter(Boolean);
`;

async function handles() {
  return wd('GET', s('/window/handles'));
}

async function inEachWindow(fn) {
  for (const h of await handles()) {
    try {
      await wd('POST', s('/window'), { handle: h });
    } catch {
      continue; // a window closed between the list and the switch
    }
    const hit = await fn(h);
    if (hit) return hit;
  }
  return null;
}

async function exec(script, args = []) {
  return wd('POST', s('/execute/sync'), { script, args });
}

async function poll(secs, find) {
  const until = Date.now() + secs * 1000;
  let last = '';
  for (;;) {
    try {
      const hit = await inEachWindow(find);
      if (hit) return hit;
    } catch (e) {
      // The app is still starting, or a window is mid-navigation. Keep the
      // reason for the failure message.
      last = String(e.message || e);
    }
    if (Date.now() > until) return { miss: last };
    await sleep(500);
  }
}

async function main(argv) {
  const [cmd, ...args] = argv;
  switch (cmd) {
    case 'start': {
      const [app, secs = '120'] = args;
      if (!app) fail('start needs the app binary');
      const until = Date.now() + Number(secs) * 1000;
      let last;
      for (;;) {
        try {
          const v = await wd('POST', '/session', {
            capabilities: { alwaysMatch: { 'tauri:options': { application: app } } },
          });
          fs.writeFileSync(SESSION_FILE, JSON.stringify({ id: v.sessionId }));
          console.log(`session ${v.sessionId}`);
          console.log(JSON.stringify(v.capabilities, null, 2));
          return;
        } catch (e) {
          last = e;
          // The driver is up and said no: a native driver that cannot
          // attach, or an app that would not start. Waiting will not change it.
          if (e.answered) fail(`session not created: ${e.message}`);
          if (Date.now() > until) fail(`no session within ${secs}s: ${last.message || last}`);
          await sleep(2000);
        }
      }
    }
    case 'windows': {
      for (const h of await handles()) {
        await wd('POST', s('/window'), { handle: h });
        const title = await wd('GET', s('/title')).catch((e) => `? ${e.message}`);
        const url = await wd('GET', s('/url')).catch((e) => `? ${e.message}`);
        console.log(`${h}  ${JSON.stringify(title)}  ${url}`);
      }
      return;
    }
    case 'dump': {
      for (const h of await handles()) {
        await wd('POST', s('/window'), { handle: h });
        const page = await exec(`${SCAN}
          return {
            title: document.title,
            url: location.href,
            text: (document.body && document.body.innerText) || '',
            buttons: controls('button, a, [role=button]').map((b) => [b.textContent.trim(), ...labelOf(b)].join(' | ')),
            fields: controls('input, textarea').map((i) => labelOf(i).join(' | ') + ' = ' + JSON.stringify(i.type === 'password' ? '(hidden)' : i.value)),
          };`).catch((e) => ({ error: e.message }));
        console.log(`=== window ${h}`);
        console.log(JSON.stringify(page, null, 2));
      }
      return;
    }
    case 'wait-text': {
      const [secs, text] = args;
      const hit = await poll(Number(secs), async (h) => {
        const found = await exec(
          `${SCAN}
          const want = arguments[0];
          if (((document.body && document.body.innerText) || '').includes(want)) return true;
          return controls('input, textarea, button, [aria-label]').some((el) => labelOf(el).some((t) => t.includes(want)));`,
          [text],
        );
        return found ? { handle: h } : null;
      });
      if (!hit || hit.miss !== undefined) fail(`no window reading "${text}" within ${secs}s ${hit && hit.miss ? '(' + hit.miss + ')' : ''}`);
      console.log(`found "${text}" in window ${hit.handle}`);
      return;
    }
    case 'click': {
      const [secs, text] = args;
      const hit = await poll(Number(secs), async (h) => {
        const el = await exec(
          `${SCAN}
          const want = arguments[0];
          return controls('button, a, [role=button]').find((b) =>
            [(b.textContent || '').trim(), ...labelOf(b)].some((t) => t === want) && !b.disabled) || null;`,
          [text],
        );
        return el ? { handle: h, el } : null;
      });
      if (!hit || hit.miss !== undefined) fail(`no button "${text}" within ${secs}s ${hit && hit.miss ? '(' + hit.miss + ')' : ''}`);
      // Element Click: the driver scrolls it into view and clicks its centre
      // with a pointer, rather than a synthetic click() from the page.
      await wd('POST', s(`/element/${hit.el[W3C_ELEMENT]}/click`), {});
      console.log(`clicked "${text}" in window ${hit.handle}`);
      return;
    }
    case 'type': {
      const [secs, label, text] = args;
      const hit = await poll(Number(secs), async (h) => {
        const el = await exec(
          `${SCAN}
          const want = arguments[0];
          return controls('input, textarea').find((i) => labelOf(i).some((t) => t.includes(want))) || null;`,
          [label],
        );
        return el ? { handle: h, el } : null;
      });
      if (!hit || hit.miss !== undefined) fail(`no field labelled "${label}" within ${secs}s ${hit && hit.miss ? '(' + hit.miss + ')' : ''}`);
      const id = hit.el[W3C_ELEMENT];
      await wd('POST', s(`/element/${id}/click`), {});
      // Element Send Keys: key events through the driver, so React sees the
      // input events it sees from a person. Setting .value would skip them.
      await wd('POST', s(`/element/${id}/value`), { text });
      const value = await wd('GET', s(`/element/${id}/property/value`));
      if (value !== text) fail(`field "${label}" reads ${value.length} characters after typing ${text.length}`);
      console.log(`typed ${text.length} characters into "${label}" in window ${hit.handle}`);
      return;
    }
    case 'screenshot': {
      const [file] = args;
      const png = await wd('GET', s('/screenshot'));
      fs.writeFileSync(file, Buffer.from(png, 'base64'));
      console.log(`saved ${file}`);
      return;
    }
    case 'quit': {
      await wd('DELETE', s(''));
      console.log('session ended');
      return;
    }
    default:
      console.log('usage: webdriver.mjs start|windows|dump|wait-text|click|type|screenshot|quit ...');
      process.exit(2);
  }
}

main(process.argv.slice(2)).catch((e) => fail(e.message || String(e)));
