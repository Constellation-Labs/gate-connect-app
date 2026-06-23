import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { initAnalytics, captureException } from "./lib/analytics";
import "./index.css";

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
