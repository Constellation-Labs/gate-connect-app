import { Component, type ErrorInfo, type ReactNode } from "react";
import { captureException } from "../lib/analytics";
import { proxyStatus, quitApp } from "../lib/api";
import { Button } from "./gc/ui";

interface Props {
  children: ReactNode;
}

/** What the backend says about traffic, independent of whether this window can
 * still render. `unknown` is its own state and never collapses into "off": the
 * crash may well be the bridge itself, and guessing "off" here would tell a
 * user their traffic stopped when it is very likely still flowing. */
type Routing = "checking" | "on" | "off" | "unknown";

interface State {
  error: Error | null;
  info: ErrorInfo | null;
  routing: Routing;
  quitting: boolean;
}

/** Catches render-time throws so a crash surfaces as a readable message
 * instead of a blank white window. The onboarding and popover windows are
 * both first-launch-critical: a silent blank leaves the user with nothing
 * to act on, so we show the error (and forward it to PostHog) here.
 *
 * This screen answers the question the user actually has. It used to open with
 * a heading, one sentence, and the full JS and component stack in a `<pre>`,
 * with no way out of a window that has no menu bar and no reload of its own.
 * The one thing a user of a credential proxy needs to know when its UI dies is
 * whether their paid API traffic is still routing, and the backend can be
 * asked: the engine runs in the Rust process and does not care that the
 * webview threw. So: routing status first, an exit second, and the stack
 * behind a disclosure for whoever is reporting it. */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, info: null, routing: "checking", quitting: false };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    this.setState({ info });
    captureException(error);
    // Asked once, on the crash, not polled: this screen is a dead end by
    // definition and a ticking status line would only add motion to it.
    proxyStatus()
      .then((state) => this.setState({ routing: state.running ? "on" : "off" }))
      .catch(() => this.setState({ routing: "unknown" }));
  }

  render() {
    const { error, info, routing, quitting } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="flex h-full flex-col gap-3 overflow-y-auto bg-gc-surface p-6 text-gc-ink">
        <h1 className="text-[17px] font-semibold tracking-[-0.01em]">Something went wrong</h1>

        {/* The lede, and deliberately not the error. This window failed; the
            gateway is a separate process and usually did not. */}
        <p className="text-[13px] leading-[1.45] text-gc-ink-3">
          This window hit an unexpected error.{" "}
          {routing === "on" ? (
            <span className="font-medium text-gc-ink">
              Your traffic is still routing through Gate.
            </span>
          ) : routing === "off" ? (
            <span className="font-medium text-gc-ink">
              Routing is off, so nothing is going through Gate right now.
            </span>
          ) : routing === "checking" ? (
            "Checking whether your traffic is still routing…"
          ) : (
            <span className="font-medium text-gc-ink">
              Gate Connect couldn&rsquo;t check whether routing is still on.
            </span>
          )}
        </p>

        {/* Reload first: reopening the window is the cheap fix and it keeps
            routing exactly as it is. Quit is the honest second option and
            wears the destructive treatment, because on macOS and Windows it
            stops the engine and takes routing down with it. */}
        <div className="flex items-center gap-2">
          <Button
            variant="secondary"
            size="sm"
            disabled={quitting}
            onClick={() => window.location.reload()}
          >
            Reload window
          </Button>
          <Button
            variant="danger"
            size="sm"
            disabled={quitting}
            onClick={() => {
              this.setState({ quitting: true });
              void quitApp().catch(() => this.setState({ quitting: false }));
            }}
          >
            {quitting ? "Quitting…" : "Quit Gate Connect"}
          </Button>
        </div>

        {/* Collapsed, like every other raw payload in the app. A stack trace
            is evidence for a bug report, not the first thing a user reads
            about their own machine. */}
        <details className="mt-auto">
          <summary className="cursor-pointer py-0.5 text-[11.5px] text-gc-ink-3">
            Error details
          </summary>
          <pre className="mt-1.5 whitespace-pre-wrap break-words rounded bg-gc-sunken p-3 font-mono text-[10.5px] leading-relaxed text-gc-ink-2">
            {error.name}: {error.message}
            {"\n\n"}
            {error.stack ?? "(no stack)"}
            {info?.componentStack ? `\n\nComponent stack:${info.componentStack}` : ""}
          </pre>
        </details>
      </div>
    );
  }
}
