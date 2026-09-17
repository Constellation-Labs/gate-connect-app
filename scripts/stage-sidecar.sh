#!/usr/bin/env bash
# Stage the environment forwarder where the Tauri bundler expects a sidecar.
#
# `externalBin` wants the target triple in the filename, and the bundler's build
# script checks the file exists - so this has to run before `tauri dev` and
# `tauri build` alike, not only in CI. It is wired into `beforeDevCommand` and
# `beforeBuildCommand` for exactly that reason.
#
# The forwarder is a separate binary on purpose: a running process holds a file
# lock on its own image, and if that image were the app's, the Windows updater
# could not replace it. See `crates/forwarder/src/main.rs`.
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p src-tauri/binaries

# Release for a real build, debug for `tauri dev` - building the release profile
# on every dev launch would add minutes for a binary nobody is profiling.
#
# A flag rather than an environment variable: `tauri build` runs
# `beforeBuildCommand` through `cmd /C` on Windows, which cannot parse a POSIX
# `VAR=value cmd` prefix.
profile="debug"
flag=""
if [ "${1:-}" = "--release" ]; then
  profile="release"
  flag="--release"
fi

# Honour CARGO_TARGET_DIR: a caller that redirected the build output would
# otherwise have us copy from a directory cargo never wrote to.
target_dir="${CARGO_TARGET_DIR:-target}"

stage() {
  local triple="$1" ext=""
  case "$triple" in *windows*) ext=".exe" ;; esac
  # shellcheck disable=SC2086
  cargo build $flag -p gate-connect-forwarder --target "$triple"
  cp "$target_dir/$triple/$profile/gate-connect-forwarder$ext" \
     "src-tauri/binaries/gate-connect-forwarder-$triple$ext"
}

host="$(rustc -vV | sed -n 's/^host: //p')"

if [ "$(uname -s)" = "Darwin" ]; then
  # The release build is universal, so both arches have to exist and be lipo'd.
  # A dev build only needs the host's, and adding the other target would make a
  # first run download a toolchain for no benefit.
  if [ "$profile" = "release" ]; then
    rustup target add x86_64-apple-darwin aarch64-apple-darwin >/dev/null
    stage x86_64-apple-darwin
    stage aarch64-apple-darwin
    lipo -create \
      src-tauri/binaries/gate-connect-forwarder-x86_64-apple-darwin \
      src-tauri/binaries/gate-connect-forwarder-aarch64-apple-darwin \
      -output src-tauri/binaries/gate-connect-forwarder-universal-apple-darwin
  else
    stage "$host"
  fi
else
  stage "$host"
fi
