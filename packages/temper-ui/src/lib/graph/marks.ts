// marks.ts — pure mark-encoding decisions for the graph canvas.
//
// Shape encodes the *axis*, color encodes doc-type. A cogmap facet is an idea in the
// map (circle); a context-homed resource is the work it was derived_from — a document
// (rounded square). `home`, not `doc_type`, drives the shape, so a steward-distilled
// facet whose doc_type is "session" still reads as an idea, while its context twin
// reads as a document.
//
// `[Beat D — 2026-08-20]` `groupByAxis` was the a11y mirror of this rule and went with
// `CompositionA11yList`. The successor's `GraphA11yList` groups by ARM — how each node
// reached the answer — which is what the reader asked about; `presentation.ts` owns
// that vocabulary.

export type NodeMarkShape = 'circle' | 'square';

export function nodeMarkShape(home: 'context' | 'cogmap'): NodeMarkShape {
	return home === 'cogmap' ? 'circle' : 'square';
}
