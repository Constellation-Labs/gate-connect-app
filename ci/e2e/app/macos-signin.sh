#!/usr/bin/env bash
# AG-905 spike: install the built macOS bundle, launch it, and sign in with an
# API key through the app's own window, driven by accessibility (macos-ax.js).
#
# CI runners only. It edits /etc/hosts, trusts a root in the System keychain,
# binds port 443 and installs into /Applications, none of which belongs on a
# developer machine.
#
# The app is the release-profile bundle from bundle.yml, so every test seam is
# compiled out (`env::test_seam` honors them only under debug_assertions) and
# nothing can point it at a mock from the environment. It talks to its built-in
# default gateway, so this makes that hostname the mock instead:
#
#   1. a throwaway CA and a leaf for the default gateway host, the CA trusted in
#      the System keychain (rustls-platform-verifier reads the OS store);
#   2. /etc/hosts sends that host to 127.0.0.1, so nothing the app sends can
#      reach the real gateway, whatever it decides to call;
#   3. ci/e2e/mock-gateway.mjs on 443 with that leaf, logging every request.
#
# Then: install from the dmg, launch, walk the intro, "Use an API key", type a made-up key,
# "Connect and continue", and assert the signed-in state twice: the window moves
# on to "Name this device", and account.json carries the key's prefix.
#
# Env: BUNDLE_DIR (a .dmg somewhere under it), OUT (screenshots, AX dumps, logs).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
OUT="${OUT:-$PWD/app-e2e-out}"
BUNDLE_DIR="${BUNDLE_DIR:?BUNDLE_DIR must name the directory holding the .dmg}"
mkdir -p "$OUT"

GATEWAY_HOST="gateway.constellationgate.ai"
APP="/Applications/Gate Connect.app"
DATA="$HOME/Library/Application Support/Gate Connect"
KEY="sk-gw-e2e-00000000000000000000000000000000"
AX=(osascript -l JavaScript "$HERE/macos-ax.js")

WORK="$(mktemp -d)"
STEP="setup"
on_exit() {
  local rc=$?
  if [ "$rc" -ne 0 ]; then
    echo "::error::app e2e failed at step: $STEP"
    screencapture -x "$OUT/failed-$STEP.png" || echo "screencapture failed"
    "${AX[@]}" dump >"$OUT/failed-$STEP.ax.txt" 2>&1 || true
  fi
  cp "$WORK/capture.jsonl" "$OUT/" 2>/dev/null || true
  cp "$WORK/mock.out" "$OUT/" 2>/dev/null || true
  cp -R "$DATA" "$OUT/app-data" 2>/dev/null || true
  # The app's own log, wherever this build writes it.
  cp -R "$HOME/Library/Logs/ai.constellation.gate-connect" "$OUT/app-logs" 2>/dev/null || true
  [ -n "${MOCK_PID:-}" ] && sudo kill "$MOCK_PID" 2>/dev/null || true
  exit "$rc"
}
trap on_exit EXIT

step() {
  STEP="$1"
  echo "::group::$1"
}
endstep() { echo "::endgroup::"; }

step "accessibility-probe"
"${AX[@]}" probe
endstep

step "mock-gateway"
(
  cd "$WORK"
  openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.pem \
    -subj "/CN=Gate Connect App E2E CA" -days 2 \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -keyout leaf.key -out leaf.csr \
    -subj "/CN=$GATEWAY_HOST" 2>/dev/null
  printf 'subjectAltName=DNS:%s\nextendedKeyUsage=serverAuth\n' "$GATEWAY_HOST" >leaf.ext
  openssl x509 -req -in leaf.csr -CA ca.pem -CAkey ca.key -CAcreateserial \
    -out leaf.pem -days 2 -extfile leaf.ext 2>/dev/null
)
sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain "$WORK/ca.pem"
echo "127.0.0.1 $GATEWAY_HOST" | sudo tee -a /etc/hosts >/dev/null
sudo dscacheutil -flushcache
sudo killall -HUP mDNSResponder || true
: >"$WORK/capture.jsonl"
sudo env CAPTURE_LOG="$WORK/capture.jsonl" MOCK_PORT=443 \
  MOCK_CERT="$WORK/leaf.pem" MOCK_KEY="$WORK/leaf.key" \
  "$(command -v node)" "$ROOT/ci/e2e/mock-gateway.mjs" >"$WORK/mock.out" 2>&1 &
