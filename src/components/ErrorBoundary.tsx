import { Component, type ErrorInfo, type ReactNode } from "react";
import { captureException } from "../lib/analytics";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
  info: ErrorInfo | null;
}

/** Catches render-time throws so a crash surfaces as a readable message
 * instead of a blank white window. The onboarding and popover windows are
 * both first-launch-critical: a silent blank leaves the user with nothing
 * to act on, so we show the error (and forward it to PostHog) here. */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, info: null };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    this.setState({ info });
    captureException(error);
  }

  render() {
    const { error, info } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="flex h-full flex-col gap-3 overflow-y-auto bg-gc-surface p-6 text-gc-ink">
        <h1 className="text-[17px] font-bold tracking-[-0.01em]">Something went wrong</h1>
        <p className="text-[13px] text-gc-ink-4">
          The window hit an unexpected error while loading. Details below.
        </p>
        <pre className="whitespace-pre-wrap break-words rounded-lg bg-gc-sunken p-3 font-mono text-[11.5px] leading-relaxed text-gc-ink-2">
          {error.name}: {error.message}
          {"\n\n"}
          {error.stack ?? "(no stack)"}
          {info?.componentStack ? `\n\nComponent stack:${info.componentStack}` : ""}
        </pre>
      </div>
    );
  }
}
