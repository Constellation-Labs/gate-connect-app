#!/usr/bin/env bash
# Run the pull-request test tiers locally against the mock gateway (AG-909).
#
#   bash scripts/test-tiers.sh                  all four tiers
#   bash scripts/test-tiers.sh --tier daemon    one tier (repeatable)
#
# Tiers, each the commands its CI job runs (.github/workflows/ci.yml):
#
#   workspace  `rust` and `web` jobs: cargo fmt/clippy/test --workspace,
#              pnpm build/test/typecheck:e2e/test:e2e. On this host.
#   daemon     the helper daemon tests (crates/cli/tests/helper_daemon.rs).
#              Linux only, so a macOS or Windows host runs them in the
#              `daemon` container (docker/test-tiers).
#   app        the installed-app sign-in (`app-e2e-linux`). Never on the host:
#              always in the `app` container, with --network none. The
#              macOS and Windows drivers need a disposable VM; see
#              docs/testing.md.
#   routing    the catalog routing assertions in the default (pinned CA)
#              mode. The OS-trust mode writes the machine trust store and
#              stays CI-only, as ci/catalog-routing/run.sh enforces.
#
# No credentials: every tier talks to loopback mocks. The app tier needs a
# Linux .deb, from --bundle DIR, --from-run ID (a CI run's bundle-ubuntu-22.04
# artifact, through `gh`), or, by default, the newest green CI run of the
# current branch.
#
# Exit 0 when every selected tier passed, 1 otherwise. A failure names the
# tier and the step, and the log that holds it.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="Constellation-Labs/gate-connect-app"
ALL_TIERS=(workspace daemon app routing)

# Overridable so parallel checkouts do not share build caches.
IMAGE="${GC_TIERS_IMAGE:-gate-connect-test-tiers}"
VOL="${GC_TIERS_VOLUME_PREFIX:-gc-test-tiers-}"

usage() {
  cat <<'EOF'
Usage: bash scripts/test-tiers.sh [options]

  --tier NAME     run only this tier: workspace, daemon, app, routing
                  (repeatable, or comma separated)
  --bundle DIR    a directory holding the Linux .deb for the app tier
  --from-run ID   download the .deb from that CI run (needs `gh auth login`)
  --fail-fast     stop at the first failed tier
  --list          print the tiers and exit
  -h, --help      this text

Env: GC_TIERS_IMAGE (image name), GC_TIERS_VOLUME_PREFIX (named volume
prefix), BUNDLE_DIR (same as --bundle).
EOF
}

TIERS=()
BUNDLE="${BUNDLE_DIR:-}"
FROM_RUN=""
FAIL_FAST=0
while [ $# -gt 0 ]; do
  case "$1" in
    --tier) shift; IFS=',' read -r -a t <<<"${1:-}"; TIERS+=("${t[@]}") ;;
    --tier=*) IFS=',' read -r -a t <<<"${1#--tier=}"; TIERS+=("${t[@]}") ;;
    --bundle) shift; BUNDLE="${1:-}" ;;
    --bundle=*) BUNDLE="${1#--bundle=}" ;;
    --from-run) shift; FROM_RUN="${1:-}" ;;
    --from-run=*) FROM_RUN="${1#--from-run=}" ;;
    --fail-fast) FAIL_FAST=1 ;;
    --list) printf '%s\n' "${ALL_TIERS[@]}"; exit 0 ;;
    -h | --help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done
[ ${#TIERS[@]} -gt 0 ] || TIERS=("${ALL_TIERS[@]}")
for t in "${TIERS[@]}"; do
  case " ${ALL_TIERS[*]} " in *" $t "*) ;; *) echo "unknown tier: $t (one of: ${ALL_TIERS[*]})" >&2; exit 2 ;; esac
done

case "$(uname -s)" in
  Darwin) HOST=macos ;;
  Linux) HOST=linux ;;
  MINGW* | MSYS* | CYGWIN*) HOST=windows ;;
  *) echo "unsupported host: $(uname -s)" >&2; exit 2 ;;
esac

RUN_DIR="$ROOT/target/test-tiers/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$RUN_DIR"

# A path docker can mount: Docker Desktop on Windows wants the native form,
# and Git Bash must not rewrite the -v argument on the way.
hostpath() { if [ "$HOST" = windows ]; then cygpath -w "$1"; else printf '%s' "$1"; fi; }
docker_() { if [ "$HOST" = windows ]; then MSYS_NO_PATHCONV=1 docker "$@"; else docker "$@"; fi; }

