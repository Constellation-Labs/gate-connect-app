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

# Executable suffix, by name rather than by probing for it. `[ -x "$BIN" ]`
# looks like the safe way to add `.exe` only when needed, and is not: MSYS
# stat() silently falls back to `NAME.exe` when `NAME` is missing, so the test
# passes for a path that does not literally exist and the suffix never gets
# appended. Bash then papers over it, because its exec does the same fallback -
# which is why `"$CLI" proxy trust-ca` below ran fine on Windows. `env` does
# not, so the extensionless harness path reached it as a command that is not
# there: a bare 127, no message, and Playwright reporting only "Process from
# config.webServer was not able to start". Naming the suffix was right and was
# not the whole story: the 127 survived it, because a bare 127 is also what Git
# Bash reports for a native exe that died with an NTSTATUS. The preflight
# below read that one out as STATUS_ENTRYPOINT_NOT_FOUND: an example target
# gets no application manifest, so it was handed the 5.82 comctl32, which does
# not export the `TaskDialogIndirect` the desktop library imports. Fixed in
# `src-tauri/build.rs`; the preflight stays, for the next silent 127.
EXE=""
if [ "$OS" = "Windows" ]; then EXE=".exe"; fi

if [ "$OS" = "Windows" ]; then
  # Git Bash rewrites anything that looks like a path, which corrupts openssl's
  # `-subj "/CN=..."` into a Windows path. `ci/e2e/run.sh` documents the same
  # guard; dropping it here made the CA subject nonsense and the leaf unusable.
  export MSYS2_ARG_CONV_EXCL="*"
  export MSYS_NO_PATHCONV=1
fi

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

# 0700, and that is not hygiene. With GATE_UI_HARNESS_ROUTING the run installs
# a root CA whose PRIVATE KEY the secrets seam writes here as a plain file; in a
# world-writable temp dir at a predictable path, any other local user could read
# it and mint certificates every browser on the machine would trust. The `-m` is
# on the create so there is no window between mkdir and chmod.
rm -rf "$WORK"
if [ "$OS" = "Windows" ]; then
  # `mkdir -m` is a chmod folded into the create, and that chmod fails outright
  # under Git Bash - "cannot change permissions of '/tmp/gc-ui-e2e': Permission
  # denied" - which took the whole Windows run down before the first spec ran.
  # The mode has nothing to protect there anyway: Git Bash maps /tmp to the
  # per-user %TEMP%, already ACL'd to the one account, not a shared world-
  # writable /tmp. The paragraph above is a Unix threat model.
  mkdir -p "$WORK"
  mkdir -p "$WORK/home" "$WORK/secrets" "$WORK/ca"
else
  mkdir -m 700 -p "$WORK"
  mkdir -m 700 -p "$WORK/home" "$WORK/secrets" "$WORK/ca"
fi

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
# On Windows, everything this script starts has to be told when to die, and
# `exec`ing the harness below is half of the same fix. Playwright's webServer
# teardown is `taskkill /T /F` on the shell it spawned, then a wait for the
# child's stdout/stderr pipes to close. `/T` walks parent PIDs, and a Cygwin
# process that `exec`s a native one exits its own stub: `node … &` is a fork
# whose child execs node, and `env … "$BIN"` was env execing the harness, so
# both landed with a dead parent and outside the tree taskkill sees. The
# harness survived with the pipe, and Playwright waited on it until the job
# hit its 30-minute limit - 19 minutes after the last test had passed. This
# process's own Windows PID is the one thing taskkill reliably kills, so the
# mocks and the harness each watch it and follow it down. Unix needs neither:
# Playwright kills the whole process group there.
if [ "$OS" = "Windows" ]; then
  MOCK_EXIT_WITH_PID="$(cat "/proc/$$/winpid")"
  export MOCK_EXIT_WITH_PID
fi

CAPTURE_LOG="$(winpath "$CAPTURE")" MOCK_PORT="$MOCK_PORT" \
  MOCK_CERT="$(winpath "$WORK/ca/leaf.pem")" MOCK_KEY="$(winpath "$WORK/ca/leaf.key")" \
  node "$(winpath "$ROOT/ci/e2e/mock-gateway.mjs")" >"$WORK/mock.out" 2>&1 &
MOCK_PID=$!

MOCK_AUTH_PORT="$AUTH_PORT" MOCK_AUDIT_LOG="$(winpath "$AUDIT_LOG")" \
  node "$(winpath "$ROOT/ci/e2e/mock-auth.mjs")" >"$WORK/mock-auth.out" 2>&1 &
AUTH_PID=$!

