import { useRef } from "react";
import { Takeover, TAKEOVER_Z } from "./Takeover";
import { Button } from "./gc/ui";
import { Icon } from "./gc/Icon";
import type { HermesProviderChoice } from "../lib/useRouting";

/**
 * The popover's half of the Hermes provider gate.
 *
 * The window shell asks this through `HermesProviderDialog`; this is the same
 * question in the popover's own furniture, and it exists because the popover
 * connects Hermes through `App.tsx`'s own `setToolRouted` rather than through
 * `useRouting`. Without it the shell reachable from `gcNewUi(false)` routed
 * Hermes at an uninspected provider silently - which is the defect the window's
 * dialog was written to close, left standing in the other shell.
 *
 * **Not a shared component with the window's dialog, deliberately.** That one
 * is a 480-600px centred `Modal` for a 1280x800 window; this is a 360px
 * takeover over a popover, in `gc/*` ink that dies with the popover. What the
 * two share is the decision and the words for it, which is why the sentences
 * here are the dialog's, shortened to the width - and why a change to either
 * belongs in both until the popover goes.
 *
 * **Cancel means cancel**, as it does in the window. Declining leaves Hermes
 * switched off rather than routed at a provider nothing inspects: that state is
 * the bug, and a button offering it would ship the bug as a preference. Focus
 * lands on Cancel for the reason every gate in the app puts it on the safe
 * button - Enter on an unread panel must not decide what this one decides.
 */
export function HermesProviderNotice({
  domains,
  defaulted,
  onCancel,
  onConfirm,
}: {
  domains: HermesProviderChoice[];
  /** Whether the provider is Hermes' documented default rather than one the
   *  person wrote into `config.yaml`. Decides the opening sentence: an
   *  unconfigured Hermes really does call OpenRouter, but "your config uses"
   *  would be a claim about a file nobody read. */
  defaulted: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const safeRef = useRef<HTMLButtonElement>(null);
  const joined = (xs: string[]) =>
    xs.length === 1 ? xs[0] : `${xs.slice(0, -1).join(", ")} and ${xs[xs.length - 1]}`;
  const names = domains.map((d) => d.name);
  const list = joined(names);
  const plural = names.length !== 1;
  // "Both" is a claim about the count, so it is only made for two.
  const confirmLabel =
    names.length === 1 ? "Turn on" : names.length === 2 ? "Turn both on" : "Turn all on";
  // The rows whose switch also reaches a tool of their own. Empty for a
  // proxy-only provider such as OpenRouter.
  const reaching = domains.filter((d) => d.tools.length > 0);

  return (
    <Takeover
      z={TAKEOVER_Z.trust}
      labelledBy="hermes-provider-title"
      initialFocus={safeRef}
      onEscape={onCancel}
    >
      {/* The same family the certificate pre-flight opens on: something Gate
          needs turned on before it can do what the switch just promised. */}
      <Icon name="triangleAlert" size={56} />

      <div className="flex flex-col gap-1.5">
        <h1
          id="hermes-provider-title"
          className="text-gc-panel-title font-semibold tracking-[-0.01em] text-gc-ink"
        >
          Turn on {list} too?
        </h1>
        <p className="text-gc-body-sm leading-snug text-gc-ink-3">
          {defaulted
            ? `Hermes has no provider in its config, so it uses ${list}, its default.`
            : `Your Hermes config uses ${list} as its model provider.`}{" "}
          Gate only inspects providers you have turned on, so Hermes traffic
          would pass through unseen until {plural ? "these are" : "this is"} on
          too.
        </p>
        {reaching.length > 0 ? (
          <p className="text-gc-caption leading-snug text-gc-ink-3">
            {reaching.map((d) => (
              <span key={d.slug}>
                Turning on {d.name} also covers {joined(d.tools)}. If{" "}
                {d.tools.length === 1 ? "it is" : "they are"} installed, Gate
                connects {d.tools.length === 1 ? "it" : "them"} the next time
                Gate Connect starts.{" "}
              </span>
            ))}
          </p>
        ) : null}
        {/* The breadth, which is what makes this a question rather than
            something Gate should just do on the user's behalf. */}
        <p className="text-gc-caption leading-snug text-gc-ink-2">
          {plural ? "They apply" : "It applies"} to every app on this machine
          that uses {plural ? "them" : list}, not just Hermes.
        </p>
      </div>

      <div className="mt-1 flex w-full flex-col gap-2">
        <Button variant="accent" full onClick={onConfirm}>
          {confirmLabel}
        </Button>
        <Button ref={safeRef} variant="secondary" full onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </Takeover>
  );
}
