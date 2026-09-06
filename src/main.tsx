import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { App } from "./App";
import { NewUiApp } from "./NewUiApp";
import { TrayApp } from "./TrayApp";
import { newUiEnabled } from "./lib/newUi";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Onboarding } from "./screens/Onboarding";
import { initAnalytics, captureException } from "./lib/analytics";
import { describe, logError } from "./lib/log";
import { applyTextScale, readStoredScale } from "./lib/useTextScale";
import "./index.css";

// Before first paint, so a user who scaled to 200% does not watch the popover
// render at 100% and reflow. The hook re-asserts this on mount and owns every
// change after it; this is only the head start.
applyTextScale(readStoredScale());

// Start analytics before render (no-op without a build-time key, and no-op when
// the user has opted out of diagnostic data) and forward uncaught frontend
// crashes to PostHog error tracking.
//
// Not awaited: consent has to be read over IPC, and blocking first paint on it
// would trade a visible delay for a few milliseconds of telemetry. The handlers
// below no-op until the client exists, which is the safe direction.
void initAnalytics();

// Both sinks, because they answer different questions. PostHog aggregates across
// installs but needs a build-time key and the user's consent, so on a developer
// or staging machine it is usually silent - which is how an unhandled rejection
// that killed every switch in the window left no trace anyone could read. The
// local log is the one you can open, and it is off in production.
//
// `unhandledrejection` is the load-bearing one here: every routing write is
// called as `void routing.…`, so a throw from one arrives nowhere else.
window.addEventListener("error", (e) => {
  captureException(e.error ?? e.message);
  logError(`uncaught: ${e.message} (${e.filename}:${e.lineno}:${e.colno})`);
});
window.addEventListener("unhandledrejection", (e) => {
  captureException(e.reason);
  logError(`unhandled rejection: ${describe(e.reason)}`);
});

// The same bundle backs all three windows: "main" renders the app, "tray"
// renders the compact tray popover, and the full-size intro window
// ("onboarding") renders only the intro. getCurrentWindow() throws outside
// Tauri (plain-browser dev), so `?window=tray` picks the surface there - the
// same escape the e2e suite uses to reach it.
const windowKind = (() => {
  try {
    return getCurrentWindow().label;
  } catch {
    return new URLSearchParams(window.location.search).get("window") ?? "main";
  }
})();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {/* The onboarding and tray windows keep their own content whatever the
        shell flag says - each is a separate window with a separate job. */}
    {windowKind === "onboarding" ? (
      <ErrorBoundary>
        <Onboarding />
      </ErrorBoundary>
    ) : windowKind === "tray" ? (
      <ErrorBoundary>
        <TrayApp />
      </ErrorBoundary>
    ) : newUiEnabled() ? (
      <ErrorBoundary>
        <NewUiApp />
      </ErrorBoundary>
    ) : (
      <ErrorBoundary>
        <App />
      </ErrorBoundary>
    )}
  </React.StrictMode>,
);