# For a manual run ended by Ctrl+C before the harness is up, or a failure on
# the way there. Once the harness is `exec`ed this trap is gone with the shell;
# by then Playwright's group kill (Unix) or the PID watch above (Windows) is
# what ends the mocks.
cleanup() { kill "$MOCK_PID" "$AUTH_PID" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

# Supplied by playwright.live.config.ts and inherited through the environment;
# generated here only for a manual run, which then has no test to share it with.
# The harness refuses to start without it: `/invoke` reaches every command the
# app has, and a loopback port with no token is reachable from any page in the
# developer's browser.
if [ -z "${GATE_UI_HARNESS_TOKEN:-}" ]; then
  GATE_UI_HARNESS_TOKEN="$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
  export GATE_UI_HARNESS_TOKEN
fi

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
  CLI="$ROOT/target/debug/gate-connect$EXE"
  GATE_CONNECT_TEST_HOME="$(winpath "$WORK/home")" \
  GATE_CONNECT_TEST_SECRETS="$(winpath "$WORK/secrets")" \
    "$CLI" proxy trust-ca --system-trust \
    || echo "note: promptless CA trust failed; enable() will try its own escalation"
fi

BIN="$ROOT/target/debug/examples/ui-harness$EXE"

# --- Preflight, Windows only. ----------------------------------------------
# A native exe that dies with an NTSTATUS - a DLL it cannot find, an entry
# point a system DLL lacks, a Rust abort, a stack overflow - is reported by
# Git Bash as exit 127 with nothing on stderr, the same as "command not
# found": Cygwin's `status_exit` folds every 0xC0000000 status other than an
# access violation or illegal instruction into 127, and the one message it
# prints (for a missing DLL) goes to the console rather than the pipe.
# Playwright then has one sentence to offer, and it is not a diagnosis. So the
# harness runs once here with the token withheld, which it refuses with exit 2
# and a sentence (`required_env`, reached only after the mock runtime has
# built): that exit proves the exe loads and gets as far as reading its
# environment. Anything else is read out the way Windows reports it - the raw
# status from a native parent (PowerShell hands the child's code through
# untouched), the child's own stderr, the exe's import table from dumpbin where
# Visual Studio is installed, and the last few minutes of error-level
# application events - before failing with a message of our own.
if [ "$OS" = "Windows" ]; then
  BINW="$(winpath "$BIN")"
  set +e
  env -u GATE_UI_HARNESS_TOKEN \
    GATE_CONNECT_TEST_HOME="$(winpath "$WORK/home")" \
    GATE_CONNECT_TEST_SECRETS="$(winpath "$WORK/secrets")" \
    "$BIN"
  PRE=$?
  set -e
  if [ "$PRE" -ne 2 ]; then
    echo "ui-harness preflight: expected exit 2 with the token withheld, got $PRE" >&2
    ls -la "$BIN" >&2 || true
    # Base64 of UTF-16LE rather than a quoted `-Command`: the script has to
    # cross bash, the MSYS argument rewriter and PowerShell's own parser, and
    # each has a different idea of what a quote is. `-EncodedCommand` skips all
    # three. The seams stay set and the token stays withheld, so this is the
    # same run as the one above, minus Cygwin's translation of its exit.
    PS_PROBE='
      "session: " + (Get-Process -Id $PID).SessionId + ", interactive: " + [Environment]::UserInteractive
      $p = Start-Process -FilePath $env:GC_BINW -Wait -PassThru -NoNewWindow -RedirectStandardError $env:GC_PRE_ERR
      "raw exit code: 0x{0:X8} ({1})" -f ($p.ExitCode -band 0xFFFFFFFF), $p.ExitCode
      "child stderr:"
      Get-Content $env:GC_PRE_ERR -ErrorAction SilentlyContinue
      "error-level application events, last 10 minutes:"
      Get-WinEvent -FilterHashtable @{LogName="Application"; Level=2; StartTime=(Get-Date).AddMinutes(-10)} -ErrorAction SilentlyContinue |
        Format-List TimeCreated, ProviderName, Message
    '
    GC_BINW="$BINW" GC_PRE_ERR="$(winpath "$WORK/preflight.err")" \
    env -u GATE_UI_HARNESS_TOKEN \
      GATE_CONNECT_TEST_HOME="$(winpath "$WORK/home")" \
      GATE_CONNECT_TEST_SECRETS="$(winpath "$WORK/secrets")" \
      powershell.exe -NoProfile -EncodedCommand \
        "$(printf '%s' "$PS_PROBE" | iconv -f UTF-8 -t UTF-16LE | base64 -w0)" >&2 || true
    VSWHERE="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
    if [ -x "$VSWHERE" ]; then
      DUMPBIN="$("$VSWHERE" -latest -find '**\dumpbin.exe' 2>/dev/null | head -1 | tr -d '\r')"
      if [ -n "$DUMPBIN" ]; then
        "$(cygpath -u "$DUMPBIN")" /dependents "$BINW" >&2 || true
      fi
    fi
    exit 1
  fi
fi

# The secrets seam keeps this off the real keychain, which on macOS would
# otherwise prompt per rebuild (see CLAUDE.md, "Running the app locally"); the
# home seam keeps every tool config, CA and helper socket the run creates
# inside $WORK.
export GATE_CONNECT_TEST_HOME="$(winpath "$WORK/home")"
export GATE_CONNECT_TEST_SECRETS="$(winpath "$WORK/secrets")"
export GATE_CONNECT_TEST_CA="$(winpath "$WORK/ca/ca.pem")"
export GATE_UI_HARNESS_CAPTURE="$(winpath "$CAPTURE")"
export GATE_CONNECT_TEST_AUDIT_ENDPOINT="http://127.0.0.1:$AUTH_PORT/audit/emit"
export GATE_CONNECT_TEST_TOKEN_ENDPOINT="http://127.0.0.1:$AUTH_PORT/oauth2/token"
export GATE_CONNECT_TEST_ORGS_ENDPOINT="http://127.0.0.1:$AUTH_PORT/v1/me/orgs"
export GATE_UI_HARNESS_PORT="${GATE_UI_HARNESS_PORT:-5610}"
export GATE_UI_HARNESS_TOKEN
if [ "$OS" = "Windows" ]; then
  export GATE_UI_HARNESS_EXIT_WITH_PID="$MOCK_EXIT_WITH_PID"
fi

# `exec`, not `env`: see the note above the mocks. This shell's parent is the
# native cmd.exe Playwright spawned, so on exec the shell stays as the stub
# Windows knows and the harness is created as ITS child - inside the tree
# `taskkill /T` walks. Through `env`, a Cygwin process whose parent is also
# Cygwin, the stub exits and the harness is orphaned. On Unix it changes
# nothing that matters: the process group is what gets killed.
exec "$BIN"
