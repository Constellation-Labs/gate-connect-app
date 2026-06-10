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

/** Human name of the OS secret store where credentials live. */
export function secretStoreName(p: Platform): string {
  switch (p) {
    case "windows":
      return "Windows Credential Manager";
    case "linux":
      return "the system secret service";
    case "macos":
      return "the macOS keychain";
    default:
      return "your system's secure store";
  }
}
