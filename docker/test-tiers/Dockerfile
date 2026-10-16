# syntax=docker/dockerfile:1
#
# Disposable Linux environments for scripts/test-tiers.sh (AG-909). Two targets:
#
#   daemon  the Rust toolchain and the Linux build deps, for the helper daemon
#           tests (crates/cli/tests/helper_daemon.rs). They exist only on Linux,
#           so a macOS or Windows host runs them here.
#   app     the installed-app tier: a desktop session (Xvfb, a D-Bus session
#           bus, gnome-keyring), WebKitWebDriver and tauri-driver, Node, and
#           every runtime dependency of the .deb, so the bundle installs with
#           no network. ci/e2e/app/windows-linux-signin.sh runs inside it.
#
# Ubuntu 22.04 for both, because that is the CI runner: the .deb is built on
# it, and the daemon tests should see the same glibc and libdbus CI does.
#
# The repo is never copied in. scripts/test-tiers.sh bind-mounts the checkout
# read-only at /src, so the build context is this directory alone.

ARG UBUNTU=ubuntu:22.04

# ---------------------------------------------------------------- daemon ----
FROM ${UBUNTU} AS daemon
ENV DEBIAN_FRONTEND=noninteractive
# The `rust` job's Linux deps, minus the webkit stack: the daemon tests build
# gate-connect-cli and gate-connect-core only, never the desktop crate.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl build-essential pkg-config libdbus-1-dev \
 && rm -rf /var/lib/apt/lists/*
# Toolchain and components baked in: the tests run with --network none, and
# rust-toolchain.toml would otherwise make rustup fetch clippy and rustfmt.
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path \
      --profile minimal --default-toolchain 1.88.0 --component rustfmt,clippy \
 && chmod -R a+rX /opt/rustup /opt/cargo
# Not root, as on a runner: some daemon checks are about file permissions,
# which root would pass whatever they are. The registry and target dirs exist
# here, owned by the user, so the named volumes mounted over them start out
# with that ownership.
RUN useradd -m -u 1000 tester \
 && mkdir -p /opt/cargo/registry /opt/cargo/git /target \
 && chown tester:tester /opt/cargo/registry /opt/cargo/git /target
ENV CARGO_TARGET_DIR=/target CARGO_PROFILE_DEV_DEBUG=line-tables-only
USER tester
WORKDIR /src

# ------------------------------------------------------------ tauri-driver ----
# Built from source like the `app-e2e-linux` job does. tauri-driver 2.1 needs
# Rust 1.90, newer than the repo's pinned 1.88, so it gets its own toolchain
# in a throwaway stage, on the same Ubuntu so the binary links against the
# same glibc.
FROM ${UBUNTU} AS tauri-driver
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl build-essential pkg-config \
 && rm -rf /var/lib/apt/lists/*
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.90.0
RUN /root/.cargo/bin/cargo install tauri-driver@2.1.0 --locked --root /opt/tauri-driver

# ------------------------------------------------------------------- app ----
FROM ${UBUNTU} AS app
ENV DEBIAN_FRONTEND=noninteractive
# The `app-e2e-linux` job's packages (webkit2gtk-driver xvfb dbus-x11
# gnome-keyring libsecret-tools imagemagick), what that runner image already
# has (sudo, curl, openssl, xauth, gdbus), and the .deb's own Depends
# (libnss3-tools libdbus-1-3 libappindicator3-1 libwebkit2gtk-4.1-0
# libgtk-3-0). The package lists are kept: the script installs the .deb with
# apt-get, offline, and apt resolves the local file against them.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl openssl sudo xz-utils xauth \
      webkit2gtk-driver xvfb dbus dbus-x11 gnome-keyring libsecret-tools \
      libglib2.0-bin imagemagick \
      libnss3-tools libdbus-1-3 libappindicator3-1 libwebkit2gtk-4.1-0 libgtk-3-0 \
      desktop-file-utils hicolor-icon-theme
# Node 20, as `actions/setup-node` gives the CI job. Ubuntu 22.04's own is 12.
ARG NODE_VERSION=20.18.1
ARG TARGETARCH
RUN set -eu; \
    case "${TARGETARCH:-amd64}" in amd64) arch=x64 ;; arm64) arch=arm64 ;; *) echo "unsupported arch ${TARGETARCH}" >&2; exit 1 ;; esac; \
    base="https://nodejs.org/dist/v${NODE_VERSION}"; file="node-v${NODE_VERSION}-linux-${arch}.tar.xz"; \
    cd /tmp; curl -fsSLO "$base/$file"; curl -fsSL "$base/SHASUMS256.txt" | grep " $file\$" | sha256sum -c -; \
    tar -xJf "$file" -C /usr/local --strip-components=1 --exclude CHANGELOG.md --exclude LICENSE --exclude README.md; \
    rm "$file"; node --version
COPY --from=tauri-driver /opt/tauri-driver/bin/tauri-driver /usr/local/bin/tauri-driver
# The runner's shape: an ordinary user with passwordless sudo, since the
# script edits /etc/hosts, the CA anchors and port 443 through sudo. All of it
# lands in this container and goes with it.
RUN useradd -m -u 1000 tester \
 && echo 'tester ALL=(ALL) NOPASSWD:ALL' >/etc/sudoers.d/tester \
 && chmod 0440 /etc/sudoers.d/tester
# The marker windows-linux-signin.sh accepts in place of a GitHub-hosted
# runner. On its own it proves nothing: the script also requires /.dockerenv,
# which the container runtime creates and no image can ship, and a PID 1 that
# is not the host's init.
ENV GATE_CONNECT_E2E_CONTAINER=1
# The Secret Service, made race-free. The signin script starts
# `gnome-keyring-daemon --unlock` and probes it with secret-tool right away.
# When the probe wins the race for the bus name, D-Bus activates a second
# daemon. That daemon finds a half-written login.keyring, asks the prompter
# for a password, and waits forever on a display nobody watches (seen on
# ubuntu-latest). So any daemon D-Bus activates starts unlocked with the same
# password, and the prompter is gone, so a prompt fails at once instead of
# hanging.
RUN printf '%s\n' '[D-BUS Service]' 'Name=org.freedesktop.secrets' \
      "Exec=/bin/sh -c 'printf e2e | exec /usr/bin/gnome-keyring-daemon --foreground --unlock --components=secrets'" \
      >/usr/share/dbus-1/services/org.freedesktop.secrets.service \
 && rm -f /usr/share/dbus-1/services/org.gnome.keyring.SystemPrompter.service \
          /usr/share/dbus-1/services/org.gnome.keyring.PrivatePrompter.service
COPY app-entry.sh /usr/local/bin/app-entry.sh
USER tester
# And a complete login keyring (password "e2e", the script's) already on disk,
# so no daemon ever reads one mid-write.
RUN dbus-run-session -- sh -c '\
      eval "$(printf e2e | gnome-keyring-daemon --unlock --components=secrets)"; \
      for i in $(seq 1 50); do \
        gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
          --method org.freedesktop.DBus.NameHasOwner org.freedesktop.secrets | grep -q true && break; \
        sleep 0.1; done; \
      printf seed | secret-tool store --label=seed gc-app-e2e seed && secret-tool clear gc-app-e2e seed' \
 && test -s /home/tester/.local/share/keyrings/login.keyring
WORKDIR /home/tester
ENTRYPOINT ["/usr/local/bin/app-entry.sh"]
