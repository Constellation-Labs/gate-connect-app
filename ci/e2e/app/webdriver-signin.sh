#!/usr/bin/env bash
# AG-905 spike: install the built Windows or Linux bundle, launch it under
# tauri-driver, and sign in with an API key through the app's own window,
# driven by WebDriver (webdriver.mjs). The macOS half is macos-signin.sh; this
# is the same flow and the same three assertions.
#
# CI runners only. It edits the hosts file, trusts a root machine-wide, binds
# port 443 and installs the app, none of which belongs on a developer machine.
#
# The app is the release-profile bundle from bundle.yml, so every test seam is
# compiled out and nothing in the environment can point it at a mock. As on
# macOS, the runner makes the built-in default gateway host the mock instead:
#
#   1. a throwaway CA and a leaf for the default gateway host, the CA trusted
#      in the OS store (rustls-platform-verifier reads it): LocalMachine Root on
#      Windows, /usr/local/share/ca-certificates on Linux;
#   2. the hosts file sends that host to 127.0.0.1 and ::1, so nothing the app
#      sends can reach the real gateway, whatever it decides to call (the
#      gateway publishes AAAA records too; the mock listens on 127.0.0.1 only,
#      so ::1 is refused at once and the app falls back to v4);
#   3. ci/e2e/mock-gateway.mjs on 443 with that leaf, logging every request.
#
# Driving: tauri-driver launches the installed binary through the platform's
# own WebDriver, msedgedriver for WebView2 on Windows and WebKitWebDriver for
# WebKitGTK on Linux. Neither needs anything compiled into the app: on Windows
# msedgedriver hands WebView2 its remote-debugging arguments through the
# environment, and on Linux tauri-driver sets TAURI_WEBVIEW_AUTOMATION, which
# tauri-runtime-wry reads at runtime in any build profile.
#
# Linux runs under a display and a session bus (the workflow wraps this in
# xvfb-run and dbus-run-session), and starts gnome-keyring here: a release
# build stores the key in the Secret Service, since the file seam is compiled
# out.
#
# Env: BUNDLE_DIR (the installer somewhere under it), OUT (screenshots, page
# dumps, logs).
set -euo pipefail

# CI=true alone is not enough: self-hosted runners and some dev boxes set it.
if [ "${GITHUB_ACTIONS:-}" != "true" ] || [ "${RUNNER_ENVIRONMENT:-}" != "github-hosted" ]; then
  echo "webdriver-signin.sh changes the hosts file, the machine trust store, port 443 and the installed app: GitHub-hosted runners only" >&2
  exit 2
fi

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
OUT="${OUT:-$PWD/app-e2e-out}"
BUNDLE_DIR="${BUNDLE_DIR:?BUNDLE_DIR must name the directory holding the installer}"
mkdir -p "$OUT"

GATEWAY_HOST="gateway.constellationgate.ai"
KEY="sk-gw-e2e-00000000000000000000000000000000"

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) OS=windows ;;
  Linux) OS=linux ;;
  *) echo "webdriver-signin.sh drives Windows and Linux; macOS is macos-signin.sh" >&2; exit 2 ;;
esac

# Git Bash rewrites an argument that looks like a path, which turns a Windows
# switch such as /flushdns into one. NOCONV runs a command without that.
NOCONV=(env MSYS2_ARG_CONV_EXCL='*')
native() { if [ "$OS" = windows ]; then cygpath -w "$1"; else printf '%s' "$1"; fi; }

if [ "$OS" = windows ]; then
  DATA="$(cygpath -u "$LOCALAPPDATA")/Gate Connect"
  HOSTS=/c/Windows/System32/drivers/etc/hosts
  SUDO=()
else
  DATA="$HOME/.local/share/Gate Connect"
  HOSTS=/etc/hosts
  SUDO=(sudo)
fi

WORK="$(mktemp -d)"
export WD_SESSION="$WORK/session.json"
WD=(node "$HERE/webdriver.mjs")

