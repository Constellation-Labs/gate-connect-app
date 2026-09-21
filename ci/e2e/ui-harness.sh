#!/usr/bin/env bash
# Starts the UI-e2e harness (src-tauri/examples/ui-harness.rs) and the mocks it
# routes to. Launched by playwright.config.ts's `live` project; see
# `e2e/live/README.md`.
#
# Everything here exists so a CLICK in the real UI can reach a real gateway:
#
#   1. A wiped, throwaway home. The harness is one stateful process for a whole
#      run, so a run inheriting the last one's account and tool configs would
#      pass or fail for reasons written days ago.
#   2. A throwaway CA + 127.0.0.1 leaf, minted exactly as ci/e2e/run.sh does.
#      Only the relay validates the mock gateway's cert, and it trusts this CA
#      through GATE_CONNECT_TEST_CA - no OS trust store involved.
#   3. The HTTPS mock gateway (shared with run.sh) writing to a capture log, and
#      the plain-HTTP mock auth, which also sinks audit emits so a toggle does
#      not log a TLS failure per click.
#   4. Optionally, machine-wide trust for the app's OWN proxy CA. See below.
#
# Runs on Linux, macOS, and Windows (Git Bash), like ci/e2e/run.sh.
set -euo pipefail

OS="other"
case "$(uname -s)" in
  Linux) OS="Linux" ;;
  Darwin) OS="Darwin" ;;
  MINGW* | MSYS* | CYGWIN*) OS="Windows" ;;
esac

# Native path for consumers that aren't msys-aware (node, the Rust binaries).
winpath() { if [ "$OS" = "Windows" ]; then cygpath -w "$1"; else printf '%s' "$1"; fi; }

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# Under the system temp dir, NOT in the repo, and that is load-bearing on Linux:
# the helper daemon binds `$HOME/run/gate-connect/proxyd.sock`, and a Unix socket
# path is capped at ~108 bytes. A checkout a few directories deep blew that cap,
# and the only symptom was `connect_tool` reporting "proxy helper did not come up
# within ~2s" - the daemon's real error being "binding <path>" with no errno.
WORK="${GATE_UI_HARNESS_WORK:-${TMPDIR:-/tmp}/gc-ui-e2e}"
WORK="${WORK%/}"
MOCK_PORT="${GATE_UI_HARNESS_MOCK_PORT:-8453}"
AUTH_PORT="${GATE_UI_HARNESS_AUTH_PORT:-8454}"

rm -rf "$WORK"
mkdir -p "$WORK/home" "$WORK/secrets" "$WORK/ca"

# --- 1. Throwaway CA + leaf for the mock gateway. --------------------------
# Bare filenames from inside the dir, so it works with either the msys or the
# native openssl on Windows (run.sh learned this the hard way).
(
  cd "$WORK/ca" || exit 1
  openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout ca.key -out ca.pem \
    -subj "/CN=Gate Connect UI E2E CA" -days 2 \
    -addext "basicConstraints=critical,CA:TRUE" 2>/dev/null

  openssl req -newkey rsa:2048 -nodes \
    -keyout leaf.key -out leaf.csr \
    -subj "/CN=localhost" 2>/dev/null

  cat > leaf.ext <<'EXT'
subjectAltName=DNS:localhost,IP:127.0.0.1
extendedKeyUsage=serverAuth
EXT

  openssl x509 -req -in leaf.csr \
    -CA ca.pem -CAkey ca.key -CAcreateserial \
    -out leaf.pem -days 2 -extfile leaf.ext 2>/dev/null
)

CAPTURE="$WORK/capture.jsonl"
: > "$CAPTURE"
AUDIT_LOG="$WORK/audit-emits.jsonl"
: > "$AUDIT_LOG"

# --- 2. The mocks. ---------------------------------------------------------
CAPTURE_LOG="$(winpath "$CAPTURE")" MOCK_PORT="$MOCK_PORT" \
  MOCK_CERT="$(winpath "$WORK/ca/leaf.pem")" MOCK_KEY="$(winpath "$WORK/ca/leaf.key")" \
  node "$(winpath "$ROOT/ci/e2e/mock-gateway.mjs")" >"$WORK/mock.out" 2>&1 &
