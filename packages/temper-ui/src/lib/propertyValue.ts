/**
 * What a property value IS, so the renderer can stay declarative (spec D5).
 * One key = one row: a scalar renders inline; an object/array collapses to a
 * summary and expands on demand. Depth is opt-in, so the property set always
 * reads as the resource's key set.
 *
 * Deliberately NOT `flattenPayload` (lib/graph/atlas/payloadRows.ts): that
 * flattens `facet` into sibling dot-path rows sitting at the same visual level
 * as `date` and `tags`, which stops the set reading as a key set. See spec D5.
 */
export type ClassifiedValue =
	| { kind: 'scalar'; text: string }
	| { kind: 'object'; entries: [string, unknown][]; summary: string }
	| { kind: 'array'; items: unknown[]; summary: string };

/** Classify one JSONB property value. Total — never throws, for any input. */
export function classifyValue(value: unknown): ClassifiedValue {
	if (value === null || value === undefined) return { kind: 'scalar', text: '—' };

	if (Array.isArray(value)) {
		if (value.length === 0) return { kind: 'scalar', text: '[]' };
		return { kind: 'array', items: value, summary: `[${value.length}]` };
	}

	if (typeof value === 'object') {
		const entries = Object.entries(value as Record<string, unknown>);
		if (entries.length === 0) return { kind: 'scalar', text: '{}' };
		return {
			kind: 'object',
			entries,
			summary: `{${entries.length} ${entries.length === 1 ? 'key' : 'keys'}}`
		};
	}

	return { kind: 'scalar', text: String(value) };
}
