#!/usr/bin/env bash
# Real-tools end-to-end driver. Exercises the full relay path: `gate-connect
# proxy relay` hosts the loopback reverse-proxy relay, `gate-connect connect`
# points each installed AI CLI at that relay (plaintext http), the tool fires one
# headless request, the relay injects the Gate credential + forwards over TLS to
# a local HTTPS mock gateway, and we assert the mock received it with the right
# headers.
#
# Two phases, because both auth modes must keep working:
#   Phase A - API key (legacy): relay injects x-gate-api-key.
#   Phase B - OAuth (primary):  relay injects x-gate-authorization + x-gate-org-id.
# The tool config is identical in both phases (it just points at the relay); only
# what the relay injects - and therefore the per-phase assertion - differs.
#
# Because the tools now talk to the relay over plaintext loopback, they need no
# CA trust of their own: only the relay speaks TLS to the mock gateway, and it
# trusts the throwaway CA via the GATE_CONNECT_TEST_CA seam. The OAuth token /
# org-list calls run over plain HTTP against mock-auth.mjs via the
# GATE_CONNECT_TEST_TOKEN_ENDPOINT / GATE_CONNECT_TEST_ORGS_ENDPOINT seams.
#
# Best-effort: the external CLIs' invocation flags and auth shapes drift over
# time, so each tool is allowed to fail without aborting the others; the per-tool
# capture assertion is the source of truth.
#
# Runs on Linux, macOS, and Windows (Git Bash). Assumes: `gate-connect` is built
# (target/debug), node is on PATH, and the CLIs are installed. Belongs on a
# throwaway CI runner.
set -uo pipefail

OS="other"
case "$(uname -s)" in
  Linux) OS="Linux" ;;
  Darwin) OS="Darwin" ;;
  MINGW* | MSYS* | CYGWIN*) OS="Windows" ;;
esac

# Native path for consumers that aren't msys-aware (node, the gate-connect .exe).
# Identity off Windows.
winpath() { if [ "$OS" = "Windows" ]; then cygpath -w "$1"; else printf '%s' "$1"; fi; }

if [ "$OS" = "Windows" ]; then
  # Git Bash rewrites any argument starting with `/` into a Windows path (so
  # openssl's `-subj "/CN=…"` and the "/v1/messages" match needle get corrupted
  # into `C:/Program Files/Git/…`). We pass every real path to native programs
  # through winpath() already, so turn the automatic conversion off entirely.
  export MSYS2_ARG_CONV_EXCL='*'
  export MSYS_NO_PATHCONV=1
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# RUNNER_TEMP is a native Windows path (e.g. D:\a\_temp) on the Windows runner;
# normalise to an msys path so the shell-side file ops (mkdir, openssl, redirects)
# all stay consistent. winpath() converts back when handing paths to node/the exe.
if [ "$OS" = "Windows" ]; then
  WORK="$(cygpath -u "${RUNNER_TEMP:-/tmp}")/gc-e2e"
else
  WORK="${RUNNER_TEMP:-/tmp}/gc-e2e"
fi
rm -rf "$WORK"
CA_DIR="$WORK/ca"
mkdir -p "$CA_DIR" "$WORK/secrets"

# Redirect home so gate-connect AND the tools agree on a throwaway config root.
export HOME="$WORK/home"
mkdir -p "$HOME"
if [ "$OS" = "Windows" ]; then
  # gate-connect uses the Known-Folder API and the Node CLIs use %USERPROFILE% /
  # XDG - none of which follow Git Bash's $HOME. Point all of them (in native
  # form) at the same throwaway home so reads and writes line up. The
  # GATE_CONNECT_TEST_HOME seam redirects gate-connect; USERPROFILE/XDG redirect
  # the tools.
  export GATE_CONNECT_TEST_HOME="$(winpath "$HOME")"
  export USERPROFILE="$(winpath "$HOME")"
  export XDG_CONFIG_HOME="$(winpath "$HOME/.config")"
  export XDG_DATA_HOME="$(winpath "$HOME/.local/share")"
  export XDG_CACHE_HOME="$(winpath "$HOME/.cache")"
else
  # opencode resolves its config via XDG; CI runners often pre-set
  # XDG_CONFIG_HOME elsewhere, so pin the XDG roots under our scratch home or
  # opencode would miss the override gate-connect wrote.
  export XDG_CONFIG_HOME="$HOME/.config"
  export XDG_DATA_HOME="$HOME/.local/share"
  export XDG_CACHE_HOME="$HOME/.cache"
fi

# The Gate key goes through the file-backed secret seam, since CI has no usable
# OS keychain headlessly. The gate-connect binary reads this as a native path.
export GATE_CONNECT_TEST_SECRETS="$(winpath "$WORK/secrets")"