MOCK_PID=$!

MOCK_AUTH_PORT="$AUTH_PORT" MOCK_AUDIT_LOG="$(winpath "$AUDIT_LOG")" \
  node "$(winpath "$ROOT/ci/e2e/mock-auth.mjs")" >"$WORK/mock-auth.out" 2>&1 &
AUTH_PID=$!

# Not `exec`ed below, so these can be reaped: Playwright kills this script, and
# an exec would have orphaned both node processes on every run.
cleanup() { kill "$MOCK_PID" "$AUTH_PID" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

cargo build --locked -p gate-connect-desktop --example ui-harness

# --- 3. Trust the app's own proxy CA. --------------------------------------
# `GATE_UI_HARNESS_ROUTING=1` is the single opt-in for the routing arc, and it
# means one thing: this run may install a root CA on THIS MACHINE. Everything
# else the harness does stays inside $WORK, and `e2e/live/routing.spec.ts`
# skips itself without it - a suite that installed a root on every developer
# who ran `pnpm test:e2e` would be a poor trade for coverage.
#
# It is needed because `manager::enable()` calls `ca::ensure_trusted()`, which
# writes the distro trust store (Linux), the System keychain (macOS) or the
# LocalMachine root store (Windows).
#
# Best-effort, deliberately. `trust-ca --system-trust` is the promptless path
# built for headless hosts ("build agents, containers, servers") and escalates
# with `sudo -n`, which works on a CI runner and fails on a desktop with a
# sudo password. Failing here would then be wrong: `ensure_trusted()` has its
# own escalation (pkexec / the macOS auth dialog) that a desktop session can
# satisfy interactively, so the connect can still succeed. Let it try.
#
# CI removes the root again afterwards - `ui-e2e` in ci.yml owns that step, the
# same way `win-system-trust` does. A local run leaves it installed; that is
# what the opt-in is consenting to.
if [ "${GATE_UI_HARNESS_ROUTING:-0}" = "1" ]; then
  cargo build --locked -p gate-connect-cli
  CLI="$ROOT/target/debug/gate-connect"
  [ -x "$CLI" ] || CLI="$CLI.exe"
  GATE_CONNECT_TEST_HOME="$(winpath "$WORK/home")" \
  GATE_CONNECT_TEST_SECRETS="$(winpath "$WORK/secrets")" \
    "$CLI" proxy trust-ca --system-trust \
    || echo "note: promptless CA trust failed; enable() will try its own escalation"
fi

BIN="$ROOT/target/debug/examples/ui-harness"
[ -x "$BIN" ] || BIN="$BIN.exe"

# The secrets seam keeps this off the real keychain, which on macOS would
# otherwise prompt per rebuild (see CLAUDE.md, "Running the app locally"); the
# home seam keeps every tool config, CA and helper socket the run creates
# inside $WORK.
env \
  GATE_CONNECT_TEST_HOME="$(winpath "$WORK/home")" \
  GATE_CONNECT_TEST_SECRETS="$(winpath "$WORK/secrets")" \
  GATE_CONNECT_TEST_CA="$(winpath "$WORK/ca/ca.pem")" \
  GATE_UI_HARNESS_CAPTURE="$CAPTURE" \
  GATE_CONNECT_TEST_AUDIT_ENDPOINT="http://127.0.0.1:$AUTH_PORT/audit/emit" \
  GATE_CONNECT_TEST_TOKEN_ENDPOINT="http://127.0.0.1:$AUTH_PORT/oauth2/token" \
  GATE_CONNECT_TEST_ORGS_ENDPOINT="http://127.0.0.1:$AUTH_PORT/v1/me/orgs" \
  GATE_UI_HARNESS_PORT="${GATE_UI_HARNESS_PORT:-5610}" \
  "$BIN"
