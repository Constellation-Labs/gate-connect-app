import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export type Platform = "macos" | "windows" | "linux" | "unknown";

// Cached across mounts so only the first lookup hits the backend.
let cached: Platform | null = null;

export async function fetchPlatform(): Promise<Platform> {
  if (cached) return cached;
  try {
    const os = await invoke<string>("app_platform");
    cached = os === "macos" || os === "windows" || os === "linux" ? os : "unknown";
  } catch {
    cached = "unknown";
  }
  return cached;
}

/** The resolved OS, read synchronously, for the non-React callers that need
 *  to name the secret store in a string (`classifyError`). `unknown` until the
 *  first `fetchPlatform` lands, which happens on the app's first render, so by
 *  the time an error is being classified this is the real platform. */
export function currentPlatform(): Platform {
  return cached ?? "unknown";
}

/** Current OS, resolved once. `unknown` only during the first async tick. */
export function usePlatform(): Platform {
  const [p, setP] = useState<Platform>(cached ?? "unknown");
  useEffect(() => {
    let alive = true;
    fetchPlatform().then((v) => {
      if (alive) setP(v);
    });
    return () => {
      alive = false;
    };
  }, []);
  return p;
}

/** Where credentials live, as a noun phrase ready to drop after "in" / "from".
 *
 *  Returns the determiner too, because Windows does not take one: "keychain"
 *  and "keyring" are common nouns that want "your", but Credential Manager is
 *  the store's actual name and reads like "in Keychain Access" without it.
 *
 *  Not "secret service" on Linux: that is the freedesktop D-Bus API we store
 *  through, whereas "keyring" is what GNOME calls the thing in its own UI, and
 *  it parallels "keychain". Nobody has ever seen "secret service" in a
 *  settings window.
 *
 *  PRODUCT.md's first principle is that the user should always feel where the
 *  key lives, which is worth nothing if we name the wrong vault - so every
 *  string that says this goes through here rather than hardcoding "keychain".
 *
 *  @param determiner "the" where a nearby "your" would already be doing the
 *  work ("your session lives in the keyring"), "your" otherwise. */
export function secretStoreName(p: Platform, determiner: "your" | "the" = "your"): string {
  switch (p) {
    case "windows":
      return "Credential Manager";
    case "linux":
      return `${determiner} keyring`;
    case "macos":
      return `${determiner} keychain`;
    default:
      return `${determiner} system’s secure store`;
  }
}

/** Whether a host-scoped row also covers the same site in a browser, as a
 *  sentence to append - or the empty string where there is nothing to claim.
 *
 *  The chat rows are matched on HOST by `proxy::decide`, never on which app
 *  made the request, so wherever a browser follows the OS proxy the row covers
 *  the browser too. That is the one thing a row named "Claude Desktop chat"
 *  cannot convey, and the thing a user most needs before flipping a switch that
 *  inspects a session cookie. macOS and Windows are that case: the system proxy
 *  is the browser's proxy and the system trust store is its trust store.
 *
 *  Linux says nothing, deliberately, and the empty string is the whole point -
 *  not an oversight to fill in later. The browser is genuinely not covered
 *  there, twice over: Gate wires the proxy through environment variables
 *  (`system_proxy_linux.rs` writes `~/.config/environment.d/gate-proxy.conf`,
 *  and there is no PAC on Linux at all) while browsers take their proxy from
 *  the desktop's own settings, which Gate never writes; and the CA goes into
 *  the system trust store (`ca_linux.rs`), which is not where Chromium keeps
 *  user-added roots. But spelling that out costs three clauses in a 360px
 *  popover to explain a mechanism the user cannot act on, and the sentence
 *  before it has already bounded the scope by naming the host. Saying less is
 *  not the same as claiming more: the copy never promises the browser, so
 *  silence here is accurate. If Linux ever grows NSS trust and desktop proxy
 *  settings, this returns the same sentence macOS does and nothing else moves.
 *
 *  Empty on `unknown` for the same reason it is empty on Linux, plus one: that
 *  value is the first async tick, and a claim about interception is the last
 *  thing to guess at. */
export function browserScopeNote(p: Platform): string {
  return p === "macos" || p === "windows"
    ? "That includes the same site open in your browser."
    : "";
}

/** The platform's own name for the accelerator modifier, for copy that teaches a
 *  shortcut. "Cmd" on macOS and "Ctrl" everywhere else, spelled the way each
 *  platform's own settings windows spell it, because a Windows user told to
 *  press Cmd has been told to press a key their keyboard does not have.
 *
 *  Lives here with the other platform-named strings for the reason given above
 *  `secretStoreName`: naming the wrong thing undoes the instruction. */
export function modKeyLabel(p: Platform): string {
  return p === "macos" ? "Cmd" : "Ctrl";
}

/** Where the local proxy's CA has to be trusted. A bare noun: callers supply
 *  the determiner, since this one is a common noun everywhere.
 *
 *  Distinct from `secretStoreName` - different vault, different question. On
 *  Linux this is the system CA trust store (`ca_linux.rs` writes to
 *  `/usr/local/share/ca-certificates` or `/etc/pki/ca-trust/source/anchors`),
 *  which is emphatically not a keyring. */
export function trustStoreName(p: Platform): string {
  return p === "macos" ? "keychain" : "certificate store";
}

/** What the OS is about to ask, said *before* the user clicks Trust.
 *
 *  Trusting the CA is the one action in the app that hands the user off to a
 *  system dialog, and each platform raises a different one: Windows shows a
 *  security warning naming the certificate, macOS raises the Security Agent
 *  for the login password, Linux escalates to sudo/pkexec. An unexpected
 *  security dialog reads as something going wrong - naming it first turns it
 *  into the step the user was told to expect.
 *
 *  Windows gets the reassurance rather than the detail: its dialog is a red
 *  "Security Warning" written to make you hesitate, so what the user needs
 *  before clicking is that it's expected and which button ends it. The
 *  certificate's own name belongs in `trustPromptWaiting`, where it can be
 *  matched against the name the dialog is quoting back. */
export function trustPromptHint(p: Platform): string {
  switch (p) {
    case "windows":
      return "Windows will show a security warning: that’s expected, choose Yes.";
    case "macos":
      return "macOS will ask for your login password.";
    case "linux":
      return "You’ll be asked for your administrator password.";
    default:
      return "Your system will ask you to confirm.";
  }
}

/** The same handoff, said *while* the OS dialog is up and we're blocked on it.
 *
 *  Present tense and an instruction, because at this point the dialog is on
 *  screen and the only thing left is which button to press. Without it the
 *  popover just sits there with a disabled button while a scary-looking system
 *  warning waits somewhere on screen, possibly behind the window.
 *
 *  Windows quotes the CA's common name back at the user ("claiming to
 *  represent: Gate Connect Local CA"), so naming it here is what lets them
 *  match the dialog in front of them to the app that raised it. Must match
 *  `cert_authority.rs`'s CA_COMMON_NAME exactly. */
export function trustPromptWaiting(p: Platform): string {
  switch (p) {
    case "windows":
      return "Windows is asking you to confirm “Gate Connect Local CA”. Choose Yes to finish.";
    case "macos":
      return "macOS is asking for your login password. Enter it to finish.";
    case "linux":
      return "Enter your administrator password to finish.";
    default:
      return "Your system is asking you to confirm. Approve it to finish.";
  }
}
