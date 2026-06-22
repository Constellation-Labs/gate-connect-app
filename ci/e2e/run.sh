#!/usr/bin/env bash
# Real-tools end-to-end driver. Points each installed AI CLI at a local HTTPS
# mock gateway via the normal `gate-connect connect` flow, fires one headless
# request, and asserts the mock received it with the Gate headers. Best-effort:
# the external CLIs' invocation flags and auth shapes drift over time, so each
# tool is allowed to fail without aborting the others; the per-tool capture
# assertion is the source of truth.
#
# Runs on Linux, macOS, and Windows (Git Bash). Assumes: `gate-connect` is built
# (target/debug), node is on PATH, and the CLIs are installed. May mutate the OS
# trust store, so it only belongs on a throwaway CI runner.
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
  # XDG — none of which follow Git Bash's $HOME. Point all of them (in native
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

CLI="$ROOT/target/debug/gate-connect"
[ "$OS" = "Windows" ] && CLI="$CLI.exe"
PORT=8443
# Use 127.0.0.1, not localhost: on macOS `localhost` can resolve to IPv6 ::1
# first, but the mock binds 127.0.0.1 only — so a tool would hit a dead ::1
# address. The leaf cert carries an IP:127.0.0.1 SAN so TLS still validates.
BASE_URL="https://127.0.0.1:$PORT"
CAPTURE="$WORK/capture.jsonl"
: > "$CAPTURE"
# Tool stdout/stderr goes here, not the step's inherited pipe: on Windows a tool
# (codex) can leave an orphaned grandchild process that keeps the pipe open and
# stalls the whole step. Writing to a file lets the step finish; we print it
# after each run for visibility.
TOOL_OUT="$WORK/tool.out"

# Node tools (claude / opencode-on-bun) talk to the mock over TLS. Rather than
# fight each runtime's CA-trust quirks (bun ignores NODE_EXTRA_CA_CERTS on
# macOS; msys cert paths confuse Node on Windows), skip verification for these
# local-only calls — the assertion is about the request reaching the gateway
# with the right headers, not about CA trust. ANTHROPIC_API_KEY just lets claude
# attach an auth header so it actually sends.
export NODE_TLS_REJECT_UNAUTHORIZED=0
export ANTHROPIC_API_KEY="sk-ant-e2e-dummy"
# Codex (Rust) has no env to trust a custom CA for a model provider
# (openai/codex#9526), so it relies on the OS trust store — Linux only below.
export CODEX_CA_CERTIFICATE="$(winpath "$CA_DIR/ca.pem")"

PASS=0
FAIL=0

# Launch a tool (output to a file, never the step's pipe) and poll the capture
# until the expected request shows up or we time out. We deliberately do NOT
# `wait` on the process: on Windows the codex shim spawns a grandchild that may
# be unkillable from msys, and blocking on it would stall the whole step. Since
# the tool's stdout/stderr go to a file, leaving it orphaned is harmless — the
# step still completes once the script exits and the EXIT trap stops the mock.
run_until_capture() {
  local needle="$1"
  shift
  : > "$TOOL_OUT"
  "$@" </dev/null >"$TOOL_OUT" 2>&1 &
  local pid=$!
  local i=0
  while [ "$i" -lt 90 ]; do
    grep -q "$needle" "$CAPTURE" 2>/dev/null && break
    kill -0 "$pid" 2>/dev/null || break # process exited on its own
    sleep 1
    i=$((i + 1))
  done
  kill -KILL "$pid" 2>/dev/null # best-effort; not waited on
}

# ---------------------------------------------------------------------------
# 1. Mint a throwaway CA + a 127.0.0.1 leaf. On Linux/macOS we also trust the CA
#    in the OS store (for Codex's Rust TLS); Node tools rely on the verify-skip
#    above, so Windows needs no trust-store wiring.
# ---------------------------------------------------------------------------
# Run openssl from inside CA_DIR with bare filenames so it works whether the
# openssl on PATH is the msys build (wants /c/… paths) or a native Windows one
# (wants C:\… paths) — relative names resolve against the process cwd either way.
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

export NODE_EXTRA_CA_CERTS="$(winpath "$CA_DIR/ca.pem")"

case "$OS" in
  Linux)
    sudo cp "$CA_DIR/ca.pem" /usr/local/share/ca-certificates/gc-e2e.crt
    sudo update-ca-certificates
    ;;
  Darwin)
    sudo security add-trusted-cert -d -r trustRoot \
      -k /Library/Keychains/System.keychain "$CA_DIR/ca.pem"
    ;;
  Windows)
    # Into the current-user Root store (no elevation needed). Codex's Rust TLS
    # reads the Windows cert store, so this is the Windows analogue of the Linux
    # trust above.
    certutil -user -addstore -f Root "$(winpath "$CA_DIR/ca.pem")" >/dev/null 2>&1 || true
    ;;
esac

