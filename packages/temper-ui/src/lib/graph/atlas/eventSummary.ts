// eventSummary.ts — a one-line, best-effort summary of an event for the collapsed
// TrailRail history row. Payload-first; relationship summaries resolve a target
// TITLE from the loaded subgraph nodes when present, else fall back to the label.
// Never throws — a malformed/unknown payload yields null (row shows kind + actor only).
export function summarizeEvent(
	kind: string,
	payload: unknown,
	nodesById?: Map<string, { title: string }>
): string | null {
	if (payload === null || typeof payload !== 'object') return null;
	const p = payload as Record<string, unknown>;
	switch (kind) {
		case 'property_set':
		case 'property_asserted': {
			const key = str(p.property_key);
			if (!key) return null;
			const val = 'value' in p ? scalarish(p.value) : null;
			return val === null ? key : `${key} → ${val}`;
		}
		case 'relationship_asserted':
		case 'relationship_retyped':
		case 'relationship_reweighted': {
			const label = str(p.label) ?? str(p.edge_kind);
			const targetId = str((p.target as Record<string, unknown> | undefined)?.id);
			const title = targetId ? nodesById?.get(targetId)?.title : undefined;
			if (label && title) return `${label} → ${title}`;
			return label ?? null;
		}
		default:
			return null;
	}
}

function str(v: unknown): string | null {
	return typeof v === 'string' && v.length > 0 ? v : null;
}
function scalarish(v: unknown): string | null {
	if (typeof v === 'string') return v;
	if (typeof v === 'number' || typeof v === 'boolean') return String(v);
	return null;
}
