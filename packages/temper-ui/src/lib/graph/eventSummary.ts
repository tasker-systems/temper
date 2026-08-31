// eventSummary.ts — a one-line, best-effort summary of an event for the collapsed
// history row (the graph surface's `NodeRail`, and the vault's `EventHistory`; the Atlas's
// `TrailRail` was the original consumer and went in Beat D). Payload-first; relationship
// summaries resolve a target
// TITLE from the loaded subgraph nodes when present, else fall back to the label.
// Never throws — a malformed/unknown payload yields null (row shows kind + actor only).
export function summarizeEvent(
	kind: string,
	payload: unknown,
	nodesById?: Map<string, { title: string }>,
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
		case 'data_artifact_committed': {
			const family = str(p.artifact_kind);
			if (!family) return null;
			const parts = [
				family,
				str(p.intent),
				humanBytes(p.content_bytes),
				hashPrefix(p.content_hash),
			];
			if (Array.isArray(p.supersedes) && p.supersedes.length > 0) {
				parts.push(`supersedes ${p.supersedes.length}`);
			}
			return parts.filter((s) => s !== null).join(' · ');
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

/** 1024-based size with one decimal, trailing .0 trimmed — a legibility aid;
 *  the payload's `content_bytes` stays the exact count. */
function humanBytes(v: unknown): string | null {
	if (typeof v !== 'number' || !Number.isFinite(v) || v < 0) return null;
	const units = ['B', 'KB', 'MB', 'GB', 'TB'];
	let n = v;
	let u = 0;
	while (n >= 1024 && u < units.length - 1) {
		n /= 1024;
		u += 1;
	}
	const magnitude = u === 0 ? String(n) : n.toFixed(1).replace(/\.0$/, '');
	return `${magnitude} ${units[u]}`;
}

/** A short, honestly-labeled hash prefix — a pointer to the content's identity,
 *  not the content. The ledger carries only the hash; the summary says so. */
function hashPrefix(v: unknown): string | null {
	if (typeof v !== 'string' || v.length < 8) return null;
	return `sha256:${v.slice(0, 8)}…`;
}
