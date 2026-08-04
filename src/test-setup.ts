// jsdom implements no layout, so it ships no `Element.prototype.scrollIntoView`.
// Components that scroll themselves into view (the destructive confirm panel in
// Settings) would throw on mount for a reason that has nothing to do with the
// behaviour under test. Stub it once here rather than guarding every call site
// in product code.
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = function scrollIntoView() {};
}
