/**
 * The sizes a request-row mark is drawn at, and nothing else.
 *
 * Two values because the frames draw two: 16 on the app pane's Recent activity
 * rows (272:3282) and 20 on the Overview's Security events (`1402:18014`,
 * `1402:18017`). Typed the way `Modal`'s `width` is, so a size no frame draws
 * cannot be passed. Both `VendorMark` and `ToolMark` take it, and size their
 * slot from `MARK_SLOT` rather than an inline style, since the folder sizes
 * everything else with Tailwind's `size-*`.
 */
export type MarkSize = 16 | 20;

export const MARK_SLOT: Record<MarkSize, string> = { 16: "size-4", 20: "size-5" };
