import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { OpenCodeEnvDialog } from "./dialogs";

const noop = () => {};

afterEach(cleanup);

/**
 * The question asked before one app's switch turns on a machine-wide channel.
 *
 * It had no test at all, which is how the copy drifted from the mechanism
 * twice: once claiming the proxy variables are how OpenCode routes (AG-893
 * caught it), and once saying nothing about which processes they reach
 * (AG-895, below). These pin what the dialog owes a reader rather than its
 * exact sentences, so the wording can still be edited.
 */
describe("OpenCodeEnvDialog", () => {
  it("names the second thing the click turns on, in the title", () => {
    // The whole reason the dialog exists: one switch, two effects. A person
    // who reads only the title still learns the second one.
    render(<OpenCodeEnvDialog onCancel={noop} onConfirm={noop} />);
    expect(
      screen.getByRole("heading", { name: /also turns on Command-line tools/i }),
    ).toBeTruthy();
  });

  it("says the variables reach only what starts afterwards", () => {
    // AG-895. The ticket's literal claim - toggling OpenCode offers to close
    // Codex - does not survive the code: every `offerAfterChange` caller is
    // scoped to the slugs that wrote. The suspicion under it is sound, and
    // this sentence is the answer to it. A process's environment is fixed at
    // spawn, so an already-open terminal keeps going direct.
    //
    // Nothing else in the app says so. `env-proxy` is not a process, so
    // `ReopenEvidence::process_names_known` is false and `reopen_pending`
    // declines to claim anything, which is correct and leaves this silent.
    render(<OpenCodeEnvDialog onCancel={noop} onConfirm={noop} />);
    expect(screen.getByText(/already open keep the environment/i)).toBeTruthy();
    expect(screen.getByText(/Reopen them/i)).toBeTruthy();
  });

  it("does not claim the variables are how OpenCode routes", () => {
    // The correction AG-893's review made. `integrations/opencode.rs` rewrites
    // `provider.<id>.options.baseURL` for every provider known at connect
    // time; the variables carry what that snapshot cannot - a provider added
    // later, one outside the allowlist, one skipped as local.
    render(<OpenCodeEnvDialog onCancel={noop} onConfirm={noop} />);
    expect(
      screen.queryByText(/no gateway setting of its own/i),
    ).toBeNull();
    expect(screen.getByText(/providers you had set up/i)).toBeTruthy();
  });

  it("says a certificate is involved, which is the larger fact", () => {
    // `NODE_EXTRA_CA_CERTS` adds Gate's interception certificate to the trust
    // roots of every Node process started afterwards. Harder to discover than
    // "git and curl go through Gate" and a bigger claim on the machine.
    render(<OpenCodeEnvDialog onCancel={noop} onConfirm={noop} />);
    expect(screen.getByText(/tells Node to trust Gate/i)).toBeTruthy();
  });

  it("is informational, so the primary turns both on and cancel is the way out", () => {
    // Nothing is replaced or removed, so this is not a destructive dialog and
    // focus stays on the primary - unlike the dialogs where "no" is the safe
    // answer.
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(<OpenCodeEnvDialog onCancel={onCancel} onConfirm={onConfirm} />);

    screen.getByRole("button", { name: "Turn both on" }).click();
    expect(onConfirm).toHaveBeenCalledOnce();

    screen.getByRole("button", { name: "Cancel" }).click();
    expect(onCancel).toHaveBeenCalledOnce();
  });
});
