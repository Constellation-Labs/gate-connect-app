#!/usr/bin/env bash
# Real-tools end-to-end driver. Points each installed AI CLI at a local HTTPS
# mock gateway via the normal `gate-connect connect` flow, fires one headless
# request, and asserts the mock received it with the Gate headers. Best-effort:
# the external CLIs' invocation flags and auth shapes drift over time, so each
# tool is allowed to fail without aborting the others; the per-tool capture
# assertion is the source of truth.
#
# Assumes: `gate-connect` is built (target/debug), node is on PATH, and the CLIs
# (claude / codex / opencode) are installed. Mutates the OS trust store, so it
# only belongs on a throwaway CI runner.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="${RUNNER_TEMP:-/tmp}/gc-e2e"
rm -rf "$WORK"
CA_DIR="$WORK/ca"
mkdir -p "$CA_DIR"

# Redirect the real home so gate-connect AND the tools agree on config paths
# (the tools don't know about GATE_CONNECT_TEST_HOME — they read $HOME). The
# Gate key still goes through the file-backed secret seam, since CI has no
# usable OS keychain headlessly.
export HOME="$WORK/home"
mkdir -p "$HOME"
export GATE_CONNECT_TEST_SECRETS="$WORK/secrets"
mkdir -p "$GATE_CONNECT_TEST_SECRETS"

CLI="$ROOT/target/debug/gate-connect"
PORT=8443
BASE_URL="https://localhost:$PORT"
CAPTURE="$WORK/capture.jsonl"
: > "$CAPTURE"

PASS=0
FAIL=0

# Portable timeout: run a command, kill it after N seconds. macOS has no
# `timeout`, so we roll a watchdog. stdin is closed so a tool that probes for
# a TTY can't block waiting on input.
with_timeout() {
  local secs="$1"
  shift
  "$@" </dev/null &
  local pid=$!
  ( sleep "$secs"; kill -TERM "$pid" 2>/dev/null ) &
  local wd=$!
  wait "$pid" 2>/dev/null
  kill "$wd" 2>/dev/null
  wait "$wd" 2>/dev/null
}

# ---------------------------------------------------------------------------
# 1. Mint a throwaway CA + a localhost leaf, and trust the CA in the OS store
#    (covers Rust tools using the platform verifier / native-tls; Node tools
#    are covered by NODE_EXTRA_CA_CERTS, set in the workflow).
# ---------------------------------------------------------------------------
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout "$CA_DIR/ca.key" -out "$CA_DIR/ca.pem" \
  -subj "/CN=Gate Connect E2E CA" -days 2 \
  -addext "basicConstraints=critical,CA:TRUE"

openssl req -newkey rsa:2048 -nodes \
  -keyout "$CA_DIR/leaf.key" -out "$CA_DIR/leaf.csr" \
  -subj "/CN=localhost"

cat > "$CA_DIR/leaf.ext" <<'EXT'
subjectAltName=DNS:localhost,IP:127.0.0.1
extendedKeyUsage=serverAuth
EXT

openssl x509 -req -in "$CA_DIR/leaf.csr" \
  -CA "$CA_DIR/ca.pem" -CAkey "$CA_DIR/ca.key" -CAcreateserial \
  -out "$CA_DIR/leaf.pem" -days 2 -extfile "$CA_DIR/leaf.ext"

# Node tools (claude / opencode-on-bun) read NODE_EXTRA_CA_CERTS, which *adds*
# to the system roots. Codex (Rust) honours CODEX_CA_CERTIFICATE — backend-
# agnostic, so it works on macOS where the keychain trust below didn't reach
# Codex's TLS stack.
export NODE_EXTRA_CA_CERTS="$CA_DIR/ca.pem"
export CODEX_CA_CERTIFICATE="$CA_DIR/ca.pem"

case "$(uname -s)" in
  Linux)
    sudo cp "$CA_DIR/ca.pem" /usr/local/share/ca-certificates/gc-e2e.crt
    sudo update-ca-certificates
    ;;
  Darwin)
    sudo security add-trusted-cert -d -r trustRoot \
      -k /Library/Keychains/System.keychain "$CA_DIR/ca.pem"
    ;;
esac

# ---------------------------------------------------------------------------
# 2. Start the mock gateway and wait until it accepts TLS.
# ---------------------------------------------------------------------------
CAPTURE_LOG="$CAPTURE" MOCK_PORT="$PORT" \
  MOCK_CERT="$CA_DIR/leaf.pem" MOCK_KEY="$CA_DIR/leaf.key" \
  node "$ROOT/ci/e2e/mock-gateway.mjs" &
MOCK_PID=$!
trap 'kill "$MOCK_PID" 2>/dev/null' EXIT

for _ in $(seq 1 40); do
  if curl -sk "$BASE_URL/healthz" >/dev/null 2>&1; then break; fi
  sleep 0.5
done

# ---------------------------------------------------------------------------
# 3. Sign in once; every tool reuses this account.
# ---------------------------------------------------------------------------
"$CLI" login --base-url "$BASE_URL" --api-key "sk-gw-e2e" || {
  echo "login failed"; exit 1;
}

# run_tool <label> <slug> <path-needle> -- <invoke cmd...>
run_tool() {
  local label="$1" slug="$2" needle="$3"
  shift 3
  [ "$1" = "--" ] && shift
  echo "::group::$label"
  : > "$CAPTURE"
  if "$CLI" connect "$slug"; then
    with_timeout 90 "$@"
    "$CLI" disconnect "$slug" >/dev/null 2>&1
    if node "$ROOT/ci/e2e/assert-capture.mjs" "$CAPTURE" "$needle"; then
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
  env ANTHROPIC_API_KEY="sk-ant-e2e-dummy" \
  claude --bare -p "ping" --settings "$HOME/.claude/settings.json"

# --- Codex: apikey mode → base_url + /v1, POSTs /v1/responses. The credential
#     helper reads OPENAI_API_KEY from auth.json.
mkdir -p "$HOME/.codex"
printf '{"auth_mode":"apikey","OPENAI_API_KEY":"sk-e2e-dummy"}' > "$HOME/.codex/auth.json"
run_tool "codex" "codex" "/v1/responses" -- \
  codex exec --skip-git-repo-check "ping"

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