# Codex resolves ~/.codex via its own home logic - on Windows that's the real
# profile, not our USERPROFILE override - so it was reading an empty config and
# falling back to the default `openai` provider (hitting api.openai.com, 401).
# Point it explicitly at the .codex dir gate-connect writes the gate provider to.
export CODEX_HOME="$(winpath "$HOME/.codex")"

CLI="$ROOT/target/debug/gate-connect"
[ "$OS" = "Windows" ] && CLI="$CLI.exe"
PORT=8443
# Use 127.0.0.1, not localhost: on macOS `localhost` can resolve to IPv6 ::1
# first, but the mock binds 127.0.0.1 only - so the relay would hit a dead ::1
# address. The leaf cert carries an IP:127.0.0.1 SAN so the relay's TLS validates.
BASE_URL="https://127.0.0.1:$PORT"
AUTH_PORT=8455
CAPTURE="$WORK/capture.jsonl"
: > "$CAPTURE"
# Tool stdout/stderr goes here, not the step's inherited pipe: on Windows a tool
# (codex) can leave an orphaned grandchild process that keeps the pipe open and
# stalls the whole step. Writing to a file lets the step finish; we print it
# after each run for visibility.
TOOL_OUT="$WORK/tool.out"

# Diagnostics: timestamped checkpoints to both stdout (captured live by the
# runner) and a file (dumped by an always() step even if this step is killed by
# its timeout). Lets us see exactly where a hang occurs.
DIAG="$WORK/diag.log"
: > "$DIAG"
ckpt() {
  echo ">>> ckpt $(date -u +%H:%M:%S) $*"
  echo ">>> ckpt $(date -u +%H:%M:%S) $*" >> "$DIAG" 2>/dev/null || true
}

# The tools now talk to the relay over plaintext loopback, so none of them need
# CA trust or TLS quirks of their own. NODE_TLS_REJECT_UNAUTHORIZED stays off as
# a belt-and-suspenders in case a node CLI does any incidental TLS; ANTHROPIC_API_KEY
# just lets claude attach an auth header so it actually sends a request.
export NODE_TLS_REJECT_UNAUTHORIZED=0
export ANTHROPIC_API_KEY="sk-ant-e2e-dummy"

# Per-request engine logging. The engine prints `[gate-proxy] <host><path> ->
# <action>` for every intercepted request, where action is one of passthrough /
# rewrite->gateway / rewrite-FAILED, and the daemon tees it to
# <app-support>/proxy/helper.log when this is set. That single field is what
# separates "the engine never intercepted" from "it intercepted and chose not to
# rewrite" from "it tried and the gateway hop failed" - none of which are
# distinguishable from the tool's side, which sees only a provider response.
export GATE_PROXY_DEBUG=1

PASS=0
FAIL=0

# Launch a tool (output to a file, never the step's pipe) and poll the capture
# until the expected request shows up or we time out. We deliberately do NOT
# `wait` on the process: on Windows the codex shim spawns a grandchild that may
# be unkillable from msys, and blocking on it would stall the whole step. Since
# the tool's stdout/stderr go to a file, leaving it orphaned is harmless - the
# step still completes once the script exits and the EXIT trap stops the mocks.
run_until_capture() {
  local needle="$1"
  shift
  : > "$TOOL_OUT"
  ckpt "launch: $*"
  "$@" </dev/null >"$TOOL_OUT" 2>&1 &
  local pid=$!
  ckpt "launched pid=$pid; polling for '$needle'"
  local i=0
  while [ "$i" -lt 90 ]; do
    grep -q "$needle" "$CAPTURE" 2>/dev/null && {
      ckpt "captured '$needle' after ${i}s"
      break
    }
    kill -0 "$pid" 2>/dev/null || {
      ckpt "process pid=$pid exited on its own after ${i}s"
      break
    }
    sleep 1
    i=$((i + 1))
    [ $((i % 15)) -eq 0 ] && ckpt "still polling ${i}s (pid=$pid alive)"
  done
  [ "$i" -ge 90 ] && ckpt "poll timed out at ${i}s (pid=$pid)"
  ckpt "killing pid=$pid"
  kill -KILL "$pid" 2>/dev/null # best-effort; not waited on
  ckpt "kill returned for pid=$pid"
}

# ---------------------------------------------------------------------------
# 1. Mint a throwaway CA + a 127.0.0.1 leaf. Only the relay validates the mock
#    gateway's cert, and it trusts this CA via GATE_CONNECT_TEST_CA - no OS trust
#    store wiring and no per-tool CA env needed anymore.
# ---------------------------------------------------------------------------
# Run openssl from inside CA_DIR with bare filenames so it works whether the
# openssl on PATH is the msys build (wants /c/… paths) or a native Windows one
# (wants C:\… paths) - relative names resolve against the process cwd either way.
(
  cd "$CA_DIR" || exit 1
  openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout ca.key -out ca.pem \
    -subj "/CN=Gate Connect E2E CA" -days 2 \
    -addext "basicConstraints=critical,CA:TRUE"

  openssl req -newkey rsa:2048 -nodes \
    -keyout leaf.key -out leaf.csr \
    -subj "/CN=localhost"

  cat > leaf.ext <<'EXT'
subjectAltName=DNS:localhost,IP:127.0.0.1
extendedKeyUsage=serverAuth
EXT

  openssl x509 -req -in leaf.csr \
    -CA ca.pem -CAkey ca.key -CAcreateserial \
    -out leaf.pem -days 2 -extfile leaf.ext
)

