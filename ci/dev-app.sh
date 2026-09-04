#!/usr/bin/env bash
#
# Run the desktop app for local development without the macOS keychain.
#
# WHY THIS EXISTS. `pnpm app` runs `tauri dev`, which rebuilds the Rust binary on
# every change. macOS scopes a keychain item's ACL to the *binary that created
# it*, so each rebuild produces an executable the login keychain has never seen
# and the OS re-prompts for permission. "Always Allow" resolves it for exactly
# one build and then the next `cargo run` invalidates it again, and answering it
# needs the login password, which is not something a dev loop should ask for
# several times an hour.
#
# Denying is worse than annoying. Gate Connect treats an unreadable key as an
# unusable account and falls back to the built-in default gateway, REWRITING
# `account.json` on the way. A developer pointed at staging who dismisses the
# prompt is silently moved back to the production URL, which is how this script
# came to be written.
#
# WHAT IT DOES. Sets `GATE_CONNECT_TEST_SECRETS`, the seam in
# `crates/core/src/keychain.rs` that backs secrets with files instead of the OS
# store. The key is entered once and persists across rebuilds, and the keychain
# is never touched, so the prompt cannot appear.
#
# THE TRADE, STATED PLAINLY. The Gate key is then a plaintext file rather than a
# keychain item. That is a real reduction in protection and the reason this is a
# dev script and not a default: `GATE_CONNECT_TEST_SECRETS` is unset in every
# shipped build, so released copies always use the real keychain. Use a staging
# key here, never a production one.
#
# The store lives OUTSIDE the repository (`~/.gate-connect-dev/secrets`) so it
# cannot be committed by a stray `git add -A`, whatever .gitignore happens to
# say at the time.
#
# USAGE
#   ci/dev-app.sh                      # staging (the default for dev work)
#   ci/dev-app.sh http://localhost:3000  # a gateway you are running yourself
#
# The first launch has no key and asks for one. Paste it once; later launches
# read it from the file.

set -euo pipefail

GATEWAY="${1:-https://gateway-staging.constellationgate.ai}"
SECRETS_DIR="${GATE_CONNECT_DEV_SECRETS:-$HOME/.gate-connect-dev/secrets}"

mkdir -p "$SECRETS_DIR"
chmod 700 "$SECRETS_DIR"

# Also set the BUILD-time default. `account.json` is the source of truth, but
# when it is absent or unusable the app falls back to `VITE_GATE_DEFAULT_BASE_URL`
# (see src/lib/config.ts), which ships as production. Leaving that unset is what
# lets a bad session quietly repoint a developer at prod.
export VITE_GATE_DEFAULT_BASE_URL="$GATEWAY"
export GATE_CONNECT_TEST_SECRETS="$SECRETS_DIR"

echo "Gate Connect dev"
echo "  gateway : $GATEWAY"
echo "  secrets : $SECRETS_DIR (files, not the keychain)"
if [ -z "$(ls -A "$SECRETS_DIR" 2>/dev/null)" ]; then
  echo "  note    : store is empty, so the app will ask for a key. Use a STAGING key."
fi
echo

exec pnpm app
