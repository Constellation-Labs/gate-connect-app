import { ConstellationHexMark } from "./ConstellationHexMark";
import { ConnPill, IconButton } from "./ui";

/** Popover header - hex mark + "Gate Connect" wordmark, workspace sub-label,
 *  connection pill, and the settings gear. Port of the prototype's PopHeader.
 *
 *  Carries `border-b` for the same reason `SubHeader` and the credential footer
 *  do: the popover has three fixed zones and the hairlines are what mark them.
 *  This header had none, so a scrolled body slid under an identical white
 *  background and the row at the top of the viewport was cut mid-glyph by
 *  nothing at all. The Seam Rule reserves solid hairlines for exactly this
 *  case, dividers between fixed zones, and forbids them only on cards. */
export function PopHeader({
  workspace,
  pill = "connected",
  pillLabel,
  onGear,
}: {
  /** Who the user is on this gateway: the org where there is one, and the
   * gateway host on the surfaces that have nowhere else to print it. Empty
   * renders no sub-label at all, which is what Home passes for a key account:
   * Home prints the host on its own line, and a header repeating it 230px above
   * said the same thing twice. */
  workspace: string;
  pill?: "connected" | "partial" | "idle" | "signedout";
  /** Overrides the pill text. Used when routing is on but there is nothing
   * installed to route, where "Routing on" over "nothing installed to route"
   * is technically true and reads as a contradiction. */
  pillLabel?: string;
  onGear?: () => void;
}) {
  return (
    // Two rows, not two columns. The sub-label used to sit inside the left
    // column, so its width was whatever the pill and the gear left over:
    // 195-206px depending on which pill string is up. Raising the platform's
    // minimum font size to 16px - the one text-scaling control a fixed,
    // non-resizable popover window actually exposes to a user - pushed
    // "Constellation Labs" past that and truncated an 18-character org name.
    // Spanning the full width gives it ~332px, which fixes that case and lets
    // every longer org show more of itself at any size.
    // `flex flex-wrap`, not a two-column grid, and every floor in `em` rather
    // than `px`. At 200% text the wordmark needs ~200px and the pill and gear
    // need ~170 in a 332px row, and the grid had no way to say that: the h1 was
    // `min-w-0` with a `whitespace-nowrap` child, so the wordmark overflowed its
    // own column and painted straight through the pill. Measured at 2x, "Gate
    // Connect" and "Routing on" occupied the same pixels.
    //
    // With `basis-[8em]` the h1 asks for eight characters' worth of room at
    // whatever the current size is: 128px at 100%, where it and the pill share
    // one line comfortably, and 256px at 200%, where their total exceeds the row
    // and the pill group wraps beneath instead of colliding. One rule, no
    // breakpoints, and the trigger is the type size itself.
    <div className="sticky top-0 z-[5] flex flex-wrap items-center gap-x-2 gap-y-1 border-b border-gc-line bg-gc-surface px-3.5 pb-2 pt-3.5">
      <h1
        tabIndex={-1}
        data-screen-focus
        className="inline-flex min-w-0 flex-1 basis-[8em] items-center gap-2 outline-none"
      >
        <ConstellationHexMark size={17} />
        {/* `truncate` rather than `whitespace-nowrap`: a wordmark that must not
            wrap still must not overlap its neighbour, and this is the backstop
            for any size the basis above does not anticipate. */}
        <span className="truncate text-gc-title font-semibold tracking-[-0.02em] text-gc-navy">
          Gate <span className="text-gc-accent">Connect</span>
        </span>
      </h1>
      <div className="flex shrink-0 items-center gap-1.5 ml-auto">
        {/* The header pill answers exactly one question: is traffic routing
            through Gate right now? (Signed-in state lives in Settings.)
            "Partly routed" is the honest answer while the CA is untrusted:
            config tools route, proxy-routed apps don't yet. */}
        <ConnPill
          state={pill}
          label={
            pillLabel ??
            (pill === "connected"
              ? "Routing on"
              : pill === "partial"
                ? "Needs trust"
                : "Routing off")
          }
        />
        {onGear && (
          <IconButton icon="settings" size={15} onClick={onGear} aria-label="Settings" />
        )}
      </div>
      {workspace && (
        // Row two, spanning both columns. `title` because an org name is a
        // proper noun with no shortening the app is entitled to invent, and the
        // ellipsis truncation paints exists in no attribute, so a name long
        // enough to still get cut here stays recoverable.
        //
        // `pl-[25px]` is derived, not arbitrary: the hex mark is 17px and the
        // h1's gap is 8px, so this is the wordmark's own left edge. It reads as
        // an off-grid value and is the one place in the header that must not be
        // rounded to the 4px scale.
        // `leading-tight`, not `leading-none`: at 10.5px `leading-none` makes
        // the line box 10.5px tall while the text's content box is 12px, and
        // `truncate`'s `overflow: hidden` then clips the bottom pixel off every
        // descender - the g in "Engineering", the p in "Platform". Measured
        // `scrollHeight 12` against `clientHeight 11`. The extra 2px of line box
        // costs nothing here because the row is sized by the wordmark above it.
        // `w-full`, not `col-span-2`: the header is a flex row now, and a
        // full-basis item is how a flex child claims its own line. No `mt-1`
        // either: the row's `gap-y-1` is the same 4px, and keeping both pushed
        // every screen below the header down by exactly that much.
        <span
          title={workspace}
          className="w-full truncate pl-[25px] font-mono text-gc-label leading-tight text-gc-ink-3"
        >
          {workspace}
        </span>
      )}
    </div>
  );
}
