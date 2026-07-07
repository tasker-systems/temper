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

/** Greedy word-wrap into ≤ maxLines lines of ≤ cap chars; final line ellipsis-truncated. */
export function wrapLabel(text: string, cap: number, maxLines = 2): string[] {
	if (text.length <= cap) return [text];
	const words = text.split(/\s+/).filter(Boolean);
	const lines: string[] = [];
	let cur = '';
	for (let i = 0; i < words.length; i++) {
		const cand = cur ? `${cur} ${words[i]}` : words[i];
		if (cand.length <= cap || !cur) {
			cur = cand;
		} else {
			lines.push(cur);
			cur = words[i];
		}
		if (lines.length === maxLines - 1) {
			const rest = [cur, ...words.slice(i + 1)].join(' ');
			lines.push(truncateLabel(rest, cap));
			return lines;
		}
	}
	if (cur) lines.push(truncateLabel(cur, cap));
	return lines;
}

/** Salience → field intensity (0..1). Exponent > 1 widens the salient/tail separation. */
export function intensityOf(salience: number | null, maxSalience: number): number {
	if (maxSalience <= 0) return 0;
	return Math.pow(Math.min(1, (salience ?? 0) / maxSalience), 1.4);
}

/** Field-effect style from intensity: brighter fill/stroke + wider glow for salient regions. */
export function fieldStyle(intensity: number, ghost: boolean) {
	if (ghost) return { fillOpacity: 0.04, strokeOpacity: 0.2, glowPx: 0 };
	return {
		fillOpacity: 0.05 + intensity * 0.3,
		strokeOpacity: 0.25 + intensity * 0.5,
		glowPx: 1 + intensity * 11
	};
}

/** The top-K regions by salience — the ones that draw an in-panorama label. */
export function labeledRegionIds(
	regions: { id: string; salience: number | null }[],
	k: number
): Set<string> {
	return new Set(
		[...regions]
			.sort((a, b) => (b.salience ?? 0) - (a.salience ?? 0))
			.slice(0, k)
			.map((r) => r.id)
	);
}
