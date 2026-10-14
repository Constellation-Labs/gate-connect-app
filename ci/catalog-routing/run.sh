#!/usr/bin/env bash
# AG-906: run the catalog routing suite with this install's CA trusted
# machine-wide, so the scripted client validates the engine's leaves through
# the OS trust store the way a real app does.
#
# CI only. It writes the machine trust store (LocalMachine Root on Windows,
# the System keychain on macOS, the system anchors on Linux). Never run it on
# a workstation. `cleanup.sh` undoes it and the job runs that under always().
#
# Usage: run.sh   (from the repo root, after `cargo build -p gate-connect-cli`
#                  and `cargo test --no-run -p gate-connect-core --test catalog_routing`)
set -uo pipefail

if [ "${CI:-}" != "true" ]; then
  echo "refusing to run outside CI: this script changes the machine trust store" >&2
  exit 2
fi

# shellcheck source=ci/catalog-routing/env.sh
. "$(dirname "$0")/env.sh"
mkdir -p "$GATE_CONNECT_TEST_HOME" "$GATE_CONNECT_TEST_SECRETS"

fail=0
check() { # check <what> <status> [detail]
  if [ "$2" -eq 0 ]; then
    echo "PASS: $1"
  else
    echo "FAIL: $1"
    [ -n "${3:-}" ] && printf '%s\n' "$3" | sed 's/^/    /'
    fail=$((fail + 1))
  fi
}

# Every call that could raise a trust dialog is bounded: a dialog does not
# fail, it waits. Portable, since macOS has no `timeout`.
bounded() { # bounded <seconds> <cmd...>; output in $OUT, status returned
  local sec=$1; shift
  local out; out=$(mktemp)
  "$@" >"$out" 2>&1 &
  local pid=$!
  ( sleep "$sec"; kill -9 "$pid" 2>/dev/null ) &
  local watchdog=$!
  wait "$pid"; local rc=$?
  kill "$watchdog" 2>/dev/null; wait "$watchdog" 2>/dev/null
  OUT=$(cat "$out"); rm -f "$out"
  [ $rc -eq 137 ] && OUT="$OUT
(timed out after ${sec}s)"
  return $rc
}

bounded 60 "$CLI" proxy status
check 'status starts untrusted' "$(grep -q 'CA trust: not trusted' <<<"$OUT"; echo $?)" "$OUT"

bounded 120 "$CLI" proxy trust-ca --system-trust; rc=$?
check 'trust-ca --system-trust returns, with no dialog' $rc "$OUT"

# The machine store, read by the OS's own tool, not by our code.
case "$GC_OS" in
  windows)
    certutil -store Root "$CN" >/dev/null 2>&1
    check 'the CA is in the LocalMachine Root store' $?
    if certutil -user -store Root "$CN" >/dev/null 2>&1; then rc=1; else rc=0; fi
    check 'the CA is not in the per-user Root store' $rc
    ;;
  macos)
    security find-certificate -c "$CN" /Library/Keychains/System.keychain >/dev/null 2>&1
    check 'the CA is in the System keychain' $?
    security verify-cert -p ssl -c "$CA_CERT" >/dev/null 2>&1
    check 'security verify-cert -p ssl accepts it' $?
    ;;
  linux)
    test -f "/usr/local/share/ca-certificates/$CN.crt"
    check 'the anchor file is installed' $?
    body=$(sed -n '2p' "$CA_CERT")
    grep -qF "$body" /etc/ssl/certs/ca-certificates.crt
    check 'the system bundle carries the CA' $?
    ;;
esac

bounded 60 "$CLI" proxy status
check 'status sees the machine-wide install as trusted' "$(grep -q 'CA trust: trusted' <<<"$OUT"; echo $?)" "$OUT"

if [ $fail -gt 0 ]; then
  echo "$fail trust check(s) failed; not running the suite"
  exit 1
fi

GATE_CONNECT_ROUTING_OS_TRUST=1 cargo test --locked -p gate-connect-core --test catalog_routing -- --nocapture
