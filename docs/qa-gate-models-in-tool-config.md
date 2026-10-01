# QA: Gate models written into the app's own config (Codex, Hermes)

Branch: `feat/gate-models-in-tool-config` (Gate Connect).

**What changed.** Choosing Gate models in Gate Connect now writes them into the app's own config. The app shows them in its own model picker, and Gate serves only those models, on the organization's credits. A model outside the chosen set is refused, never swapped. If the user changes the model inside the app, Gate Connect shows App default and says so.

## Setup

1. Quit the installed Gate Connect. Two instances fight over the relay, the forwarder and the system proxy.
2. Back up the tool configs. `pnpm app:local` edits the real ones.
   - `~/.codex/config.toml`
   - `~/.hermes/config.yaml`
3. Check `CODEX_HOME`. If your shell exports it (Orca does), Gate Connect edits `$CODEX_HOME/config.toml`, and a `codex` started from another shell reads `~/.codex`. Run the app and `codex` from shells that agree.
4. Run the branch against staging with a staging key: `pnpm app:local` from the worktree.
5. Check the staging org:
   - it has Gate credits (PAYG balance);
   - the catalogue has models marked compatible with Codex and Hermes.
6. Codex must be installed and logged in (`codex login`). Hermes must be installed.
   - **Restarting the Codex TUI is not enough.** The TUI talks to a long-running app-server daemon, which reads the model catalog only when the daemon starts. After every model change in Gate Connect, quit the TUI and run `env -u CODEX_HOME codex app-server daemon restart`. Otherwise new sessions still get the old catalog.
7. The ChatGPT desktop app runs its own Codex, which reads the same `~/.codex`. Its new sessions pick up the change too.

## Codex

| # | Steps | Expected |
|---|---|---|
| C1 | In the Codex pane, choose Gate model, pick one model, and accept billing | If Codex is running, the restart offer appears. `config.toml` has `model = "<gate id>"`, a `model_catalog_json` line, and a `base_url` ending in `/__gate/t/codex/gate/v1` |
| C2 | Start a new `codex` session and open `/model` | Only the chosen Gate models are listed |
| C3 | Send a prompt | A normal answer. Activity shows it under Codex, billed to Gate credits |
| C4 | Choose 2 models, then switch between them inside Codex (`/model`) | Both work. The pane still says Gate model |
| C5 | `codex exec -m gpt-5.6-sol "hi"`, with a model that is not in the set | Error "…is not one of the Gate models enabled for Codex…". No charge |
| C6 | Change the model inside Codex to one that is not in the set, then focus Gate Connect again | The pane shows App default and a notice ("You switched Codex to …"). `config.toml` keeps your model, and the Gate catalog line is gone |
| C7 | Switch to App default | `config.toml` matches the backup, except the `[model_providers.gate]` block, which stays while routing is on |
| C8 | Quit Gate Connect, then send a Codex prompt | Codex is back on its own model and works directly |

## Hermes

| # | Steps | Expected |
|---|---|---|
| H1 | The Hermes pane now has a model card. Pick 2 models and accept | `config.yaml` has `model.provider: gate-connect`, `model.default` set to the first model, and a `providers.gate-connect` block listing both. Comments and other sections are untouched |
| H2 | Run `hermes model`, or `/model` in a session | Lists exactly the 2 Gate models under "Gate Connect" |
| H3 | Run `hermes -z "hi"`, then `hermes -z "hi" -m <second id>` | Both answer, under Hermes in Activity |
| H4 | `hermes -z "hi" -m some/other-model` | `HTTP 400: …is not one of the Gate models enabled for Hermes…` |
| H5 | Use `hermes model` to switch to another provider (for example OpenRouter), then focus Gate Connect again | The pane shows App default and the notice. `providers.gate-connect` is gone, and your provider choice stays |
| H6 | Switch to App default, or disconnect | `config.yaml` is byte-identical to the backup: `diff` prints nothing |

## Both apps

- **Restart offer.** After any model change while the app is running, the offer lists that app. Choosing "Close" shows the app come back with the change applied.
- **Gate Connect closed.** With an app still on Gate models, closing Gate Connect makes the app show "Gate Connect is not running… Gate models…" instead of hanging.
- **Copy.** No text says "served immediately", or that the app's own model is left unchanged.

## Reporting a failure

Include:
- the step number;
- the app version;
- the relevant config snippet, with keys removed;
- any error text.

---

## Round 2: review fixes (#382, 2026-09-30)

Run the dev build (`env -u CODEX_HOME pnpm app:local`). Use a plain Terminal, or prefix commands with `env -u CODEX_HOME`, so Codex reads `~/.codex`.

| # | Steps | Expected |
|---|---|---|
| R1 | Codex and Hermes on Gate models. Quit Gate Connect (Cmd+Q), then reopen it | While it is closed, the configs are back on the app's own model. After reopening, both cards say Gate model again. `grep -E '^model\|base_url' ~/.codex/config.toml` shows the Gate id and `/gate/v1` |
| R2 | Close every Codex session. Check `pgrep -fl "app-server --listen"`, then change Codex's Gate models in Gate Connect | The daemon PID changes (it was restarted). A new `codex` shows the new set in `/model` straight away |
| R3 | Open a Codex session, then change Codex's Gate models | The restart offer lists Codex. "Close" closes the session and restarts the daemon (new PID). The reopened `codex` shows the new set |
| R4 | No Codex session open (ChatGPT app closed too), then open the Codex pane | No "Reopen CLI to finish" or "Close tool" card |
| R5 | Look at the top banner | It reads "N of M apps" (no "on") |
| R6 | In Hermes, run `hermes model` and pick another provider, then focus Gate Connect | Hermes shows App default and the notice "You switched Hermes to …". Dismiss removes the notice. Choosing Gate models again sticks |
| R7 | In Codex, `/model` to the second model of the set. Focus Gate Connect | The card leads with that model, not the first one chosen |
| R8 | `codex exec -m gpt-5.6-sol "hi"` | Refused: "…not one of the Gate models enabled for Codex…" |
| R9 | *(Optional)* Claude Code on Gate models. Start `claude`, run `/model`, send a prompt. Quit and reopen Gate Connect, then start `claude` again | `/model` lists the set. It still answers after the reopen. Afterwards, App default restores `~/.claude/settings.json` (diff against the backup) |
