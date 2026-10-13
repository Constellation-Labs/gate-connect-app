// macOS accessibility driver for the installed Gate Connect bundle (AG-905).
//
// JXA over System Events, so it needs nothing but the OS: no WebDriver (WKWebView
// has none), no test hook in the app, and the bundle it drives is the one a user
// installs. WKWebView publishes the page's accessibility tree to the OS, so a
// <button> is an AXButton named by its text and an <input> is an AXTextField
// labelled by its <label>, which is the same reading the browser specs assert on.
//
//   osascript -l JavaScript macos-ax.js <command> [args...]
//
//   probe                      is this process trusted for accessibility?
//   wait-window <secs>         wait for a window of the app to exist
//   dump                       print the app's accessibility tree
//   wait-text <secs> <text>    wait for an element whose text contains <text>
//   click <secs> <text>        AXPress the button (or link) whose text is <text>
//   type <secs> <label> <text> focus the field labelled <label> and type <text>
//
// Exit 0 on success, 1 on a failed step (with the reason on stderr), 2 on
// usage. Matching is exact on a button's own text and substring elsewhere.

ObjC.import("stdlib");

const BUNDLE_ID = "ai.constellation.gate-connect";
const se = Application("System Events");

function fail(msg) {
  console.log("FAIL: " + msg);
  $.exit(1);
}

function proc() {
  const p = se.processes.whose({ bundleIdentifier: BUNDLE_ID });
  return p.length > 0 ? p[0] : null;
}

function safe(f) {
  try {
    const v = f();
    return v === null || v === undefined ? "" : String(v);
  } catch (e) {
    return "";
  }
}

// One node per element, with every attribute a match might use. Children are
// read one level at a time; the tree is a few hundred elements, so this is a
// second or two, which is fine for a step that waits anyway.
function walk(el, depth, out) {
  if (depth > 40) return;
  const kids = (() => {
    try {
      return el.uiElements();
    } catch (e) {
      return [];
    }
  })();
  for (const k of kids) {
    const node = {
      el: k,
      depth,
      role: safe(() => k.role()),
      name: safe(() => k.name()),
      desc: safe(() => k.description()),
      value: safe(() => k.value()),
      title: safe(() => k.title()),
      placeholder: safe(() => k.attributes.byName("AXPlaceholderValue").value()),
    };
    out.push(node);
    walk(k, depth + 1, out);
  }
}

function tree() {
  const p = proc();
  if (!p) return [];
  const out = [];
  for (const w of p.windows()) {
    out.push({
      el: w,
      depth: 0,
      role: "AXWindow",
      name: safe(() => w.name()),
      desc: "",
      value: "",
      title: "",
      placeholder: "",
    });
    walk(w, 1, out);
  }
  return out;
}

function texts(n) {
  return [n.name, n.desc, n.value, n.title, n.placeholder].filter((s) => s.length > 0);
}

function poll(secs, find) {
  const until = Date.now() + secs * 1000;
  for (;;) {
    const hit = find(tree());
    if (hit) return hit;
    if (Date.now() > until) return null;
    delay(0.5);
  }
}

function describeNode(n) {
  return `${"  ".repeat(n.depth)}${n.role} ${JSON.stringify({
    name: n.name,
    desc: n.desc,
    value: n.value.length > 80 ? n.value.slice(0, 80) + "..." : n.value,
    title: n.title,
    placeholder: n.placeholder,
  })}`;
}

function run(argv) {
  const cmd = argv[0];
  switch (cmd) {
    case "probe": {
      // AXIsProcessTrusted answers for the process asking, which is this
      // osascript: the one whose events reach the app.
      ObjC.import("ApplicationServices");
      const trusted = $.AXIsProcessTrusted();
      console.log("AXIsProcessTrusted: " + trusted);
      try {
        console.log("System Events UI scripting enabled: " + se.uiElementsEnabled());
      } catch (e) {
        console.log("System Events UI scripting: " + e);
      }
      return;
    }
    case "wait-window": {
      const secs = Number(argv[1] || 60);
      const until = Date.now() + secs * 1000;
      for (;;) {
        const p = proc();
        if (p && p.windows().length > 0) {
          console.log(`windows: ${p.windows().map((w) => safe(() => w.name())).join(", ")}`);
          return;
        }
        if (Date.now() > until) fail(`no Gate Connect window within ${secs}s`);
        delay(0.5);
      }
    }
    case "dump": {
      for (const n of tree()) console.log(describeNode(n));
      return;
    }
    case "wait-text": {
      const [, secs, text] = argv;
      const hit = poll(Number(secs), (t) => t.find((n) => texts(n).some((s) => s.includes(text))));
      if (!hit) fail(`no element reading "${text}" within ${secs}s`);
      console.log("found: " + describeNode(hit).trim());
      return;
    }
    case "click": {
      const [, secs, text] = argv;
      const hit = poll(Number(secs), (t) =>
        t.find(
          (n) =>
            (n.role === "AXButton" || n.role === "AXLink") &&
            texts(n).some((s) => s.trim() === text),
        ),
      );
      if (!hit) fail(`no button "${text}" within ${secs}s`);
      hit.el.actions.byName("AXPress").perform();
      console.log("pressed: " + describeNode(hit).trim());
      return;
    }
    case "type": {
      const [, secs, label, text] = argv;
      const hit = poll(Number(secs), (t) =>
        t.find(
          (n) => n.role === "AXTextField" && texts(n).some((s) => s.includes(label)),
        ),
      );
      if (!hit) fail(`no text field labelled "${label}" within ${secs}s`);
      const p = proc();
      p.frontmost = true;
      hit.el.focused = true;
      delay(0.3);
      // Real key events through the window server, so React sees input events
      // exactly as it does from a person. Setting AXValue would bypass them.
      se.keystroke(text);
      console.log("typed into: " + describeNode(hit).trim());
      return;
    }
    default:
      console.log("usage: macos-ax.js probe|wait-window|dump|wait-text|click|type ...");
      $.exit(2);
  }
}
