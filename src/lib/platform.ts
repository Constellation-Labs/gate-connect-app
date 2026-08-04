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
      return `${determiner} system's secure store`;
  }
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
