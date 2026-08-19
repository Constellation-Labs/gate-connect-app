#!/usr/bin/env bash
# Verify that a release carries every artifact the release workflow is meant to
# publish, and that each one actually downloads.
#
# Two failure modes this exists to catch, both of which have already shipped:
#
#   - A matrix leg uploads nothing, or uploads stale bytes, and still exits 0.
#     The alias step only checks that `gh release upload` returned success, and
#     the AppImage fix step is currently the sole place that reads anything back.
#
#   - An asset lands and is then lost before the draft is published. That is how
#     v0.2.0 shipped without Gate.Connect_universal.dmg for a week: the alias
#     uploaded at 03:01:46, the macOS job ended at 03:01:51, and by the 03:09:05
#     publish the asset was gone. Nothing in CI noticed, because nothing looked.
#
# MODE=draft      Read the not-yet-published release through the API. A draft's
#                 assets 404 on their public URL, and its download path is
#                 /download/untagged-<hash>/ rather than /download/<tag>/, so the
#                 public URLs cannot be exercised yet.
# MODE=published  Fetch the public browser_download_url unauthenticated, the way
#                 a user does, and additionally check the stable
#                 /releases/latest/download/ aliases, which exist only once the
#                 release is published.
#
# Every check runs before the script exits, so one run reports every problem
# rather than stopping at the first.

set -uo pipefail

: "${REPO:?REPO is required (owner/name)}"
: "${TAG:?TAG is required (e.g. v0.2.0)}"
: "${MODE:?MODE is required (draft or published)}"

if [ "$MODE" != draft ] && [ "$MODE" != published ]; then
  echo "MODE must be 'draft' or 'published', got '$MODE'" >&2
  exit 1
fi

VERSION="${TAG#v}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

FAILED=0
fail() { printf '  FAIL  %s\n' "$*" >&2; FAILED=$((FAILED + 1)); }
ok()   { printf '  ok    %s\n' "$*"; }
skip() { printf '  skip  %s\n' "$*"; }

# --- resolve the release -----------------------------------------------------
# A draft is not reachable through /releases/tags/<tag> (that 404s), so find it
# by scanning the release list for the tag instead.
mapfile -t MATCHES < <(gh api --paginate "repos/$REPO/releases" \
  --jq ".[] | select(.tag_name == \"$TAG\") | .id")

if [ "${#MATCHES[@]}" -eq 0 ]; then
  echo "No release found for tag $TAG." >&2
  echo "If this ran against a draft, the job token may lack draft visibility;" >&2
  echo "raise the job's permissions to 'contents: write' and re-run." >&2
  exit 1
fi
if [ "${#MATCHES[@]}" -gt 1 ]; then
  echo "Tag $TAG matches ${#MATCHES[@]} releases (ids: ${MATCHES[*]})." >&2
  echo "Duplicate drafts split uploads across releases; delete the stale one." >&2
  exit 1
fi
RELEASE_ID="${MATCHES[0]}"

if ! gh api --paginate "repos/$REPO/releases/$RELEASE_ID/assets" \
  --jq '.[] | [.name, (.id|tostring), (.size|tostring), .state, .browser_download_url] | @tsv' \
  > "$WORK/assets.tsv"; then
  echo "Could not list assets for release $RELEASE_ID" >&2
  exit 1
fi

has_asset() { awk -F'\t' -v n="$1" '$1 == n { f = 1 } END { exit !f }' "$WORK/assets.tsv"; }

# --- what the workflow is supposed to have produced --------------------------
VERSIONED=(
  "Gate.Connect_${VERSION}_amd64.AppImage"
  "Gate.Connect_${VERSION}_amd64.AppImage.sig"
  "Gate.Connect_${VERSION}_amd64.deb"
  "Gate.Connect_${VERSION}_amd64.deb.sig"
  "Gate.Connect_${VERSION}_universal.dmg"
  "Gate.Connect_${VERSION}_x64-setup.exe"
  "Gate.Connect_${VERSION}_x64-setup.exe.sig"
)
# The macOS updater bundle is the one artifact tauri names without the version.
# There is deliberately no .dmg.sig: the dmg is not an updater target.
UPDATER=(
  "Gate.Connect_universal.app.tar.gz"
  "Gate.Connect_universal.app.tar.gz.sig"
)
# alias -> the versioned installer it must be a byte-identical copy of, since
# the alias step creates it with cp. A mismatch means the alias is serving a
# different build than the versioned asset next to it.
ALIASES=(
  "Gate.Connect_amd64.AppImage:Gate.Connect_${VERSION}_amd64.AppImage"
  "Gate.Connect_amd64.deb:Gate.Connect_${VERSION}_amd64.deb"
  "Gate.Connect_x64-setup.exe:Gate.Connect_${VERSION}_x64-setup.exe"
  "Gate.Connect_universal.dmg:Gate.Connect_${VERSION}_universal.dmg"
)

EXPECTED=( "${VERSIONED[@]}" "${UPDATER[@]}" )
for pair in "${ALIASES[@]}"; do EXPECTED+=( "${pair%%:*}" ); done
EXPECTED+=( "latest.json" )

echo "Release id $RELEASE_ID, tag $TAG, mode $MODE, ${#EXPECTED[@]} expected assets"
echo
echo "Presence and download:"