MOCK_PID=$!
for _ in $(seq 1 50); do
  if curl -fsS --cacert "$WORK/ca.pem" "https://$GATEWAY_HOST/health" >/dev/null 2>&1; then break; fi
  sleep 0.2
done
# The name resolves to the mock and the leaf verifies against the throwaway CA.
curl -fsS --cacert "$WORK/ca.pem" "https://$GATEWAY_HOST/health"
echo
# And the OS trust store accepts it, which is what the app's TLS stack asks.
security verify-cert -c "$WORK/leaf.pem" -p ssl -s "$GATEWAY_HOST"
: >"$WORK/capture.jsonl"
endstep

step "install"
DMG="$(find "$BUNDLE_DIR" -name "*.dmg" | head -1)"
[ -n "$DMG" ] || { echo "no .dmg under $BUNDLE_DIR" >&2; exit 1; }
echo "installing from $DMG"
MNT="$(mktemp -d)"
hdiutil attach -nobrowse -readonly -mountpoint "$MNT" "$DMG" >/dev/null
sudo rm -rf "$APP"
cp -R "$MNT/Gate Connect.app" /Applications/
hdiutil detach "$MNT" >/dev/null
codesign -dv "$APP" 2>&1 | sed -n '1,6p'
endstep

step "launch"
open "$APP"
"${AX[@]}" wait-window 90
"${AX[@]}" dump >"$OUT/01-launched.ax.txt" 2>&1 || true
screencapture -x "$OUT/01-launched.png" || true
endstep

# A first launch opens the intro window: a welcome frame and three steps, each
# with Next, the last with "Get started", which closes it.
step "intro"
"${AX[@]}" wait-text 60 "Welcome to Gate Connect"
for n in 2 3 4; do
  "${AX[@]}" click 30 "Next"
  "${AX[@]}" wait-text 30 "Step $n of 4"
done
"${AX[@]}" click 30 "Get started"
endstep

step "welcome"
"${AX[@]}" click 60 "Use an API key"
endstep

step "api-key-pane"
"${AX[@]}" wait-text 30 "Paste a Gate API key"
"${AX[@]}" type 30 "API key" "$KEY"
"${AX[@]}" click 30 "Connect and continue"
endstep

step "signed-in"
"${AX[@]}" wait-text 60 "Name this device"
screencapture -x "$OUT/02-signed-in.png" || true
"${AX[@]}" dump >"$OUT/02-signed-in.ax.txt" 2>&1 || true
# The app's own record of the sign-in: the key's first 12 characters, never the
# key, beside the gateway it was saved against.
python3 - "$DATA/account.json" "$KEY" "$GATEWAY_HOST" <<'PY'
import json, sys
path, key, host = sys.argv[1:]
acct = json.load(open(path))
print(json.dumps(acct, indent=2))
assert acct.get("api_key_prefix") == key[:12], acct
assert host in acct.get("gateway_base_url", ""), acct
assert key not in open(path).read(), "the key itself is on disk"
PY
# And the app used it: the mock (and only the mock, since the host resolves to
# 127.0.0.1) received the dashboard's own reads carrying this key.
python3 - "$WORK/capture.jsonl" "$KEY" <<'PY'
import json, sys
path, key = sys.argv[1:]
reqs = [json.loads(l) for l in open(path) if l.strip()]
keyed = [r["path"] for r in reqs if r["headers"].get("x-gate-api-key") == key]
print(f"{len(reqs)} requests reached the mock gateway, {len(keyed)} with the key: {sorted(set(keyed))}")
assert keyed, "no request reached the mock gateway with the key"
PY
endstep

echo "PASS: signed in with an API key through the installed app's own window"