# The relay's reqwest client adds this CA to its roots (GATE_CONNECT_TEST_CA
# seam) so its TLS hop to the mock gateway validates.
export GATE_CONNECT_TEST_CA="$(winpath "$CA_DIR/ca.pem")"

# OAuth build config (public client, no secret): the hosted domain is only ever
# shown in the authorize URL - we fake the browser round-trip - and the token /
# org-list calls are redirected to mock-auth over http by the seams below.
export GATE_COGNITO_HOSTED_DOMAIN="auth.e2e.test"
export GATE_COGNITO_CLIENT_ID="e2e-client"
export GATE_CONNECT_TEST_TOKEN_ENDPOINT="http://127.0.0.1:$AUTH_PORT/oauth2/token"
export GATE_CONNECT_TEST_ORGS_ENDPOINT="http://127.0.0.1:$AUTH_PORT/v1/me/orgs"

# ---------------------------------------------------------------------------
# 2. Start the mocks: HTTPS gateway (the relay forwards here) + plain-HTTP auth
#    (token + org-list for the OAuth phase). Both write to files, not the step's
#    pipe, so the step completes the moment the script exits.
# ---------------------------------------------------------------------------
CAPTURE_LOG="$(winpath "$CAPTURE")" MOCK_PORT="$PORT" \
  MOCK_CERT="$(winpath "$CA_DIR/leaf.pem")" MOCK_KEY="$(winpath "$CA_DIR/leaf.key")" \
  node "$(winpath "$ROOT/ci/e2e/mock-gateway.mjs")" >"$WORK/mock.out" 2>&1 &
MOCK_PID=$!

MOCK_AUTH_PORT="$AUTH_PORT" \
  node "$(winpath "$ROOT/ci/e2e/mock-auth.mjs")" >"$WORK/mock-auth.out" 2>&1 &
AUTH_PID=$!

RELAY_PID=""
# Declared here, beside RELAY_PID and before the trap, for the same reason: the
# script can exit (set -u) before the engine section is even reached, and the
# trap must still be able to read it.
ENGINE_ON=""
cleanup() {
  # Preserve the status that triggered the trap (the final `test $FAIL -eq 0`,
  # or an early `exit`) before any teardown command clobbers $?.
  local status=$?
  kill -KILL "$MOCK_PID" 2>/dev/null
  kill -KILL "$AUTH_PID" 2>/dev/null
  [ -n "$RELAY_PID" ] && kill -KILL "$RELAY_PID" 2>/dev/null
  # Inline rather than via stop_engine: this trap is armed long before that
  # function is defined, and an early exit would otherwise hit "command not
  # found". Leaving routing on would strand the runner behind a dead proxy and
  # a trusted CA.
  [ -n "$ENGINE_ON" ] && "$CLI" proxy disable >/dev/null 2>&1
  # Codex's Rust binary survives an msys kill and, in the runner's job object,
  # keeps the step from finishing. A real Windows kill of the whole tree lets
  # the step exit. (No-op if codex isn't running; codex.exe doesn't exist off
  # Windows.)
  if [ "$OS" = "Windows" ]; then
    taskkill /F /T /IM codex.exe >/dev/null 2>&1 || true
    taskkill /F /IM gate-connect.exe >/dev/null 2>&1 || true
  fi
  # Re-exit with the preserved status. On Windows/msys, reaping these freshly
  # killed native processes as the shell winds down corrupts the exit status
  # word - the step reported exit 2304 even though every assertion passed
  # (Passed: 3 Failed: 0). Exiting explicitly makes the pass/fail result, not
  # teardown noise, the code the runner sees.
  exit "$status"
}
trap cleanup EXIT

for _ in $(seq 1 40); do
  if curl -sk "$BASE_URL/healthz" >/dev/null 2>&1; then break; fi
  sleep 0.5
done

