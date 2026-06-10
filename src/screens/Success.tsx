import { PopHeader } from "../components/gc/PopHeader";
import { ConstellationHexMark } from "../components/gc/ConstellationHexMark";
import { Button } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";

/** Connected confirmation shown right after first-run. */
export function Success({
  workspace,
  proxyOn,
  onDone,
  onOpenSettings,
}: {
  workspace: string;
  proxyOn: boolean;
  onDone: () => void;
  onOpenSettings: () => void;
}) {
  return (
    <div className="flex flex-col">
      <PopHeader
        workspace={workspace}
        pill={proxyOn ? "connected" : "idle"}
        onGear={onOpenSettings}
      />
      <div className="flex flex-col items-center px-5 pb-5 pt-4 text-center">
        <div className="relative mb-3">
          <ConstellationHexMark size={46} fill="#002a5f" />
          <span className="absolute -bottom-1 -right-1 flex h-[18px] w-[18px] items-center justify-center rounded-full bg-gc-success text-white ring-2 ring-white">
            <Icon name="check" size={11} stroke={3} />
          </span>
        </div>
        <div className="text-[17px] font-semibold tracking-[-0.02em] text-gc-ink">
          You’re connected
        </div>
        <p className="mt-1.5 max-w-[280px] text-[12.5px] leading-[1.45] text-gc-ink-3">
          Gate Connect is linked to your workspace. Turn on the proxy to route your
          desktop agents through Gate.
        </p>
        <Button variant="accent" full className="mt-4" onClick={onDone}>
          Done
        </Button>
      </div>
    </div>
  );
}