# A CI runner has no proxy, and a developer's may be a live Gate Connect.
# Tests run without one, so nothing they send can reach it.
noproxy() {
  env -u HTTP_PROXY -u HTTPS_PROXY -u http_proxy -u https_proxy -u ALL_PROXY -u all_proxy \
    -u GATE_CONNECT_ROUTING_OS_TRUST "$@"
}

# ---------------------------------------------------------------- steps ----
TIER="" FAILED_STEP="" FAILED_LOG="" NOTES=()
note() { NOTES+=("$TIER: $*"); echo "note: $*"; }

# step <name> <cmd...>: run one step, log it, and on failure remember which.
step() {
  local name="$1"; shift
  local log="$RUN_DIR/$TIER-$name.log" start rc
  start=$(date +%s)
  echo "--- [$TIER] $name"
  ( cd "$ROOT" && "$@" ) 2>&1 | tee "$log"
  rc=${PIPESTATUS[0]}
  if [ "$rc" -ne 0 ]; then
    FAILED_STEP="$name" FAILED_LOG="$log"
    echo "FAIL: tier $TIER, step $name (exit $rc, $(( $(date +%s) - start ))s); log: $log"
    return 1
  fi
  echo "ok: [$TIER] $name ($(( $(date +%s) - start ))s)"
}

# ---------------------------------------------------------------- tiers ----
tier_workspace() {
  # The `rust` job. fmt runs on one CI leg only; it is platform-independent.
  step sidecar bash scripts/stage-sidecar.sh || return 1
  step rustfmt cargo fmt --all -- --check || return 1
  step clippy cargo clippy --locked --workspace --all-targets -- -D warnings || return 1
  step cargo-test noproxy cargo test --locked --workspace || return 1
  # The `web` job.
  step pnpm-install pnpm install --frozen-lockfile || return 1
  step pnpm-build pnpm build || return 1
  step vitest noproxy pnpm test || return 1
  step typecheck-e2e pnpm typecheck:e2e || return 1
  step playwright-install pnpm exec playwright install chromium || return 1
  step ui-e2e noproxy pnpm test:e2e || return 1
  note "the ui-e2e job's live-backend routing arc (GATE_UI_HARNESS_ROUTING=1) installs a root CA and stays CI-only"
}

daemon_in_container() {
  local img="$IMAGE:daemon"
  local mounts=(-v "$(hostpath "$ROOT"):/src:ro"
    -v "${VOL}cargo-registry:/opt/cargo/registry"
    -v "${VOL}cargo-git:/opt/cargo/git"
    -v "${VOL}daemon-target:/target")
  step image docker_ build --target daemon -t "$img" docker/test-tiers || return 1
  # Crates are fetched with the network on; the build and the tests run with
  # it off, so they cannot reach anything but the container's loopback.
  step fetch docker_ run --rm "${mounts[@]}" "$img" cargo fetch --locked || return 1
  step helper-daemon docker_ run --rm --network none "${mounts[@]}" "$img" \
    cargo test --locked -p gate-connect-cli --test helper_daemon || return 1
}

tier_daemon() {
  if [ "$HOST" = linux ]; then
    step helper-daemon noproxy cargo test --locked -p gate-connect-cli --test helper_daemon || return 1
  else
    note "Linux-only tests: run in the daemon container (Ubuntu 22.04), not on this $HOST host"
    daemon_in_container || return 1
  fi
}

