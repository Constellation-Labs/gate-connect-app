#!/usr/bin/env bash
# Remove the machine-wide CA run.sh installed. Runs under always(), so a run
# that failed midway does not leave a trusted root on the runner. Best-effort
# throughout: the CLI path first, then the raw store delete as a backstop.
set -u

# CI only, like run.sh: it deletes every CA named "Gate Connect Local CA",
# which is also the production CN, so a workstation run would remove the
# real install's trust.
if [ "${CI:-}" != "true" ]; then
  echo "refusing to run outside CI: this script changes the machine trust store" >&2
  exit 2
fi

# shellcheck source=ci/catalog-routing/env.sh
. "$(dirname "$0")/env.sh"

[ -x "$CLI" ] && "$CLI" proxy untrust-ca --system-trust || true

case "$GC_OS" in
  windows) certutil -delstore Root "$CN" >/dev/null 2>&1 || true ;;
  macos)
    while sudo -n security delete-certificate -c "$CN" /Library/Keychains/System.keychain >/dev/null 2>&1; do :; done
    ;;
  linux)
    sudo -n rm -f "/usr/local/share/ca-certificates/$CN.crt"
    sudo -n update-ca-certificates --fresh >/dev/null 2>&1 || true
    ;;
esac

# Report, do not fail: the runner is throwaway, and a red cleanup would hide
# the real result.
case "$GC_OS" in
  windows) certutil -store Root "$CN" >/dev/null 2>&1 && echo "WARNING: CA still in LocalMachine Root" ;;
  macos) security find-certificate -c "$CN" /Library/Keychains/System.keychain >/dev/null 2>&1 && echo "WARNING: CA still in the System keychain" ;;
  linux) [ -f "/usr/local/share/ca-certificates/$CN.crt" ] && echo "WARNING: anchor still installed" ;;
esac
echo "cleanup done"
exit 0
