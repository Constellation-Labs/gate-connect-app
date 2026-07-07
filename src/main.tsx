import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { App } from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Onboarding } from "./screens/Onboarding";
import { initAnalytics, captureException } from "./lib/analytics";
import "./index.css";

// Start analytics before render (no-op without a build-time key) and forward
// uncaught frontend crashes to PostHog error tracking.
initAnalytics();
window.addEventListener("error", (e) => captureException(e.error ?? e.message));
window.addEventListener("unhandledrejection", (e) => captureException(e.reason));

// The same bundle backs both windows: the tray popover ("main") renders the
// app, the full-size intro window ("onboarding") renders only the intro.
// getCurrentWindow() throws outside Tauri (plain-browser dev), so fall back
// to the popover app there.
const isOnboardingWindow = (() => {
  try {
    return getCurrentWindow().label === "onboarding";
  } catch {
    return false;
  }
})();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {isOnboardingWindow ? (
      <ErrorBoundary>
        <Onboarding />
      </ErrorBoundary>
    ) : (
      <ErrorBoundary>
        <App />
      </ErrorBoundary>
    )}
  </React.StrictMode>,
);
