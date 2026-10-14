# Shared by run.sh and cleanup.sh. Sourced, not executed.
# The CA's cert and key live under a throwaway home; only the machine trust
# store write in run.sh touches the real runner.
case "$(uname -s)" in
  Darwin) GC_OS=macos ;;
  Linux) GC_OS=linux ;;
  MINGW*|MSYS*|CYGWIN*) GC_OS=windows ;;
  *) echo "unsupported OS: $(uname -s)" >&2; exit 2 ;;
esac

WORK="${RUNNER_TEMP:?RUNNER_TEMP is set on every Actions runner}/gc-catalog-routing"
CLI="$PWD/target/debug/gate-connect"
if [ "$GC_OS" = windows ]; then
  WORK="$(cygpath -u "$WORK")"
  CLI="$CLI.exe"
fi
CN='Gate Connect Local CA'

# What the binaries read. Windows programs want a Windows path.
if [ "$GC_OS" = windows ]; then
  GATE_CONNECT_TEST_HOME="$(cygpath -w "$WORK/home")"
  GATE_CONNECT_TEST_SECRETS="$(cygpath -w "$WORK/secrets")"
else
  GATE_CONNECT_TEST_HOME="$WORK/home"
  GATE_CONNECT_TEST_SECRETS="$WORK/secrets"
fi
export GATE_CONNECT_TEST_HOME GATE_CONNECT_TEST_SECRETS

# Where `env::ca_material_dir` puts the certificate under the test home.
CA_CERT="$WORK/home/app-support/Gate Connect/proxy/ca-cert.pem"