os_screenshot() {
  local file="$1"
  if [ "$OS" = windows ]; then
    powershell -NoProfile -Command "
      Add-Type -AssemblyName System.Windows.Forms, System.Drawing
      \$b = [System.Windows.Forms.SystemInformation]::VirtualScreen
      \$bmp = New-Object System.Drawing.Bitmap \$b.Width, \$b.Height
      [System.Drawing.Graphics]::FromImage(\$bmp).CopyFromScreen(\$b.Left, \$b.Top, 0, 0, \$bmp.Size)
      \$bmp.Save('$(native "$file")')" || echo "screen capture failed"
  else
    import -window root "$file" 2>/dev/null || echo "screen capture failed"
  fi
}

STEP="setup"
on_exit() {
  local rc=$?
  if [ "$rc" -ne 0 ]; then
    echo "::error::app e2e ($OS) failed at step: $STEP"
    os_screenshot "$OUT/failed-$STEP.screen.png"
    if [ -f "$WD_SESSION" ]; then
      # An error card keeps its cause under a Details disclosure; open it so
      # the dump carries the cause.
      "${WD[@]}" click 2 "Details" >/dev/null 2>&1 || true
      "${WD[@]}" screenshot "$OUT/failed-$STEP.png" || true
      "${WD[@]}" dump >"$OUT/failed-$STEP.page.txt" 2>&1 || true
    fi
  fi
  [ -f "$WD_SESSION" ] && "${WD[@]}" quit >/dev/null 2>&1 || true
  cp "$WORK/capture.jsonl" "$WORK/mock.out" "$WORK/driver.out" "$OUT/" 2>/dev/null || true
  cp -R "$DATA" "$OUT/app-data" 2>/dev/null || true
  [ -n "${DRIVER_PID:-}" ] && kill "$DRIVER_PID" 2>/dev/null || true
  [ -n "${MOCK_PID:-}" ] && "${SUDO[@]}" kill "$MOCK_PID" 2>/dev/null || true
  exit "$rc"
}
trap on_exit EXIT

step() {
  STEP="$1"
  echo "::group::$1"
}
endstep() { echo "::endgroup::"; }

if [ "$OS" = linux ]; then
  step "keyring"
  # Unlocks (and so creates) the login keyring with an empty password, and
  # serves org.freedesktop.secrets on this session bus.
  eval "$(printf 'e2e' | gnome-keyring-daemon --unlock --components=secrets)"
  export GNOME_KEYRING_CONTROL
  echo "DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS DISPLAY=${DISPLAY:-}"
  # Prove the store takes a write and gives it back before the app needs it,
  # so a broken keyring names itself here and not as a failed sign-in.
  printf 'probe' | secret-tool store --label="app e2e probe" gc-app-e2e probe
  [ "$(secret-tool lookup gc-app-e2e probe)" = "probe" ] || { echo "the Secret Service did not return what was stored" >&2; exit 1; }
  secret-tool clear gc-app-e2e probe
  echo "Secret Service round trip ok"
  endstep
fi

step "mock-gateway"
(
  cd "$WORK"
  # -subj "/CN=..." would otherwise reach openssl as a Windows path.
  export MSYS2_ARG_CONV_EXCL='*'
  openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.pem \
    -subj "/CN=Gate Connect App E2E CA" -days 2 \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -keyout leaf.key -out leaf.csr \
    -subj "/CN=$GATEWAY_HOST" 2>/dev/null
  printf 'subjectAltName=DNS:%s\nextendedKeyUsage=serverAuth\n' "$GATEWAY_HOST" >leaf.ext
  openssl x509 -req -in leaf.csr -CA ca.pem -CAkey ca.key -CAcreateserial \
    -out leaf.pem -days 2 -extfile leaf.ext 2>/dev/null
)
if [ "$OS" = windows ]; then
  "${NOCONV[@]}" certutil -addstore Root "$(native "$WORK/ca.pem")" | tail -2
  printf '\r\n127.0.0.1 %s\r\n::1 %s\r\n' "$GATEWAY_HOST" "$GATEWAY_HOST" >>"$HOSTS"
  "${NOCONV[@]}" ipconfig /flushdns >/dev/null
