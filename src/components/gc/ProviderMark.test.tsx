import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
import { ProviderMark, providerMarkFor } from "./ProviderMark";

afterEach(cleanup);

/**
 * The vendor table, which is the part of this file that can be wrong without
 * looking wrong. The geometry is vendored art and a test cannot say whether a
 * path is the right shape; what it can pin is that a catalogue namespace
 * resolves, that the aliases agree, and that an unmapped vendor yields nothing
 * so the call site's cube shows.
 */
describe("providerMarkFor", () => {
  it("resolves the namespaces the catalogue actually answers with", () => {
    for (const vendor of ["anthropic", "openai", "google", "deepseek", "qwen", "mistralai"]) {
      expect(providerMarkFor(vendor), vendor).toBeTruthy();
    }
  });

  it("gives one mark to a company the catalogue spells several ways", () => {
    // Staging answers all of these for four companies. Resolving only one
    // spelling would leave the other drawing a cube beside an identical model.
    const pairs: [string, string][] = [
      ["x-ai", "xai"],
      ["z-ai", "zai"],
      ["moonshot", "moonshotai"],
      ["bytedance", "bytedance-seed"],
    ];
    for (const [a, b] of pairs) {
      const one = render(providerMarkFor(a)!).container.innerHTML;
      cleanup();
      const two = render(providerMarkFor(b)!).container.innerHTML;
      cleanup();
      expect(one, `${a} vs ${b}`).toBe(two);
    }
  });

  it("gives qwen the Alibaba mark, because 665:18457 draws it", () => {
    // lobe-icons publishes a separate `qwen-color`; the frame draws
    // `alibaba-color 1` beside `gate/qwen3-6-35b-a3b` and the file wins. qwen is
    // the catalogue's largest namespace, so this is its most visible mark.
    const qwen = render(providerMarkFor("qwen")!).container.innerHTML;
    cleanup();
    const alibaba = render(<ProviderMark name="alibaba" />).container.innerHTML;
    expect(qwen).toBe(alibaba);
  });

  it("is case insensitive, since `owned_by` is the gateway's spelling", () => {
    expect(providerMarkFor("Anthropic")).toBeTruthy();
    expect(providerMarkFor("OpenAI")).toBeTruthy();
  });

  it("returns undefined for a vendor with no mark, rather than a placeholder", () => {
    // Fourteen namespaces publish none. Undefined is what lets each call site
    // keep its own cube fallback.
    for (const vendor of ["sao10k", "thedrummer", "gryphe", "nousresearch", ""]) {
      expect(providerMarkFor(vendor), vendor).toBeUndefined();
    }
  });

  it("takes the size the call site asks for", () => {
    // The picker rows draw 16; 130:48325 draws the confirmation's at 20.
    const { container } = render(<ProviderMark name="anthropic" size={20} />);
    const svg = container.querySelector("svg")!;
    expect(svg.getAttribute("width")).toBe("20");
    expect(svg.getAttribute("height")).toBe("20");
  });

  it("scopes gradient ids per instance so two marks cannot capture each other", () => {
    // The hazard `GateAiLogoMark` documents, and the picker is where it would
    // bite: one vendor can occupy seventy rows of the list.
    const { container } = render(
      <>
        <ProviderMark name="stepfun" />
        <ProviderMark name="stepfun" />
      </>,
    );
    const ids = [...container.querySelectorAll("[id]")].map((n) => n.id);
    expect(ids).toHaveLength(2);
    expect(new Set(ids).size).toBe(2);
  });

  it("keeps every mark's own viewBox rather than rescaling coordinates", () => {
    // The Figma exports are 16 and lobe-icons is 24. Rewriting coordinates by
    // hand is where a silent shape error would come from.
    const figma = render(<ProviderMark name="anthropic" />).container.querySelector("svg")!;
    expect(figma.getAttribute("viewBox")).toBe("0 0 16 16");
    cleanup();
    const lobe = render(<ProviderMark name="google" />).container.querySelector("svg")!;
    expect(lobe.getAttribute("viewBox")).toBe("0 0 24 24");
  });

  it("hides the glyph from assistive tech, which reads the name beside it", () => {
    const { container } = render(<ProviderMark name="anthropic" />);
    expect(container.querySelector("svg")!.getAttribute("aria-hidden")).toBe("true");
  });
});
