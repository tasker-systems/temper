/** Ids whose labels are always drawn at Tier 2: the seed plus the top-K by degree. */
export function labelAnchors(
	nodes: { id: string; degree: number }[],
	seedId: string,
	k: number
): Set<string> {
	const ranked = nodes
		.filter((n) => n.id !== seedId)
		.sort((a, b) => b.degree - a.degree)
		.slice(0, k)
		.map((n) => n.id);
	return new Set([seedId, ...ranked]);
}

/** Truncate a title to `max` chars with a trailing ellipsis. */
export function truncateLabel(title: string, max: number): string {
	return title.length <= max ? title : `${title.slice(0, max - 1)}…`;
}
