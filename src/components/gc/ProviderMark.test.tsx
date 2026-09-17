import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
import { MARK_NAMES, ProviderMark, providerMarkFor, providerNameFor } from "./ProviderMark";

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
    //
    // Asserted on the resolved NAME rather than on two renders' `innerHTML`:
    // that comparison only held for marks with no ids, so an alias resolving to
    // one of the six marks that carry defs would have failed on the `useId`
    // suffix rather than on the thing under test.
    const pairs: [string, string][] = [
      ["x-ai", "xai"],
      ["z-ai", "zai"],
      ["moonshot", "moonshotai"],
      ["bytedance", "bytedance-seed"],
      ["mistralai", "mistral"],
      ["meta-llama", "meta"],
      ["amazon", "aws"],
      ["ibm-granite", "ibm"],
      ["arcee-ai", "arcee"],
    ];
    for (const [a, b] of pairs) {
      expect(providerNameFor(a), `${a} vs ${b}`).toBe(providerNameFor(b));
      expect(providerNameFor(a), a).toBeDefined();
    }
  });

  it("does not resolve a vendor off Object.prototype", () => {
    // `vendor` is the gateway's `owned_by` or the id's prefix, so a catalogue row
    // names it. On a bare index `constructor` comes back truthy, `MARKS[name]` is
    // undefined, and `.viewBox` throws inside render - which the root
    // ErrorBoundary turns into a blank main window.
    for (const hostile of ["constructor", "__proto__", "toString", "hasOwnProperty"]) {
      expect(providerNameFor(hostile), hostile).toBeUndefined();
      expect(() => providerMarkFor(hostile), hostile).not.toThrow();
      expect(providerMarkFor(hostile), hostile).toBeUndefined();
    }
  });

  it("gives qwen the Alibaba mark, because 665:18457 draws it", () => {
    // lobe-icons publishes a separate `qwen-color`; the frame draws
    // `alibaba-color 1` beside `gate/qwen3-6-35b-a3b` and the file wins. qwen is
    // the catalogue's largest namespace, so this is its most visible mark.
    expect(providerNameFor("qwen")).toBe("alibaba");
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

/**
 * Structural invariants over every mark, which is the only thing a unit test can
 * say about vendored art.
 *
 * These exist because a mark can render *nothing* while every other test passes:
 * `poolside` did. Its `<mask>` lost `width`, `height` and `mask-type: alpha`
 * when the art was converted, and without the last of those a gradient-filled
 * mask is read as luminance, so the whole mark resolved to transparent. Nothing
 * threw, the paths were all present, and the assertions above were happy.
 */
describe("every mark's geometry", () => {
  it.each(MARK_NAMES)("%s draws at least one path", (name) => {
    const { container } = render(<ProviderMark name={name} />);
    const paths = [...container.querySelectorAll("path")];
    expect(paths.length).toBeGreaterThan(0);
    expect(paths.some((p) => (p.getAttribute("d") ?? "").length > 0)).toBe(true);
  });

  it.each(MARK_NAMES)("%s resolves every url(#id) it references", (name) => {
    const { container } = render(<ProviderMark name={name} />);
    const declared = new Set([...container.querySelectorAll("[id]")].map((n) => n.id));
    const referenced = [...container.querySelectorAll("*")].flatMap((n) =>
      [...n.attributes]
        .map((a) => /^url\(#(.+)\)$/.exec(a.value)?.[1])
        .filter((v): v is string => v !== undefined),
    );
    for (const ref of referenced) expect(declared, `${name} -> #${ref}`).toContain(ref);
  });

  it.each(MARK_NAMES)("%s gives any mask a region and an alpha type", (name) => {
    // The poolside failure exactly: a mask with no width/height covers nothing
    // useful, and one without `mask-type: alpha` is read as luminance.
    const { container } = render(<ProviderMark name={name} />);
    for (const mask of container.querySelectorAll("mask")) {
      expect(mask.getAttribute("width"), `${name} mask width`).toBeTruthy();
      expect(mask.getAttribute("height"), `${name} mask height`).toBeTruthy();
      expect(mask.style.maskType, `${name} mask-type`).toBe("alpha");
    }
  });
});
