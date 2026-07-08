// marks.ts — pure mark-encoding decisions for the Atlas force-graph.
//
// Beat D: shape encodes the *axis*, color encodes doc-type. A cogmap facet is an
// idea in the map (circle); a context-homed resource is the work it was
// derived_from — a document (rounded square). `home`, not `doc_type`, drives the
// shape, so a steward-distilled facet whose doc_type is "session" still reads as an
// idea, while its context twin reads as a document.

export type NodeMarkShape = 'circle' | 'square';

export function nodeMarkShape(home: 'context' | 'cogmap'): NodeMarkShape {
	return home === 'cogmap' ? 'circle' : 'square';
}
