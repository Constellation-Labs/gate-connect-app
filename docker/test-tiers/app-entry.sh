#!/usr/bin/env bash
# Entry point of the `app` image (docker/test-tiers/Dockerfile, AG-909).
#
# Runs the installed-app sign-in exactly as the `app-e2e-linux` CI job does,
# under a virtual display and a fresh session bus:
#
#   xvfb-run -a -s "-screen 0 1440x900x24" dbus-run-session -- \
#     bash ci/e2e/app/windows-linux-signin.sh
#
# scripts/test-tiers.sh starts this container with --network none, the
# checkout read-only at /src, the bundle read-only at /bundle and a writable
# /out for screenshots, page dumps and logs. Everything the script changes
# (the hosts file, the CA anchors, port 443, the installed app) is this
# container's, and is thrown away with it.
set -euo pipefail

SRC="${SRC:-/src}"
export BUNDLE_DIR="${BUNDLE_DIR:-/bundle}"
export OUT="${OUT:-/out}"

[ -f "$SRC/ci/e2e/app/windows-linux-signin.sh" ] || {
  echo "no checkout at $SRC: mount the repo read-only there" >&2
  exit 2
}

cd "$SRC"
# Not exec'd: xvfb-run as PID 1 waits forever for Xvfb's readiness signal.
# scripts/test-tiers.sh also passes --init, so PID 1 is docker-init.
# Bounded, so a hang reads as a failure with the step it reached, not as a
# CI job that times out with nothing said. The signin takes about a minute
# natively and two under emulation.
LIMIT="${GC_APP_TIER_TIMEOUT:-900}"
rc=0
timeout -k 10 "$LIMIT" xvfb-run -a -s "-screen 0 1440x900x24" dbus-run-session -- \
  bash ci/e2e/app/windows-linux-signin.sh || rc=$?
if [ "$rc" -eq 124 ] || [ "$rc" -eq 137 ]; then
  echo "app e2e (linux) timed out after ${LIMIT}s; the last ::group:: above is the step it was in" >&2
fi
# The app writes its data 0600 as this container's user, which is not the
# host's. Everything here is mock data, so let the host read it.
chmod -R a+rX "$OUT" 2>/dev/null || true
exit "$rc"
