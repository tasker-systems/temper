/**
 * The words a region says, with its `aria-hidden` decoration stripped out.
 *
 * Spec §3.3: the region states must differ by more than one channel, and the marker glyph is one of
 * them — so a comparison that keeps the glyph is satisfied by the glyph alone, while two states say
 * the same sentence. A differential test written on raw `textContent` therefore passes on a defect
 * it was written to catch. `[found by the probe — 2026-08-21]` wording the give-up identically to
 * the failure left `RegionState.component.test.ts` green, because `⊘` and `⚠` differ.
 *
 * This reads what is left for the accessibility tree. It lives here rather than in either test file
 * because two copies of one predicate are two predicates that can disagree — the same argument the
 * `RegionState` component itself makes about six call sites spelling their own markup.
 */
export const sentenceOf = (el: Element | null | undefined): string => {
	const clone = el?.cloneNode(true) as Element | undefined;
	for (const decoration of clone?.querySelectorAll('[aria-hidden="true"]') ?? [])
		decoration.remove();
	return (clone?.textContent ?? '').replace(/\s+/g, ' ').trim();
};