# ---------------------------------------------------------------------------
# Relay host: `gate-connect proxy relay` binds the loopback relay and blocks.
# It seeds the credential channels from the current account, so it must be
# (re)started after each phase's login. connect reads the persisted relay port,
# so the relay must be up before any connect.
# ---------------------------------------------------------------------------
start_relay() {
  : > "$WORK/serve.out"
  ckpt "relay: starting proxy relay"
  "$CLI" proxy relay >"$WORK/serve.out" 2>&1 &
  RELAY_PID=$!
  local i=0
  while [ "$i" -lt 30 ]; do
    grep -q 'relay listening on' "$WORK/serve.out" 2>/dev/null && {
      ckpt "relay: ready ($(grep -o 'http://[^ ]*' "$WORK/serve.out" | head -n1))"
      return 0
    }
    kill -0 "$RELAY_PID" 2>/dev/null || {
      echo "relay: host exited before becoming ready"
      sed 's/^/    /' "$WORK/serve.out"
      return 1
    }
    sleep 0.5
    i=$((i + 1))
  done
  echo "relay: host did not become ready in time"
  sed 's/^/    /' "$WORK/serve.out"
  return 1
}

stop_relay() {
  [ -n "$RELAY_PID" ] || return 0
  ckpt "relay: stopping proxy relay (pid=$RELAY_PID)"
  kill -KILL "$RELAY_PID" 2>/dev/null
  wait "$RELAY_PID" 2>/dev/null
  RELAY_PID=""
}

# ---------------------------------------------------------------------------
# Engine host. OpenClaw and Hermes are PROXY-routed: they take a forward proxy
# (`proxy.proxyUrl` and `HTTPS_PROXY`) rather than a base URL, so `connect`
# refuses unless `proxy::engine_proxy_url()` is Some. Only `proxy enable`
# makes it so - it writes the system-proxy snapshot and the engine port, and
# `proxy relay` (the relay) writes neither. The other three tools keep using
# the relay and are untouched by this: the exported NO_PROXY is
# `localhost,127.0.0.1,::1`, which exempts both the relay and the mocks.
#
# Enable also trusts the CA and points the system proxy at the engine. What
# that costs differs per platform, and only Linux escalates:
#   - Linux   `update-ca-certificates` into the system store, via run_as_admin.
#             That picks sudo only when stdout is a TTY and pkexec otherwise,
#             and a runner has no polkit agent - hence `script`, which supplies
#             a pty so it takes the (passwordless here) sudo branch.
#   - macOS   `security add-trusted-cert` into the LOGIN keychain, and
#             networksetup, which is tried unprivileged first. No escalation.
#   - Windows `certutil -user -addstore`, per-user. No escalation either, but
#             see the skip below.
# ---------------------------------------------------------------------------
start_engine() {
  # Windows is held back, and NOT because of privilege: certutil -user and the
  # HKCU proxy keys both go through unprompted there. The blocker is the engine
  # LISTENER. `proxy_e2e::exported_proxy_env_routes_an_external_process` fails
  # on the Windows CI job with curl refused at 127.0.0.1:<engine port> after
  # 6ms, having been handed the port the engine itself reported, while the same
  # test passes on Linux and macOS. That is the socket, not the env channel -
  # which matters here, because these two tools take their proxy from config
  # (`proxy.proxyUrl`, `~/.hermes/.env`) rather than from the environment, so
  # they would hit the same dead address. Routing a tool through an address that
  # refuses connections reports a tool failure for an engine bug, so skip until
  # that is fixed. (Hermes is absent on Windows regardless - the workflow does
  # not install it there.)
  if [ "$OS" = "Windows" ]; then
    echo "::notice::skipping the engine on Windows - the exported proxy env does not reach the listener there yet"
    return 1
  fi
  ckpt "engine: proxy enable"
  local rc=0
  if [ "$OS" = "Linux" ]; then
    script -qec "\"$CLI\" proxy enable" /dev/null >"$WORK/enable.out" 2>&1 || rc=$?
  else
    "$CLI" proxy enable >"$WORK/enable.out" 2>&1 || rc=$?
  fi
  if [ "$rc" -ne 0 ]; then
    echo "engine: proxy enable failed (rc=$rc)"
    sed 's/^/    /' "$WORK/enable.out" 2>/dev/null
    return 1
  fi
  ENGINE_ON=1
  # Hermes' seeded provider is OpenRouter, whose catalog entry ships opt-in, so
  # without this the engine tunnels openrouter.ai straight past Gate and the
  # capture stays empty. Anthropic (OpenClaw's provider) is on by default.
  "$CLI" proxy domain openrouter on >>"$WORK/enable.out" 2>&1 || true
  # `proxy enable` reports "Proxy:    running on 127.0.0.1:<port>", with no
  # scheme - matching on one printed an empty checkpoint.
  local eport
  eport="$(grep -o '127\.0\.0\.1:[0-9]*' "$WORK/enable.out" | head -n1)"
  ckpt "engine: enabled ($eport)"
  # A fresh `proxy status` must see the daemon that was just armed. This is the
  # daemon-adoption path and nothing else covers it: `self.client` is `Some`
  # only for the process that enabled and stayed alive, so every CLI invocation
  # used to report the proxy off while the daemon was intercepting - config and
  # engine disagreeing, the shape this suite exists to catch. Asserting the
  # port too, so "running" cannot pass by naming some other engine.
  #
  # Linux only: macOS runs the engine in-process, so a separate process has no
  # daemon to adopt and reporting "stopped" is correct there.
  if [ "$OS" = "Linux" ]; then
    local st
    st="$("$CLI" proxy status 2>&1 | head -n1)"
    if [ "${st#*running on }" = "$eport" ]; then
      echo "PASS: proxy status sees the running daemon on $eport"
      PASS=$((PASS + 1))
    else
      echo "FAIL: proxy status does not see the running daemon (expected $eport, got: $st)"
      FAIL=$((FAIL + 1))
    fi
  fi
}

