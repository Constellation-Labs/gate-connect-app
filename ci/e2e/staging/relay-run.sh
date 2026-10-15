#!/usr/bin/env bash
# AG-907: send marked requests through Gate Connect's own relay and prove each
# one landed as the right user, org and source, then compare the org's counts.
#
#   E2E_BACKEND=mock     (works today) the relay forwards to ci/e2e/mock-gateway.mjs
#                        with MOCK_EVIDENCE=1, which records what the gateway would
#                        store and serves the two reads the checker makes.
#   E2E_BACKEND=staging  (waits on AG-1102 to AG-1105) the relay forwards to the
#                        staging gateway under the test org's key, and the checker
#                        asks staging. Refused unless the host is staging (AC6).
#
# The relay is the debug `gate-connect proxy relay`, the same one the real-tools
# job drives, so the marker header is proven to survive Gate Connect's own
# header handling (rewrite_headers in crates/core/src/proxy/relay.rs). The
# installed app's routing goes through the same code; attaching markers to the
# installed-app flows is a driver hook (see the PR).
#
# CI runners only: it redirects HOME and writes a throwaway account.
# Env: E2E_BACKEND, OUT (results), and for staging GATE_STAGING_API_KEY,
# GATE_STAGING_ORG_ID, GATE_STAGING_USER_ID, GATE_STAGING_GATEWAY_URL (optional).
set -uo pipefail

if [ "${GITHUB_ACTIONS:-}" != "true" ] || [ "${RUNNER_ENVIRONMENT:-}" != "github-hosted" ]; then
  echo "relay-run.sh redirects HOME and signs a throwaway account in: GitHub-hosted runners only" >&2
  exit 2
fi

OS=other
case "$(uname -s)" in
  Linux) OS=Linux ;;
  Darwin) OS=Darwin ;;
  MINGW* | MSYS* | CYGWIN*) OS=Windows ;;
esac
winpath() { if [ "$OS" = Windows ]; then cygpath -w "$1"; else printf '%s' "$1"; fi; }
if [ "$OS" = Windows ]; then
  # Keep Git Bash from rewriting "/__gate/..." and openssl's -subj into paths.
  export MSYS2_ARG_CONV_EXCL='*'
  export MSYS_NO_PATHCONV=1
fi

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
BACKEND="${E2E_BACKEND:-mock}"
if [ "$OS" = Windows ]; then WORK="$(cygpath -u "${RUNNER_TEMP:-/tmp}")/gc-staging"; else WORK="${RUNNER_TEMP:-/tmp}/gc-staging"; fi
OUT="${OUT:-$WORK/out}"
[ "$OS" = Windows ] && OUT="$(cygpath -u "$OUT")"
rm -rf "$WORK"
mkdir -p "$WORK/ca" "$WORK/secrets" "$WORK/home" "$OUT"
CHECK=(node "$(winpath "$HERE/check.mjs")")

CLI="$ROOT/target/debug/gate-connect"
[ "$OS" = Windows ] && CLI="$CLI.exe"
[ -x "$CLI" ] || { echo "build the CLI first: cargo build -p gate-connect-cli" >&2; exit 2; }

# A throwaway home for the account, as ci/e2e/run.sh does.
export HOME="$WORK/home"
if [ "$OS" = Windows ]; then
  export GATE_CONNECT_TEST_HOME="$(winpath "$HOME")"
  export USERPROFILE="$(winpath "$HOME")"
fi
export XDG_CONFIG_HOME="$(winpath "$HOME/.config")" XDG_DATA_HOME="$(winpath "$HOME/.local/share")"
export GATE_CONNECT_TEST_SECRETS="$(winpath "$WORK/secrets")"

export E2E_BACKEND="$BACKEND"
export E2E_LEDGER="$(winpath "$OUT/ledger.jsonl")"
: >"$OUT/ledger.jsonl"

