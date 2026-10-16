# The app e2e flows every OS runs the same way (AG-905). Sourced, never run:
# macos-flows.sh and windows-linux-flows.sh set the runner up (mock gateway,
# hosts file, trust, install) and then call these in order.
#
# Every step is text-based: click by name, type by label, wait for text. The
# driver behind `drv` is the one difference, so a step reads the same on all
# three OSes. Each step is a `step` name, which is what a failure reports.
#
# The sourcing script defines, before calling a flow:
#
#   drv <verb> [args...]   the driver: macos-ax.js, windows-uia.ps1 or
#                          webdriver.mjs, with the same verbs
#   snap <name>            screenshot and page dump into $OUT
#   app_launch             start the installed app; return once a window shows
#   app_running            exit 0 while the app's main process is alive
#   answer_os_trust        answer the dialog the OS raises when the app trusts
#                          its CA, on the OS that raises one; else nothing
#   native <path>          the path as a native program reads it
#   step <name>, endstep   name the step a failure reports
#
# and sets OS (macos, windows or linux), ROOT, HERE, WORK, OUT, DATA, KEY,
# CAPTURE (the mock gateway's request log) and CODEX_DIR.
#
# The CLI is Codex: one pinned npm package on all three runners, detected by
# its config directory, and routed through the loopback relay over plain HTTP,
# so it needs no CA of its own. Its request is `codex exec`, as in the
# real-tools e2e (ci/e2e/run.sh), asserted with the same assert-capture.mjs.

APP_SECTION="ChatGPT / Codex"
APP_SWITCH="Route $APP_SECTION"

# First launch: the intro window, then the sign-in pane, then an API key.
flow_signin() {
  # A first launch opens the intro window: a welcome frame and a few steps,
  # each with Next, the last with "Get started", which closes it. Click Next
  # until "Get started" shows, so a step added to or dropped from the intro
  # does not break this; ten clicks is far more than the intro has.
  step "intro"
  drv wait-text 60 "Welcome to Gate Connect"
  for clicks in $(seq 0 10); do
    if drv wait-text 2 "Get started" >/dev/null 2>&1; then break; fi
    if [ "$clicks" -eq 10 ]; then
      echo "no \"Get started\" after $clicks clicks on Next" >&2
      return 1
    fi
    drv click 30 "Next"
  done
  drv click 30 "Get started"
  endstep

  step "welcome"
  drv click 60 "Use an API key"
  endstep

  step "api-key-pane"
  drv wait-text 30 "Paste a Gate API key"
  drv type 30 "API key" "$KEY"
  drv click 30 "Connect and continue"
  endstep
}

# Codex as a person has it before Gate Connect is involved: a config directory
# and an API-key login. The app detects Codex by that directory, and its
# connect reads auth.json to pick the upstream URL shape (and refuses without
# it). Written before the first launch, so the first scan finds it.
seed_cli() {
  mkdir -p "$CODEX_DIR"
  printf '{"auth_mode":"apikey","OPENAI_API_KEY":"sk-e2e-codex-own-key"}' >"$CODEX_DIR/auth.json"
  codex --version
}

# After "Name this device": skip naming, then the setup's own "Turn on
# routing", which is the UI's one way to enable routing (there is no master
# switch: routing then follows the app). Then the diagnostics question, and
# the app shell.
flow_enable_routing() {
  step "skip-naming"
  drv click 30 "Skip naming"
  endstep

  # The enable creates the app's CA and trusts it, which is the OS prompt
  # answer_os_trust deals with. "Turn on routing" shows only while routing is
  # off; a "Done" here instead means routing was already on, which a first
  # launch must not be.
  step "turn-on-routing"
  drv wait-text 60 "You're connected"
  drv click 60 "Turn on routing"
  answer_os_trust
  # The pane moves on only when the enable succeeded; a failed one stays here
  # with its error, which the failure dump opens.
  drv wait-text 120 "Share diagnostic data"
  node "$HERE/assert-routing.mjs" intent "$(native "$DATA/proxy/routing-intent.json")" on 30
  endstep

  # Skipped rather than shared: nothing from this run should reach analytics.
  step "diagnostics"
  drv click 30 "Skip data sharing"
  drv wait-text 60 "$APP_SECTION"
  snap 03-routing-on
  endstep
}

