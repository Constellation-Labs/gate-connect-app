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
exec xvfb-run -a -s "-screen 0 1440x900x24" dbus-run-session -- \
  bash ci/e2e/app/windows-linux-signin.sh