for name in "${EXPECTED[@]}"; do
  line=$(awk -F'\t' -v n="$name" '$1 == n' "$WORK/assets.tsv")
  if [ -z "$line" ]; then
    fail "$name is missing from the release"
    continue
  fi
  IFS=$'\t' read -r _ id size state url <<< "$line"
  if [ "$state" != uploaded ]; then
    fail "$name is in state '$state', not 'uploaded'"
    continue
  fi
  if [ "$size" -le 0 ]; then
    fail "$name is $size bytes"
    continue
  fi

  out="$WORK/$name"
  if [ "$MODE" = draft ]; then
    # -L without --location-trusted, so curl drops the auth header on the
    # cross-host hop to storage - which is what that host requires.
    code=$(curl -sSL --retry 3 --retry-all-errors -w '%{http_code}' \
      -H "Authorization: Bearer ${GH_TOKEN:?GH_TOKEN is required}" \
      -H "Accept: application/octet-stream" \
      -o "$out" "https://api.github.com/repos/$REPO/releases/assets/$id") || code=000
  else
    code=$(curl -sSL --retry 3 --retry-all-errors -w '%{http_code}' \
      -o "$out" "$url") || code=000
  fi
  if [ "$code" != 200 ]; then
    fail "$name did not download (HTTP $code)"
    continue
  fi
  got=$(stat -c%s "$out")
  if [ "$got" != "$size" ]; then
    fail "$name downloaded $got bytes, the release reports $size"
    continue
  fi
  ok "$name ($size bytes)"
done

# --- aliases must be the same bytes as their versioned source ---------------
echo
echo "Aliases match their versioned source:"
for pair in "${ALIASES[@]}"; do
  alias_name="${pair%%:*}"
  src_name="${pair#*:}"
  if [ ! -s "$WORK/$alias_name" ] || [ ! -s "$WORK/$src_name" ]; then
    fail "$alias_name vs $src_name: one of the two did not download"
    continue
  fi
  a=$(sha256sum "$WORK/$alias_name" | cut -d' ' -f1)
  b=$(sha256sum "$WORK/$src_name" | cut -d' ' -f1)
  if [ "$a" != "$b" ]; then
    fail "$alias_name does not match $src_name ($a vs $b)"
  else
    ok "$alias_name == $src_name"
  fi
done

# --- latest.json is what the updater reads, so it gets its own pass ---------
echo
echo "latest.json:"
if [ ! -s "$WORK/latest.json" ]; then
  fail "latest.json did not download; skipping its checks"
else
  lj_version=$(jq -r '.version // ""' "$WORK/latest.json")
  if [ "$lj_version" != "$VERSION" ]; then
    fail "latest.json version is '$lj_version', the tag says '$VERSION'"
  else
    ok "version $lj_version matches the tag"
  fi

  while IFS=$'\t' read -r platform lurl lsig; do
    ref="${lurl##*/}"
    if ! has_asset "$ref"; then
      fail "$platform points at $ref, which is not an asset on this release"
      continue
    fi
    if [ -z "$lsig" ]; then
      fail "$platform has an empty signature"
      continue
    fi
    if [ "$MODE" = published ]; then
      # Ranged GET: 206 when honoured, 200 when served whole. Either proves the
      # URL resolves without pulling the entire installer again.
      code=$(curl -sSL --retry 3 -o /dev/null -w '%{http_code}' -r 0-0 "$lurl") || code=000
      if [ "$code" != 206 ] && [ "$code" != 200 ]; then
        fail "$platform URL does not resolve (HTTP $code): $lurl"
        continue
      fi
    fi
    ok "$platform -> $ref"
  done < <(jq -r '.platforms | to_entries[]
    | [.key, .value.url, (.value.signature // "")] | @tsv' "$WORK/latest.json")
fi

# --- the stable download links, which only exist after publish -------------
if [ "$MODE" = published ]; then
  echo
  echo "Stable /releases/latest/download/ aliases:"
  latest_tag=$(gh api "repos/$REPO/releases/latest" --jq '.tag_name' 2>/dev/null || echo "")
  if [ "$latest_tag" != "$TAG" ]; then
    skip "/releases/latest/ resolves to '${latest_tag:-none}', not $TAG"
  else
    for pair in "${ALIASES[@]}"; do
      alias_name="${pair%%:*}"
      code=$(curl -sSL --retry 3 -o /dev/null -w '%{http_code}' -r 0-0 \
        "https://github.com/$REPO/releases/latest/download/$alias_name") || code=000
      if [ "$code" != 206 ] && [ "$code" != 200 ]; then
        fail "/releases/latest/download/$alias_name returns HTTP $code"
      else
        ok "/releases/latest/download/$alias_name"
      fi
    done
  fi
fi

# --- anything on the release we did not expect ------------------------------
cut -f1 "$WORK/assets.tsv" | sort > "$WORK/actual.txt"
printf '%s\n' "${EXPECTED[@]}" | sort > "$WORK/expected.txt"
extras=$(comm -23 "$WORK/actual.txt" "$WORK/expected.txt")
if [ -n "$extras" ]; then
  echo
  echo "Extra assets not in the expected set (reported, not failed):"
  echo "$extras" | sed 's/^/  /'
fi

echo
if [ "$FAILED" -gt 0 ]; then
  echo "$FAILED check(s) failed." >&2
  exit 1
fi
echo "All ${#EXPECTED[@]} assets present, downloadable and consistent."
