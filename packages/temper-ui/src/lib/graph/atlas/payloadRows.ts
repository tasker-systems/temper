// payloadRows.ts — flatten an event payload (schemaless jsonb) into ordered
// key/value rows for the TrailRail's expandable event detail. Nested objects
// dot-path; arrays and other non-scalars are JSON-encoded. One generic renderer
// for every event type — no per-kind logic.
export interface PayloadRow {
	key: string;
	value: string;
}

export function flattenPayload(value: unknown, prefix = ''): PayloadRow[] {
	if (value === null || typeof value !== 'object' || Array.isArray(value)) {
		return prefix ? [{ key: prefix, value: scalar(value) }] : [];
	}
	const rows: PayloadRow[] = [];
	for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
		const key = prefix ? `${prefix}.${k}` : k;
		if (v !== null && typeof v === 'object' && !Array.isArray(v)) {
			rows.push(...flattenPayload(v, key));
		} else {
			rows.push({ key, value: scalar(v) });
		}
	}
	return rows;
}

function scalar(v: unknown): string {
	if (v === null) return 'null';
	if (typeof v === 'string') return v;
	if (typeof v === 'number' || typeof v === 'boolean') return String(v);
	return JSON.stringify(v);
}
