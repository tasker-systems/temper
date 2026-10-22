/**
 * The omission sentence every bounded view ends with, composed from its numbers so no author words
 * it out of step with them. A view that is a page of something larger says which rows it shows and
 * what lies either side: `more` is the answer's own word that rows follow, never a guess from how
 * many rows arrived.
 *
 * A graph view composes its omissions in the read's own terms: a walk says how deep it went and
 * that deeper was not reported — there is no denominator to count, and none is invented; an entry
 * read says drawn of eligible and how many in-scope nodes it did not draw for having no
 * connections; a cut to the drawing's bounds is counted, never silent.
 */
export type Page = { offset: number; size: number; more: boolean };

/** Where a graph view was read from, as its read answers it. */
export type GraphArm =
	| { read: 'walk'; from: string[]; depth: number }
	| { read: 'entry'; in: string[]; k: number };

/** What an entry read says it drew. */
export type GraphBounds = { drawn: number; eligible: number; inScope: number; truncated: boolean };

/** What the drawing's bounds clipped off the answer, each part present only when some were. */
export type GraphCut = { nodes?: number; edges?: number };

/** The read behind a graph view, as its omission clauses are composed from it. */
export type GraphOmissions = {
	arm: GraphArm | null;
	bounds: GraphBounds | null;
	cut: GraphCut | null;
};

export function omissionSentence(
	total: number,
	shown: number,
	scope: string,
	page?: Page | null,
): string {
	if (!page) {
		const omitted = Math.max(0, total - shown);
		return omitted === 0
			? `All ${total} ${scope}.`
			: `${shown} of ${total} ${scope}; ${omitted} not shown.`;
	}
	const { offset, more } = page;
	if (offset === 0 && !more && shown === total) return `All ${total} ${scope}.`;
	const after = more ? Math.max(0, total - offset - shown) : 0;
	const either = [offset > 0 && `${offset} before this page`, more && `${after} after it`]
		.filter(Boolean)
		.join(', ');
	const rows = shown === 0 ? `None of ${total}` : `${offset + 1}–${offset + shown} of ${total}`;
	return either ? `${rows} ${scope}; ${either}.` : `${rows} ${scope}.`;
}

const counted = (n: number, word: string): string => `${n} ${word}${n === 1 ? '' : 's'}`;

/** The clause naming where a graph figure was read from, for its description. */
export function graphStanding(arm: GraphArm, nameOf: (id: string) => string): string {
	if (arm.read === 'walk')
		return `reached from ${arm.from.map(nameOf).join(', ')} within ${arm.depth} hops`;
	return `entered at ${arm.in.map(nameOf).join(', ')} within a draw of ${arm.k}`;
}

/** The omission sentence for a graph view, composed from the read's own numbers. */
export function graphSentence(
	total: number,
	shown: number,
	scope: string,
	{ arm, bounds, cut }: GraphOmissions,
): string {
	if (!arm) return omissionSentence(total, shown, scope, null);
	const clauses: string[] = [];
	let head: string;
	if (arm.read === 'walk') {
		head = shown === total ? `All ${total} ${scope}` : `${shown} of ${total} ${scope}`;
		clauses.push('deeper not reported');
	} else {
		const { drawn, eligible, inScope } = bounds ?? {
			drawn: total,
			eligible: total,
			inScope: total,
			truncated: false,
		};
		head =
			eligible > drawn
				? `Drawn ${drawn} of ${eligible} eligible, ${scope} at a draw of ${arm.k}`
				: `All ${total} ${scope} at a draw of ${arm.k}`;
		const unconnected = Math.max(0, inScope - eligible);
		if (unconnected > 0) clauses.push(`${unconnected} in scope not connected, not drawn`);
	}
	if (cut) {
		const parts = [
			cut.nodes ? counted(cut.nodes, 'node') : null,
			cut.edges ? counted(cut.edges, 'edge') : null,
		].filter(Boolean);
		clauses.push(`${parts.join(' and ')} past the drawing’s bounds not drawn`);
	}
	return clauses.length ? `${head}; ${clauses.join('; ')}.` : `${head}.`;
}
