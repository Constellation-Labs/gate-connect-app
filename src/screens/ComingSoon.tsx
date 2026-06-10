import { SubHeader, Button } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";

/** Direct Gateway placeholder. The per-app per-model picker (writing gate/*
 *  model entries into each agent's config) isn't built yet — this stands in
 *  for it so the home card has somewhere to go. */
export function ComingSoon({ onBack }: { onBack: () => void }) {
  return (
    <div className="flex flex-col">
      <SubHeader title="Direct Gateway" onBack={onBack} />
      <div className="flex flex-col items-center px-6 pb-6 pt-7 text-center">
        <div className="flex h-12 w-12 items-center justify-center rounded-[14px] bg-gc-sunken text-gc-ink-3">
          <Icon name="layers" size={24} />
        </div>
        <div className="mt-3.5 text-[16px] font-semibold tracking-[-0.02em] text-gc-ink">
          Coming soon
        </div>
        <p className="mt-1.5 max-w-[290px] text-[12.5px] leading-[1.45] text-gc-ink-3">
          Pick Gate models like{" "}
          <span className="font-mono text-[11.5px] text-gc-accent-ink">gate/opus</span>{" "}
          per app and Gate Connect will write the config for Claude Code, Codex, and
          OpenCode. It’s on the way.
        </p>
        <Button full className="mt-4" onClick={onBack}>
          Back
        </Button>
      </div>
    </div>
  );
}