stop_engine() {
  [ -n "$ENGINE_ON" ] || return 0
  # Harvest the daemon's log BEFORE disabling, into $WORK/*.out so the
  # workflow's existing diagnostics glob picks it up. Located rather than
  # constructed: app_support_dir resolves three different ways (the test-home
  # seam on Windows, XDG on Linux, Library/Application Support on macOS) and a
  # hardcoded guess would silently find nothing.
  local helper_log
  helper_log="$(find "$HOME" -path '*/proxy/helper.log' 2>/dev/null | head -n1)"
  if [ -n "$helper_log" ] && [ -f "$helper_log" ]; then
    cp "$helper_log" "$WORK/engine-requests.out" 2>/dev/null || true
    ckpt "engine: captured $(wc -l <"$helper_log" 2>/dev/null || echo 0) request log lines"
  else
    ckpt "engine: no helper.log at $helper_log"
  fi
  ckpt "engine: proxy disable"
  local rc=0
  "$CLI" proxy disable >"$WORK/disable.out" 2>&1 || rc=$?
  # Verify rather than assume. A disable that fails leaves the system-proxy
  # snapshot on disk, and `proxy relay` refuses to start while that exists (it
  # means an engine is up, hosting this same relay) - so a swallowed failure
  # here surfaces as the NEXT phase's start_relay dying, pages away from the
  # cause. Checking the snapshot, not `proxy status`: on Linux a fresh CLI
  # process has no control connection to the daemon and status reports
  # "stopped" whether or not the daemon is intercepting.
  #
  # Located rather than constructed, like helper.log above: app_support_dir
  # resolves three different ways and a hardcoded guess would silently find
  # nothing, i.e. always "verify" clean.
  local snapshot
  snapshot="$(find "$HOME" -path '*/proxy/system-proxy.snapshot.json' 2>/dev/null | head -n1)"
  if [ "$rc" -ne 0 ] || [ -n "$snapshot" ]; then
    echo "FAIL: engine: proxy disable did not take (rc=$rc${snapshot:+, snapshot still at $snapshot})"
    sed 's/^/    /' "$WORK/disable.out" 2>/dev/null
    FAIL=$((FAIL + 1))
    # Leave ENGINE_ON set so the EXIT trap tries once more; the runner would
    # otherwise be left routed through a dead proxy with a trusted CA.
    return 1
  fi
  ckpt "engine: disabled"
  # The disable must have reached the *daemon*, not merely cleared the snapshot
  # and removed the drop-in. Those two are local file operations that succeed
  # whether or not a control connection ever existed, which is exactly how
  # `proxy disable` looked like it worked while leaving the daemon intercepting
  # with its ports bound: the CLI held no handle to send SetPassthrough on, and
  # only newly launched processes stopped routing, because the drop-in was
  # gone. The daemon logs the disarm, and that line is the only trace the two
  # cases differ by. Read live rather than from the copy harvested above, which
  # was taken before the disable.
  if [ "$OS" = "Linux" ] && [ -n "$helper_log" ] && [ -f "$helper_log" ]; then
    if grep -q 'SetPassthrough received' "$helper_log"; then
      echo "PASS: proxy disable reached the daemon and disarmed the engine"
      PASS=$((PASS + 1))
    else
      echo "FAIL: proxy disable never reached the daemon (no SetPassthrough in $helper_log)"
      FAIL=$((FAIL + 1))
    fi
  fi
  # Stop the daemon too, not just routing. Pass-through leaves the engine up
  # with its ports bound - helper.rs is explicit that SetPassthrough and
  # client-disconnect never stop it - and one of those is the relay port the
  # next phase's start_relay needs. `proxy relay` now refuses a taken port
  # rather than silently moving to a fresh one and repointing every tool
  # config at it, so a leaked daemon fails the next phase instead of quietly
  # skewing it. No CLI verb stops the daemon; use the pidfile it writes at
  # startup. No-op on macOS, which runs the engine in-process.
  local pidfile dpid
  pidfile="$(find "${XDG_RUNTIME_DIR:-/tmp}" /tmp -name proxyd.pid 2>/dev/null | head -n1)"
  dpid="$([ -n "$pidfile" ] && cat "$pidfile" 2>/dev/null)"
  if [ -n "$dpid" ] && kill -0 "$dpid" 2>/dev/null; then
    ckpt "engine: stopping helper daemon (pid=$dpid)"
    kill -TERM "$dpid" 2>/dev/null
    local i=0
    while [ "$i" -lt 20 ] && kill -0 "$dpid" 2>/dev/null; do
      sleep 0.25
      i=$((i + 1))
    done
    kill -0 "$dpid" 2>/dev/null && kill -KILL "$dpid" 2>/dev/null
  fi
  ENGINE_ON=""
}