# Prints "bundle-dir: <dir>" for the directory holding the .deb. It runs as a
# step, in a subshell, so tier_app reads the answer back from the step's log.
resolve_bundle() {
  local id sha DEB_DIR DEB
  if [ -n "$BUNDLE" ]; then
    DEB_DIR="$(cd "$BUNDLE" 2>/dev/null && pwd)" || { echo "--bundle $BUNDLE: no such directory"; return 1; }
  else
    id="$FROM_RUN"
    if [ -z "$id" ]; then
      command -v gh >/dev/null || { echo "no bundle: pass --bundle DIR, or install gh for --from-run"; return 1; }
      local branch; branch="$(git -C "$ROOT" rev-parse --abbrev-ref HEAD)"
      read -r id sha < <(gh run list -R "$REPO" --workflow ci.yml --branch "$branch" --status success -L 1 \
        --json databaseId,headSha -q '.[0] | "\(.databaseId) \(.headSha)"' 2>/dev/null)
      [ -n "$id" ] || { echo "no green CI run on $branch to take a bundle from: pass --bundle DIR or --from-run ID"; return 1; }
      [ "$sha" = "$(git -C "$ROOT" rev-parse HEAD)" ] || \
        echo "warning: run $id built ${sha:0:7}, and HEAD is $(git -C "$ROOT" rev-parse --short HEAD): the app tier tests that build, not your working tree"
    fi
    DEB_DIR="$ROOT/target/test-tiers/bundles/$id"
    if [ -z "$(find "$DEB_DIR" -name '*.deb' -print -quit 2>/dev/null)" ]; then
      echo "downloading bundle-ubuntu-22.04 from run $id"
      mkdir -p "$DEB_DIR"
      gh run download "$id" -R "$REPO" -n bundle-ubuntu-22.04 -D "$DEB_DIR" || return 1
    fi
  fi
  DEB="$(find "$DEB_DIR" -name '*.deb' -print -quit)"
  [ -n "$DEB" ] || { echo "no .deb under $DEB_DIR"; return 1; }
  echo "bundle: $DEB"
  echo "bundle-dir: $DEB_DIR"
}

tier_app() {
  local img="$IMAGE:app" out="$RUN_DIR/app-e2e-out" name="${VOL}app-$$"
  DEB_DIR=""
  step bundle resolve_bundle || return 1
  DEB_DIR="$(sed -n 's/^bundle-dir: //p' "$RUN_DIR/app-bundle.log" | tail -1)"
  # The .deb is amd64. An arm64 host runs the image under emulation.
  step image docker_ build --platform linux/amd64 --target app -t "$img" docker/test-tiers || return 1
  mkdir -p "$out" && chmod a+rwx "$out"
  # --network none: the mock gateway is on the container's own loopback, and
  # nothing inside can reach anything else. The hosts file, the trust store,
  # port 443 and the install are the container's, removed with it.
  trap 'docker_ rm -f "$name" >/dev/null 2>&1' INT TERM
  step signin docker_ run --rm --name "$name" --platform linux/amd64 --network none --shm-size 1g \
    -v "$(hostpath "$ROOT"):/src:ro" -v "$(hostpath "$DEB_DIR"):/bundle:ro" -v "$(hostpath "$out"):/out" \
    "$img"
  local rc=$?
  trap - INT TERM
  echo "screenshots, page dumps and logs: $out"
  if [ $rc -ne 0 ]; then
    # The script names its own step; carry it into ours.
    local inner; inner="$(grep -o 'failed at step: [A-Za-z0-9_-]*' "$FAILED_LOG" | tail -1 | sed 's/failed at step: //')"
    [ -n "$inner" ] && FAILED_STEP="signin > $inner"
    return 1
  fi
  if [ "$HOST" != linux ]; then
    note "covered the Linux installed app only; the $HOST driver ($([ "$HOST" = macos ] && echo macos-signin.sh || echo windows-uia.ps1)) needs a disposable $HOST VM, see docs/testing.md"
  fi
}

tier_routing() {
  step catalog-routing noproxy cargo test --locked -p gate-connect-core --test catalog_routing || return 1
  note "OS-trust mode skipped: ci/catalog-routing/run.sh writes the machine trust store and refuses outside CI"
}

# ---------------------------------------------------------------- main ----
RESULTS=()
overall=0
for TIER in "${TIERS[@]}"; do
  echo
  echo "=== tier: $TIER ($HOST host)"
  FAILED_STEP="" FAILED_LOG=""
  start=$(date +%s)
  if "tier_$TIER"; then
    RESULTS+=("$TIER|PASS|$(( $(date +%s) - start ))|")
  else
    RESULTS+=("$TIER|FAIL|$(( $(date +%s) - start ))|step $FAILED_STEP, log $FAILED_LOG")
    overall=1
    [ "$FAIL_FAST" = 1 ] && break
  fi
done

echo
echo "=== summary ($HOST host, logs in $RUN_DIR)"
for r in "${RESULTS[@]}"; do
  IFS='|' read -r t s secs detail <<<"$r"
  printf '  %-10s %-4s %5ss  %s\n' "$t" "$s" "$secs" "$detail"
done
for t in "${ALL_TIERS[@]}"; do
  case " ${TIERS[*]} " in *" $t "*) ;; *) printf '  %-10s not selected\n' "$t" ;; esac
done
if [ ${#NOTES[@]} -gt 0 ]; then
  echo "not covered here:"
  printf '  - %s\n' "${NOTES[@]}"
fi
exit $overall
