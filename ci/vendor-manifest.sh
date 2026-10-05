#!/usr/bin/env bash
# Re-vendor the harness manifest from a local checkout of the gateway repo.
#
#   ci/vendor-manifest.sh ~/Workspace/gate
#
# Copies `harnesses.json` byte for byte into crates/core/vendor/ and rewrites the
# recorded checksum beside it. No network: the two repositories ship separately,
# so the copy is taken from a checkout you chose, at the commit you chose, and
# `crates/core/src/manifest.rs` fails the build if the copy and the checksum
# disagree. Run `cargo test -p gate-connect-core manifest` afterwards: a drift
# test that fails is the point of re-vendoring.
set -euo pipefail

gate="${1:?usage: ci/vendor-manifest.sh <path to a gate checkout>}"
src="$gate/harnesses.json"
[ -f "$src" ] || { echo "no harnesses.json under $gate" >&2; exit 1; }

here="$(cd "$(dirname "$0")/.." && pwd)"
dest="$here/crates/core/vendor"
mkdir -p "$dest"
cp "$src" "$dest/harnesses.json"

if command -v sha256sum >/dev/null 2>&1; then
  sum="$(sha256sum "$dest/harnesses.json" | cut -d' ' -f1)"
else
  sum="$(shasum -a 256 "$dest/harnesses.json" | cut -d' ' -f1)"
fi
printf '%s\n' "$sum" > "$dest/harnesses.json.sha256"

rev="$(git -C "$gate" rev-parse --short HEAD 2>/dev/null || echo unknown)"
echo "vendored harnesses.json from $gate at $rev ($sum)"