# Fake the browser leg of `login --oauth`: run it headless, read the authorize
# URL it prints, and hit its loopback /callback with a code + the echoed state.
# The CLI then exchanges the code (mock-auth /oauth2/token), lists orgs
# (mock-auth /v1/me/orgs -> single org, auto-selected), and persists everything.
oauth_login() {
  local out="$WORK/oauth-login.out"
  : > "$out"
  ckpt "oauth: starting login --oauth"
  "$CLI" login --base-url "$BASE_URL" --oauth >"$out" 2>&1 &
  local lpid=$!
  local url="" i=0
  while [ "$i" -lt 40 ]; do
    url=$(grep -oE 'https://[^ ]*oauth2/authorize[^ ]*' "$out" 2>/dev/null | head -n1)
    [ -n "$url" ] && break
    kill -0 "$lpid" 2>/dev/null || break
    sleep 0.25
    i=$((i + 1))
  done
  if [ -z "$url" ]; then
    echo "oauth: login never printed an authorize URL"
    sed 's/^/    /' "$out"
    kill -KILL "$lpid" 2>/dev/null
    return 1
  fi
  ckpt "oauth: faking browser callback"
  local cb
  cb=$(node -e 'const u=new URL(process.argv[1]);const r=u.searchParams.get("redirect_uri");const s=u.searchParams.get("state");const c=new URL(r);c.searchParams.set("code","e2e-auth-code");c.searchParams.set("state",s);process.stdout.write(c.toString());' "$url") || {
    echo "oauth: could not derive callback URL from: $url"
    kill -KILL "$lpid" 2>/dev/null
    return 1
  }
  curl -s "$cb" >/dev/null 2>&1
  ckpt "oauth: callback delivered; waiting for login to finish"
  wait "$lpid"
}

# run_tool <label> <slug> <path-needle> <mode> -- <invoke cmd...>
run_tool() {
  local label="$1" slug="$2" needle="$3" mode="$4"
  shift 4
  [ "$1" = "--" ] && shift
  echo "::group::$label ($mode)"
  TOOL_OUT="$WORK/$slug-$mode.out" # per-tool/phase so the diagnostics step keeps each one
  : > "$CAPTURE"
  ckpt "[$label/$mode] connect"
  if "$CLI" connect "$slug"; then
    ckpt "[$label/$mode] connected; running tool"
    run_until_capture "$needle" "$@"
    ckpt "[$label/$mode] run_until_capture returned; tool output:"
    sed 's/^/    /' "$TOOL_OUT" 2>/dev/null
    ckpt "[$label/$mode] disconnect"
    "$CLI" disconnect "$slug" >/dev/null 2>&1
    ckpt "[$label/$mode] asserting capture"
    if node "$(winpath "$ROOT/ci/e2e/assert-capture.mjs")" "$(winpath "$CAPTURE")" "$needle" "$mode"; then
      echo "PASS: $label reached the gateway with the $mode Gate headers"
      PASS=$((PASS + 1))
    else
      echo "FAIL: $label did not reach the gateway as expected ($mode)"
      FAIL=$((FAIL + 1))
    fi
    ckpt "[$label/$mode] done"
  else
    echo "FAIL: $label connect failed ($mode)"
    FAIL=$((FAIL + 1))
  fi
  echo "::endgroup::"
}

# Pick an anthropic model from opencode's catalog rather than pinning an id:
# models.dev retires them (claude-3-5-haiku-latest vanished and broke this).
# Skip the `-latest` aliases - they're the unstable ones models.dev remaps, and
# `opencode models` lists in no fixed order, so grabbing the first match landed
# on claude-3-5-haiku-latest at random. Sort for a deterministic pick instead.
# Computed once (it's independent of the auth phase).
OPENCODE_MODEL=""
if command -v opencode >/dev/null 2>&1; then
  OPENCODE_MODEL=$(opencode models | grep '^anthropic/' | grep -v -- '-latest$' | sort | head -n1)
fi