# ---------------------------------------------------------------------------
# 2. Start the mock gateway and wait until it accepts TLS.
# ---------------------------------------------------------------------------
# Mock output goes to a file (not the step's pipe) so nothing but the shell
# itself holds stdout — the step then completes the moment the script exits,
# even if a tool left an orphaned process behind.
CAPTURE_LOG="$(winpath "$CAPTURE")" MOCK_PORT="$PORT" \
  MOCK_CERT="$(winpath "$CA_DIR/leaf.pem")" MOCK_KEY="$(winpath "$CA_DIR/leaf.key")" \
  node "$(winpath "$ROOT/ci/e2e/mock-gateway.mjs")" >"$WORK/mock.out" 2>&1 &
MOCK_PID=$!
cleanup() {
  kill -KILL "$MOCK_PID" 2>/dev/null
  # Codex's Rust binary survives an msys kill and, in the runner's job object,
  # keeps the step from finishing. A real Windows kill of the whole tree lets
  # the step exit. (No-op if codex isn't running; codex.exe doesn't exist off
  # Windows.)
  if [ "$OS" = "Windows" ]; then
    taskkill /F /T /IM codex.exe >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

for _ in $(seq 1 40); do
  if curl -sk "$BASE_URL/healthz" >/dev/null 2>&1; then break; fi
  sleep 0.5
done

# ---------------------------------------------------------------------------
# 3. Sign in once; every tool reuses this account.
# ---------------------------------------------------------------------------
"$CLI" login --base-url "$BASE_URL" --api-key "sk-gw-e2e" || {
  echo "login failed"
  exit 1
}

# run_tool <label> <slug> <path-needle> -- <invoke cmd...>
run_tool() {
  local label="$1" slug="$2" needle="$3"
  shift 3
  [ "$1" = "--" ] && shift
  echo "::group::$label"
  : > "$CAPTURE"
  if "$CLI" connect "$slug"; then
    run_until_capture "$needle" "$@"
    sed 's/^/    /' "$TOOL_OUT" 2>/dev/null
    "$CLI" disconnect "$slug" >/dev/null 2>&1
    if node "$(winpath "$ROOT/ci/e2e/assert-capture.mjs")" "$(winpath "$CAPTURE")" "$needle"; then
      echo "PASS: $label reached the gateway with Gate headers"
      PASS=$((PASS + 1))
    else
      echo "FAIL: $label did not reach the gateway as expected"
      FAIL=$((FAIL + 1))
    fi
  else
    echo "FAIL: $label connect failed"
    FAIL=$((FAIL + 1))
  fi
  echo "::endgroup::"
}

# --- Claude Code: gate-connect writes the gateway URL + headers into the env
#     block of ~/.claude/settings.json; claude POSTs /v1/messages there. We run
#     `--bare` (the documented CI mode — it skips the OAuth/keychain read that
#     otherwise hangs headless macOS) and feed it that exact settings file via
#     `--settings` so the env block still applies.
mkdir -p "$HOME/.claude"
run_tool "claude-code" "claude-code" "/v1/messages" -- \
  claude --bare -p "ping" --settings "$(winpath "$HOME/.claude/settings.json")"

# --- Codex: apikey mode → base_url + /v1, POSTs /v1/responses. Codex has no env
#     to trust a custom CA for a model provider (openai/codex#9526), so it relies
#     on the OS trust store — which we populate on Linux and Windows above.
#     macOS codex doesn't read the keychain for this, so it stays skipped there.
if [ "$OS" != "Darwin" ]; then
  mkdir -p "$HOME/.codex"
  printf '{"auth_mode":"apikey","OPENAI_API_KEY":"sk-e2e-dummy"}' > "$HOME/.codex/auth.json"
  run_tool "codex" "codex" "/v1/responses" -- \
    codex exec --skip-git-repo-check "ping"
else
  echo "::notice::skipping codex on macOS — its TLS stack can't trust a test CA for a custom model provider (openai/codex#9526) and ignores the keychain; codex is exercised on Linux and Windows."
fi

# --- OpenCode: gate-connect rewrites its anthropic provider's baseURL to Gate
#     → POSTs /v1/messages. A model must be named explicitly (`provider/model`),
#     otherwise `opencode run` picks a default that bypasses the anthropic
#     provider we overrode and hits the real API.
mkdir -p "$HOME/.config/opencode" "$HOME/.local/share/opencode"
printf '{"anthropic":{"type":"api","key":"sk-ant-e2e-dummy"}}' \
  > "$HOME/.local/share/opencode/auth.json"
printf '{"provider":{"anthropic":{}}}' > "$HOME/.config/opencode/opencode.json"
run_tool "opencode" "opencode" "/v1/messages" -- \
  opencode run --model anthropic/claude-3-5-haiku-latest "ping"

echo "----------------------------------------"
echo "Passed: $PASS  Failed: $FAIL"
test "$FAIL" -eq 0
