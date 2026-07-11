#!/usr/bin/env bash
# Real-tools end-to-end driver. Exercises the full relay path: `gate-connect
# proxy serve` hosts the loopback reverse-proxy relay, `gate-connect connect`
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
cleanup() {
  # Preserve the status that triggered the trap (the final `test $FAIL -eq 0`,
  # or an early `exit`) before any teardown command clobbers $?.
  local status=$?
  kill -KILL "$MOCK_PID" 2>/dev/null
  kill -KILL "$AUTH_PID" 2>/dev/null
  [ -n "$RELAY_PID" ] && kill -KILL "$RELAY_PID" 2>/dev/null
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
# Relay host: `gate-connect proxy serve` binds the loopback relay and blocks.
# It seeds the credential channels from the current account, so it must be
# (re)started after each phase's login. connect reads the persisted relay port,
# so serve must be up before any connect.
# ---------------------------------------------------------------------------
start_relay() {
  : > "$WORK/serve.out"
  ckpt "relay: starting proxy serve"
  "$CLI" proxy serve >"$WORK/serve.out" 2>&1 &
  RELAY_PID=$!
  local i=0
  while [ "$i" -lt 30 ]; do
    grep -q 'relay listening on' "$WORK/serve.out" 2>/dev/null && {
      ckpt "relay: ready ($(grep -o 'http://[^ ]*' "$WORK/serve.out" | head -n1))"
      return 0
    }
    kill -0 "$RELAY_PID" 2>/dev/null || {
      echo "relay: serve exited before becoming ready"
      sed 's/^/    /' "$WORK/serve.out"
      return 1
    }
    sleep 0.5
    i=$((i + 1))
  done
  echo "relay: serve did not become ready in time"
  sed 's/^/    /' "$WORK/serve.out"
  return 1
}

stop_relay() {
  [ -n "$RELAY_PID" ] || return 0
  ckpt "relay: stopping proxy serve (pid=$RELAY_PID)"
  kill -KILL "$RELAY_PID" 2>/dev/null
  wait "$RELAY_PID" 2>/dev/null
  RELAY_PID=""
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

# Run every installed tool against the relay and assert the given auth mode's
# Gate headers reached the mock gateway. The tool config is mode-independent (it
# just points at the relay); the relay injects the differing credential.
run_all_tools() {
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

  # --- OpenClaw: multi-provider, like OpenCode. gate-connect rewrites the
  #     anthropic provider's baseUrl in ~/.openclaw/openclaw.json to the relay →
  #     POSTs /v1/messages. We seed a minimal anthropic provider block:
  #     gate-connect detects OpenClaw off the config dir existing, and needs a
  #     supported provider (anthropic/openai/openrouter) under models.providers to
  #     have something to route. Guarded on install.
  if command -v openclaw >/dev/null 2>&1; then
    mkdir -p "$HOME/.openclaw"
    printf '{"models":{"providers":{"anthropic":{"baseUrl":"https://api.anthropic.com/v1"}}}}' \
      > "$HOME/.openclaw/openclaw.json"
    run_tool "openclaw" "openclaw" "/v1/messages" "$mode" -- \
      openclaw message "ping"
  else
    echo "::notice::skipping openclaw - CLI not installed on this runner"
  fi

  # --- Hermes: Python OpenAI-compatible agent. gate-connect rewrites
  #     model.base_url in ~/.hermes/config.yaml to the relay and injects the Gate
  #     headers into model.default_headers → POSTs /v1/chat/completions. We seed a
  #     complete model block the way a configured user's would look: provider must
  #     be set or hermes refuses to run ("No LLM provider configured"), and
  #     base_url must be a public https URL or gate-connect treats it as local and
  #     refuses to route it. provider=custom is hermes' recipe for an
  #     OpenAI-compatible endpoint: it calls model.base_url directly using
  #     model.api_key. Since hermes now talks to the plaintext relay, no
  #     ssl_verify / custom_providers TLS shim is needed. The base_url host must be
  #     one the relay allows: gate-connect derives X-Gate-Upstream-Url by stripping
  #     the trailing /v1, and the relay only forwards to upstreams in its built-in
  #     catalog - so api.openai.com works but openrouter.ai/api (bare host is
  #     openrouter.ai in the catalog, not openrouter.ai/api) would be rejected 403.
  #     Guarded on install.
  if command -v hermes >/dev/null 2>&1; then
    mkdir -p "$HOME/.hermes"
    printf 'model:\n  provider: custom\n  base_url: https://api.openai.com/v1\n  api_key: sk-e2e-dummy\n  api_mode: chat_completions\n' \
      > "$HOME/.hermes/config.yaml"
    export OPENAI_API_KEY="sk-e2e-dummy"
    run_tool "hermes" "hermes" "/v1/chat/completions" "$mode" -- \
      hermes -z "ping" --model openai/gpt-4o-mini
  else
    echo "::notice::skipping hermes - CLI not installed on this runner"
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
run_all_tools "api-key"
stop_relay
"$CLI" logout >/dev/null 2>&1 || true

# ---------------------------------------------------------------------------
# Phase B - OAuth (primary). Relay injects x-gate-authorization + x-gate-org-id.
# ---------------------------------------------------------------------------
ckpt "PHASE B (oauth)"
echo "::group::phase: oauth login"
if oauth_login; then
  echo "::endgroup::"
  start_relay || exit 1
  run_all_tools "oauth"
  stop_relay
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