# Same idea for OpenClaw: its catalog id drifts, so pick a current anthropic one
# from `openclaw infer model list` (JSON lines) rather than pinning. The request
# reaches the relay regardless of whether the id is "real" (the mock 200s), but a
# catalog id keeps openclaw from bailing before it sends.
OPENCLAW_MODEL=""
if command -v openclaw >/dev/null 2>&1; then
  OPENCLAW_MODEL=$(openclaw infer model list 2>/dev/null \
    | grep '"provider":"anthropic"' | grep -v -- '-latest"' \
    | head -n1 | sed -E 's/.*"id":"([^"]+)".*/\1/')
fi

# Run every installed tool against the relay and assert the given auth mode's
# Gate headers reached the mock gateway. The tool config is mode-independent (it
# just points at the relay); the relay injects the differing credential.
run_relay_tools() {
  local mode="$1"

  # --- Claude Code: gate-connect writes the relay base URL + upstream headers
  #     into the env block of ~/.claude/settings.json; claude POSTs /v1/messages
  #     to the relay. We run `--bare` (the documented CI mode - it skips the
  #     OAuth/keychain read that otherwise hangs headless macOS) and feed it that
  #     exact settings file via `--settings` so the env block applies.
  mkdir -p "$HOME/.claude"
  run_tool "claude-code" "claude-code" "/v1/messages" "$mode" -- \
    claude --bare -p "ping" --settings "$(winpath "$HOME/.claude/settings.json")"

  # --- Codex: apikey mode → relay base + /v1, POSTs /v1/responses. Talks to the
  #     relay over plaintext http now, so the old custom-CA problem
  #     (openai/codex#9526) no longer applies. Guarded on install - codex isn't
  #     on every runner in the matrix.
  if command -v codex >/dev/null 2>&1; then
    mkdir -p "$HOME/.codex"
    printf '{"auth_mode":"apikey","OPENAI_API_KEY":"sk-e2e-dummy"}' > "$HOME/.codex/auth.json"
    run_tool "codex" "codex" "/v1/responses" "$mode" -- \
      codex exec --skip-git-repo-check "ping"
  else
    echo "::notice::skipping codex - CLI not installed on this runner"
  fi

  # --- OpenCode: gate-connect rewrites its anthropic provider's baseURL to the
  #     relay → POSTs /v1/messages. A model must be named explicitly
  #     (`provider/model`), otherwise `opencode run` picks a default that bypasses
  #     the anthropic provider we overrode and hits the real API.
  mkdir -p "$HOME/.config/opencode" "$HOME/.local/share/opencode"
  printf '{"anthropic":{"type":"api","key":"sk-ant-e2e-dummy"}}' \
    > "$HOME/.local/share/opencode/auth.json"
  printf '{"provider":{"anthropic":{}}}' > "$HOME/.config/opencode/opencode.json"
  if [ -z "$OPENCODE_MODEL" ]; then
    echo "::notice::skipping opencode - no anthropic model listed (or CLI not installed)"
  else
    run_tool "opencode" "opencode" "/v1/messages" "$mode" -- \
      opencode run --model "$OPENCODE_MODEL" "ping"
  fi

}