else
  sudo cp "$WORK/ca.pem" /usr/local/share/ca-certificates/gate-connect-app-e2e.crt
  sudo update-ca-certificates >/dev/null
  printf '127.0.0.1 %s\n::1 %s\n' "$GATEWAY_HOST" "$GATEWAY_HOST" | sudo tee -a "$HOSTS" >/dev/null
fi
: >"$WORK/capture.jsonl"
"${SUDO[@]}" env CAPTURE_LOG="$(native "$WORK/capture.jsonl")" MOCK_PORT=443 \
  MOCK_CERT="$(native "$WORK/leaf.pem")" MOCK_KEY="$(native "$WORK/leaf.key")" \
  "$(command -v node)" "$(native "$ROOT/ci/e2e/mock-gateway.mjs")" >"$WORK/mock.out" 2>&1 &
MOCK_PID=$!
# On Windows the curl on PATH is the system one (Schannel): it needs a native
# path, and it fails a leaf with no CRL distribution point as "revocation
# status unknown". This probe only checks the name and the CA; the OS store
# check below is what speaks for the app.
CURL_TLS=(--cacert "$(native "$WORK/ca.pem")")
[ "$OS" = windows ] && CURL_TLS+=(--ssl-no-revoke)
for _ in $(seq 1 50); do
  if curl -fsS "${CURL_TLS[@]}" "https://$GATEWAY_HOST/health" >/dev/null 2>&1; then break; fi
  sleep 0.2
done
# The name resolves to the mock and the leaf verifies against the throwaway CA.
curl -fsS "${CURL_TLS[@]}" "https://$GATEWAY_HOST/health"
echo
# And the OS trust store accepts it, which is what the app's TLS stack asks.
if [ "$OS" = windows ]; then
  powershell -NoProfile -Command "(Invoke-WebRequest -UseBasicParsing 'https://$GATEWAY_HOST/health').StatusCode"
else
  curl -fsS "https://$GATEWAY_HOST/health"
  echo
fi
: >"$WORK/capture.jsonl"
endstep

step "install"
if [ "$OS" = windows ]; then
  SETUP="$(find "$BUNDLE_DIR" -name '*-setup.exe' -print -quit)"
  [ -n "$SETUP" ] || { echo "no *-setup.exe under $BUNDLE_DIR" >&2; exit 1; }
  echo "installing from $SETUP"
  powershell -NoProfile -Command "\$p = Start-Process -FilePath '$(native "$SETUP")' -ArgumentList '/S' -Wait -PassThru; exit \$p.ExitCode"
  APP=""
  for dir in "$DATA" "/c/Program Files/Gate Connect"; do
    [ -d "$dir" ] || continue
    APP="$(find "$dir" -maxdepth 1 -iname '*.exe' ! -iname 'uninstall*' ! -iname '*forwarder*' -print -quit)"
    [ -n "$APP" ] && break
  done
else
  DEB="$(find "$BUNDLE_DIR" -name '*.deb' -print -quit)"
  [ -n "$DEB" ] || { echo "no .deb under $BUNDLE_DIR" >&2; exit 1; }
  echo "installing from $DEB"
  sudo apt-get install -y "$(readlink -f "$DEB")" >/dev/null
  PKG="$(dpkg-deb -f "$DEB" Package)"
  APP="$(dpkg -L "$PKG" | grep '^/usr/bin/' | grep -v forwarder | head -1)"
fi
[ -n "$APP" ] && [ -f "$APP" ] || { echo "installed app binary not found" >&2; exit 1; }
echo "installed: $APP"
endstep

step "driver"
# Each prerequisite is checked here, so a missing one names itself rather than
# reading later as a window that never appeared.
command -v tauri-driver >/dev/null || { echo "tauri-driver is not on PATH" >&2; exit 1; }
if [ "$OS" = windows ]; then
  [ -n "${MSEDGEDRIVER:-}" ] && [ -f "$MSEDGEDRIVER" ] || { echo "MSEDGEDRIVER must name msedgedriver.exe" >&2; exit 1; }
  NATIVE_DRIVER="$(native "$MSEDGEDRIVER")"