MOCK_PID=""
RELAY_PID=""
cleanup() {
  local rc=$?
  [ -n "$RELAY_PID" ] && kill "$RELAY_PID" 2>/dev/null
  [ -n "$MOCK_PID" ] && kill "$MOCK_PID" 2>/dev/null
  cp "$WORK"/*.out "$WORK"/capture.jsonl "$OUT/" 2>/dev/null || true
  exit "$rc"
}
trap cleanup EXIT

echo "::group::backend ($BACKEND)"
case "$BACKEND" in
  mock)
    (
      cd "$WORK/ca" || exit 1
      openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.pem \
        -subj "/CN=Gate Connect Staging E2E CA" -days 2 -addext "basicConstraints=critical,CA:TRUE" 2>/dev/null
      openssl req -newkey rsa:2048 -nodes -keyout leaf.key -out leaf.csr -subj "/CN=localhost" 2>/dev/null
      printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n' >leaf.ext
      openssl x509 -req -in leaf.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out leaf.pem -days 2 -extfile leaf.ext 2>/dev/null
    ) || { echo "could not mint the mock's certificate" >&2; exit 2; }
    export GATE_CONNECT_TEST_CA="$(winpath "$WORK/ca/ca.pem")"
    export NODE_EXTRA_CA_CERTS="$(winpath "$WORK/ca/ca.pem")"
    BASE_URL="https://127.0.0.1:8443"
    KEY="sk-gw-e2e-00000000000000000000000000000000"
    export E2E_EXPECT_ORG_ID="org-e2e-1" E2E_EXPECT_USER_ID="u-e2e"
    export MOCK_EVIDENCE_URL="$BASE_URL"
    : >"$WORK/capture.jsonl"
    # A 3 s ingestion lag, so the checker's polling is exercised, not only
    # its first read.
    MOCK_EVIDENCE=1 MOCK_EVIDENCE_DELAY_MS=3000 CAPTURE_LOG="$(winpath "$WORK/capture.jsonl")" MOCK_PORT=8443 \
      MOCK_CERT="$(winpath "$WORK/ca/leaf.pem")" MOCK_KEY="$(winpath "$WORK/ca/leaf.key")" \
      node "$(winpath "$ROOT/ci/e2e/mock-gateway.mjs")" >"$WORK/mock.out" 2>&1 &
    MOCK_PID=$!
    for _ in $(seq 1 50); do "${CHECK[@]}" preflight >/dev/null 2>&1 && break; sleep 0.2; done
    ;;
  staging)
    BASE_URL="${GATE_STAGING_GATEWAY_URL:-https://gateway-staging.constellationgate.ai}"
    KEY="${GATE_STAGING_API_KEY:-}"
    ;;
  *) echo "E2E_BACKEND must be mock or staging" >&2; exit 2 ;;
esac
# AC6, before any credential is used: the account may only point at the
# backend's own hosts.
"${CHECK[@]}" assert-host "$BASE_URL" "$BACKEND" || exit 2
# Staging is asked whether it is up before anything is sent, so an outage
# reads as one (exit 3) and not as a lost request.
"${CHECK[@]}" preflight
rc=$?
[ "$rc" -eq 0 ] || exit "$rc"
echo "::endgroup::"

echo "::group::sign in and start the relay"
# From a file, not argv, so the staging key never shows in a process list.
( umask 077; printf '%s\n' "$KEY" >"$WORK/key.txt" )
"$CLI" login --base-url "$BASE_URL" --api-key-file "$(winpath "$WORK/key.txt")" >"$WORK/login.out" 2>&1 || { cat "$WORK/login.out"; echo "sign-in failed" >&2; exit 2; }
"$CLI" proxy relay >"$WORK/relay.out" 2>&1 &
RELAY_PID=$!
RELAY=""
for _ in $(seq 1 60); do
  RELAY="$(grep -o 'relay listening on http://[^ ]*' "$WORK/relay.out" 2>/dev/null | head -1 | sed 's/relay listening on //')"
  [ -n "$RELAY" ] && break
  kill -0 "$RELAY_PID" 2>/dev/null || break
  sleep 0.5
done
[ -n "$RELAY" ] || { cat "$WORK/relay.out"; echo "the relay did not start" >&2; exit 2; }
RELAY="${RELAY%/}"
rm -f "$WORK/key.txt"
echo "relay at $RELAY"
echo "::endgroup::"

echo "::group::send marked requests"
# Through the tool-marked Anthropic route, as a configured Claude Code sends.
URL="$RELAY/__gate/t/claude-code/anthropic/v1/messages"
SENT=0
for spec in "chat-allow allow" "chat-flag flag" "chat-block block"; do
  set -- $spec
  # Against staging the verdict is the test org's own; only the step differs.
  "${CHECK[@]}" send "$1" "$URL" "$2" || { echo "could not send $1" >&2; exit 2; }
  SENT=$((SENT + 1))
done
echo "::endgroup::"

echo "::group::evidence"
E2E_RESULT="$(winpath "$OUT/evidence.json")" "${CHECK[@]}" evidence
ev=$?
echo "::endgroup::"

echo "::group::counts"
if [ "$BACKEND" = mock ]; then
  E2E_RESULT="$(winpath "$OUT/counts.json")" "${CHECK[@]}" counts ledger
  ct=$?
else
  # Waits on the driver hook that reads Connect's Overview pane.
  echo "::warning::counts against staging need Connect's displayed values (driver hook); skipped"
  ct=0
fi
echo "::endgroup::"

# The most actionable of the two: config, then routing, then unreachable.
for code in 2 1 3; do
  if [ "$ev" -eq "$code" ] || [ "$ct" -eq "$code" ]; then exit "$code"; fi
done
[ "$ev" -eq 0 ] && [ "$ct" -eq 0 ] || exit 2
echo "PASS: $SENT marked request(s) landed as expected ($BACKEND backend, $OS)"
