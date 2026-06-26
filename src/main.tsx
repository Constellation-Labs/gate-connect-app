import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { App } from "./App";
import { initAnalytics, captureException } from "./lib/analytics";
import "./index.css";

// The native window starts hidden (`visible: false` in tauri.conf.json) and is
// positioned, but not shown, by Rust's setup. We reveal it here - only after
// the inline splash in index.html has actually painted - so macOS never flashes
// a blank WKWebView before its first frame. Two requestAnimationFrames guarantee
// a composited frame has hit the screen before the window appears, so the brand
// mark is on screen the instant the popover does.
requestAnimationFrame(() =>
  requestAnimationFrame(() => {
    const win = getCurrentWindow();
    void win.show();
    void win.setFocus();
  }),
);

// Start analytics before render (no-op without a build-time key) and forward
// uncaught frontend crashes to PostHog error tracking.
initAnalytics();
window.addEventListener("error", (e) => captureException(e.error ?? e.message));
window.addEventListener("unhandledrejection", (e) => captureException(e.reason));

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