else
  NATIVE_DRIVER="$(command -v WebKitWebDriver)" || { echo "WebKitWebDriver is not on PATH (webkit2gtk-driver)" >&2; exit 1; }
  [ -n "${DISPLAY:-}" ] || { echo "no DISPLAY: run under xvfb-run" >&2; exit 1; }
  [ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ] || { echo "no session bus: run under dbus-run-session" >&2; exit 1; }
fi
"$NATIVE_DRIVER" --version || true
echo "native driver: $NATIVE_DRIVER"
tauri-driver --port 4444 --native-driver "$NATIVE_DRIVER" >"$WORK/driver.out" 2>&1 &
DRIVER_PID=$!
for _ in $(seq 1 50); do
  if curl -fsS http://127.0.0.1:4444/status >/dev/null 2>&1; then break; fi
  sleep 0.2
done
curl -fsS http://127.0.0.1:4444/status || { cat "$WORK/driver.out"; exit 1; }
echo
endstep

step "launch"
"${WD[@]}" start "$(native "$APP")" 120
"${WD[@]}" windows || true
"${WD[@]}" dump >"$OUT/01-launched.page.txt" 2>&1 || true
"${WD[@]}" screenshot "$OUT/01-launched.png" || true
endstep

# A first launch opens the intro window: a welcome frame and a few steps, each
# with Next, the last with "Get started", which closes it. Click Next until
# "Get started" shows, so a step added to or dropped from the intro does not
# break this; ten clicks is far more than the intro has.
step "intro"
"${WD[@]}" wait-text 60 "Welcome to Gate Connect"
for clicks in $(seq 0 10); do
  if "${WD[@]}" wait-text 2 "Get started" >/dev/null 2>&1; then break; fi
  if [ "$clicks" -eq 10 ]; then
    echo "no \"Get started\" after $clicks clicks on Next" >&2
    exit 1
  fi
  "${WD[@]}" click 30 "Next"
done
"${WD[@]}" click 30 "Get started"
endstep

step "welcome"
"${WD[@]}" click 60 "Use an API key"
endstep

step "api-key-pane"
"${WD[@]}" wait-text 30 "Paste a Gate API key"
"${WD[@]}" type 30 "API key" "$KEY"
"${WD[@]}" click 30 "Connect and continue"
endstep

step "signed-in"
"${WD[@]}" wait-text 60 "Name this device"
"${WD[@]}" screenshot "$OUT/02-signed-in.png" || true
"${WD[@]}" dump >"$OUT/02-signed-in.page.txt" 2>&1 || true
node "$HERE/assert-signin.mjs" "$(native "$DATA/account.json")" "$(native "$WORK/capture.jsonl")" "$KEY" "$GATEWAY_HOST"
# The key is in no file the app wrote, not only account.json.
if grep -rqF "$KEY" "$DATA"; then
  echo "the key is on disk under $DATA:" >&2
  grep -rlF "$KEY" "$DATA" >&2
  exit 1
fi
echo "the key is in no file under $DATA"
# And the OS store holds an entry for it, checked without reading the secret.
SERVICE="ai.constellation.gate-connect.account.gateway-api-key"
if [ "$OS" = windows ]; then
  # Credential Manager lists targets only; keyring names them <user>.<service>.
  "${NOCONV[@]}" cmdkey /list | grep -F "$SERVICE" || { echo "no Credential Manager entry for $SERVICE" >&2; exit 1; }
else
  # SearchItems returns item paths, not secrets.
  FOUND="$(gdbus call --session --dest org.freedesktop.secrets \
    --object-path /org/freedesktop/secrets \
    --method org.freedesktop.Secret.Service.SearchItems "{'service': '$SERVICE'}")"
  echo "$FOUND"
  echo "$FOUND" | grep -q "/org/freedesktop/secrets/collection/" || { echo "no Secret Service item for $SERVICE" >&2; exit 1; }
fi
endstep

echo "PASS: signed in with an API key through the installed app's own window ($OS)"