# Connect Codex from its own pane: open it from the rail and flip "Route
# ChatGPT / Codex". The evidence is outside the UI: Codex's config now names
# the relay, and carries no Gate credential.
flow_connect_cli() {
  step "connect-cli"
  # The rail row's name leads with the app's name and goes on with its status.
  drv click-prefix 60 "$APP_SECTION"
  drv wait-text 30 "$APP_SWITCH"
  drv click 30 "$APP_SWITCH"
  # Routing is on and the CA trusted by now, so the app's own "Trust the Gate
  # certificate?" step should not come up. If it does, answer it as a person
  # would rather than fail on a property of the machine.
  if drv wait-text 3 "Trust the Gate certificate?" >/dev/null 2>&1; then
    echo "the app asked to trust its certificate again"
    drv click 30 "Trust certificate"
    answer_os_trust
  fi
  drv wait-checked 60 "$APP_SWITCH"
  node "$HERE/assert-routing.mjs" config "$(native "$CODEX_DIR/config.toml")" routed 60
  snap 04-cli-connected
  endstep
}

# One `codex exec`, which must reach the mock gateway through the relay with
# the Gate headers: the key the app holds, the upstream URL, and Codex named as
# the client. $1 names the run in the step and the output file.
flow_cli_request() {
  local tag="$1"
  step "cli-request-$tag"
  : >"$CAPTURE"
  local out="$OUT/codex-$tag.out"
  (cd "$WORK" && exec codex exec --skip-git-repo-check "ping" </dev/null >"$out" 2>&1) &
  local pid=$!
  # The mock answers 200 with a body Codex cannot use, so Codex retries rather
  # than exits. Wait for the request, then stop it.
  for _ in $(seq 1 120); do
    grep -q '/responses' "$CAPTURE" 2>/dev/null && break
    sleep 0.5
  done
  stop_cli "$pid"
  echo "codex output (last 20 lines):"
  tail -n 20 "$out" | sed 's/^/    /'
  node "$(native "$ROOT/ci/e2e/assert-capture.mjs")" "$(native "$CAPTURE")" /v1/responses api-key "" codex
  echo "PASS: codex reached the gateway through Gate ($tag)"
  endstep
}

stop_cli() {
  kill "$1" 2>/dev/null || true
  if [ "$OS" = windows ]; then
    # The npm shim's codex.exe outlives an msys kill (as in run.sh).
    MSYS2_ARG_CONV_EXCL='*' taskkill /F /T /IM codex.exe >/dev/null 2>&1 || true
  else
    pkill -f "codex exec --skip-git-repo-check" 2>/dev/null || true
  fi
}

# AG-905 AC4: quit from the app's own menu, open it again, and routing is
# still on and Codex still routes. Nothing is clicked after the relaunch except
# to open Codex's pane: the startup enable and restore are what bring it back.
flow_restart() {
  step "quit"
  drv click 30 "More"
  # The window can be gone before the driver hears back from the click.
  drv click 30 "Quit Gate Connect" || echo "the click on Quit returned an error; checking the process"
  local gone=""
  for _ in $(seq 1 120); do
    if ! app_running; then gone=1; break; fi
    sleep 0.5
  done
  [ -n "$gone" ] || { echo "the app was still running 60s after Quit" >&2; return 1; }
  echo "the app exited"
  # macOS and Windows put every tool back on its own settings on quit and
  # reconnect them on the next launch. Linux does not: its engine is a helper
  # daemon that outlives the window, so routing carries on through it.
  if [ "$OS" = linux ]; then
    node "$HERE/assert-routing.mjs" config "$(native "$CODEX_DIR/config.toml")" routed 0
  else
    node "$HERE/assert-routing.mjs" config "$(native "$CODEX_DIR/config.toml")" direct 30
  fi
  endstep

  step "relaunch"
  app_launch
  # The shell, not a setup pane: the account, the device and the diagnostics
  # answer all survived.
  drv wait-text 120 "$APP_SECTION"
  snap 05-relaunched
  endstep

  step "routing-restored"
  node "$HERE/assert-routing.mjs" intent "$(native "$DATA/proxy/routing-intent.json")" on 60
  node "$HERE/assert-routing.mjs" config "$(native "$CODEX_DIR/config.toml")" routed 90
  drv click-prefix 60 "$APP_SECTION"
  drv wait-checked 60 "$APP_SWITCH"
  snap 06-routing-restored
  endstep

  flow_cli_request after-restart
}

# The key is in no file the app wrote, and in none of Codex's: the relay
# injects it, so a config that carries it is the product's central claim
# broken.
assert_key_on_no_disk() {
  local dir
  for dir in "$DATA" "$CODEX_DIR"; do
    if grep -rqF -- "$KEY" "$dir"; then
      echo "::error::the API key is in a file under $dir:" >&2
      grep -rlF -- "$KEY" "$dir" >&2
      return 1
    fi
    echo "the key is in no file under $dir"
  done
}