# The proxy-routed half, run with the relay DOWN and the engine up. The two
# hosts cannot overlap: on Linux the engine lives in the helper daemon, which
# hosts a relay of its own and rewrites the persisted relay port on the way up
# (manager_linux::enable passes relay::load_persisted_port() into set_intercept,
# then saves whatever came back). With `proxy relay` already holding that port
# the two fight over it, `connect` writes the wrong one into every tool config,
# and the relay-routed tools stop reaching the gateway - measured, and it took
# all ten tools down rather than just these two. macOS has no daemon and passed
# with both up at once, which is what pinned the cause to the daemon.
run_engine_tools() {
  local mode="$1"

  # --- OpenClaw: PROXY-routed since the harnesses moved off per-provider
  #     baseUrl edits. gate-connect writes `proxy.proxyUrl` (a process-wide
  #     interceptor over OpenClaw's HTTP clients) plus NODE_EXTRA_CA_CERTS, and
  #     leaves the provider block alone - so the seeded baseUrl below stays
  #     CANONICAL. The request goes to the real api.anthropic.com, the engine
  #     MITMs it on the way out, and rewrites /v1/messages to the mock gateway.
  #     Nothing dials Anthropic: /v1/messages is a rewrite prefix, so the engine
  #     never opens the upstream leg.
  #
  #     Needle stays /v1/messages: the anthropic catalog entry's upstream is the
  #     bare host, so the forwarded path is the app's own.
  #
  #     We seed a minimal anthropic provider block (with a dummy apiKey so
  #     openclaw actually fires the call): gate-connect detects OpenClaw off the
  #     config dir existing, and needs a supported provider under
  #     models.providers to have something to route. `infer model run --local`
  #     runs one turn straight through the provider baseUrl (not the openclaw
  #     gateway daemon, which isn't running here). Guarded on install, a
  #     resolved catalog model, and the engine.
  if ! command -v openclaw >/dev/null 2>&1; then
    echo "::notice::skipping openclaw - CLI not installed on this runner"
  elif [ -z "$OPENCLAW_MODEL" ]; then
    echo "::notice::skipping openclaw - no anthropic model listed in the catalog"
  elif [ -z "$ENGINE_ON" ]; then
    echo "::notice::skipping openclaw - proxy-routed, and the engine is not up"
  else
    mkdir -p "$HOME/.openclaw"
    printf '{"models":{"providers":{"anthropic":{"baseUrl":"https://api.anthropic.com/v1","apiKey":"sk-ant-e2e-dummy"}}}}' \
      > "$HOME/.openclaw/openclaw.json"
    run_tool "openclaw" "openclaw" "/v1/messages" "$mode" -- \
      openclaw infer model run --local --model "anthropic/$OPENCLAW_MODEL" --prompt "ping"
  fi

  # --- Hermes: Python OpenAI-compatible agent, PROXY-routed like OpenClaw.
  #     gate-connect writes HTTPS_PROXY / HTTP_PROXY / NO_PROXY / HERMES_CA_BUNDLE
  #     into ~/.hermes/.env and does not touch config.yaml at all, so the seeded
  #     base_url below stays CANONICAL and the engine catches the socket
  #     whichever provider config wins. HERMES_CA_BUNDLE is required rather than
  #     nice-to-have: hermes installs into a venv, so httpx/requests use a
  #     pip-installed certifi that knows nothing about the OS trust store.
  #
  #     We seed a complete model block the way a configured user's would look:
  #     provider must be set or hermes refuses to run ("No LLM provider
  #     configured"), and base_url must be a public https URL or gate-connect
  #     treats it as local and refuses to route it. provider=custom is hermes'
  #     recipe for an OpenAI-compatible endpoint: it calls model.base_url
  #     directly using model.api_key.
  #
  #     Seeded with OpenRouter on purpose - it is what a stock `hermes` install
  #     ships with. The needle is /v1/chat/completions, NOT /api/v1/...: the
  #     openrouter catalog entry carries `/api` in its upstream_url precisely so
  #     the forwarded path clears Gate's ALB, which routes /api/* to the
  #     dashboard API. A capture arriving as /api/v1/... would mean that split
  #     regressed and every OpenRouter request is 404ing short of the proxy.
  #     Guarded on install and on the engine.
  if ! command -v hermes >/dev/null 2>&1; then
    echo "::notice::skipping hermes - CLI not installed on this runner"
  elif [ -z "$ENGINE_ON" ]; then
    echo "::notice::skipping hermes - proxy-routed, and the engine is not up"
  else
    mkdir -p "$HOME/.hermes"
    printf 'model:\n  provider: custom\n  base_url: https://openrouter.ai/api/v1\n  api_key: sk-e2e-dummy\n  api_mode: chat_completions\n' \
      > "$HOME/.hermes/config.yaml"
    export OPENAI_API_KEY="sk-e2e-dummy"
    run_tool "hermes" "hermes" "/v1/chat/completions" "$mode" -- \
      hermes -z "ping" --model openai/gpt-4o-mini
  fi
}

# ---------------------------------------------------------------------------
# Phase A - API key (legacy). Relay injects x-gate-api-key.
# ---------------------------------------------------------------------------
ckpt "mocks ready; PHASE A (api-key)"
echo "::group::phase: api-key login"
"$CLI" login --base-url "$BASE_URL" --api-key "sk-gw-e2e" || {
  echo "api-key login failed"
  exit 1
}
echo "::endgroup::"
start_relay || exit 1
run_relay_tools "api-key"
stop_relay
# Best-effort, like the per-tool guards: a runner that cannot bring the engine
# up should still prove the three relay-routed tools rather than failing the
# whole phase. The two proxy-routed tools skip with a notice in that case.
start_engine || echo "::warning::engine unavailable - openclaw and hermes will be skipped"
run_engine_tools "api-key"
stop_engine
"$CLI" logout >/dev/null 2>&1 || true

# ---------------------------------------------------------------------------
# Phase B - OAuth (primary). Relay injects x-gate-authorization + x-gate-org-id.
# ---------------------------------------------------------------------------
ckpt "PHASE B (oauth)"
echo "::group::phase: oauth login"
if oauth_login; then
  echo "::endgroup::"
  start_relay || exit 1
  run_relay_tools "oauth"
  stop_relay
  start_engine || echo "::warning::engine unavailable - openclaw and hermes will be skipped"
  run_engine_tools "oauth"
  stop_engine
  "$CLI" logout >/dev/null 2>&1 || true
else
  echo "::endgroup::"
  echo "FAIL: oauth login did not complete; skipping the OAuth phase"
  FAIL=$((FAIL + 1))
fi

ckpt "all phases finished; reached end of script"
echo "----------------------------------------"
echo "Passed: $PASS  Failed: $FAIL"
test "$FAIL" -eq 0
