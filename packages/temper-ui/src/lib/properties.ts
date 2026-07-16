/**
 * One property as the page renders it. The managed/open split is a read-time
 * projection over a single flat `kb_properties` store — this merges it back.
 * `managed` survives only as a presentation hint (managed keys tint toward the
 * doc-type hue); it is not a storage fact.
 */
export interface PropertyRow {
	key: string;
	value: unknown;
	managed: boolean;
}

/**
 * The ten managed keys, in render order. Mirrors `MANAGED_PROPERTY_KEYS` in
 * `crates/temper-substrate/src/keys.rs:42`. Order here is editorial (workflow
 * state first, provenance last); the Rust const's order is not meaningful.
 *
 * A `temper-*` key NOT in this list is an open key — the same inverse fate the
 * substrate's `is_managed_property_key` applies.
 */
export const MANAGED_KEY_ORDER = [
	'temper-stage',
	'temper-mode',
	'temper-effort',
	'temper-status',
	'temper-seq',
	'temper-llm-model',
	'temper-llm-run',
	'temper-provenance',
	'temper-branch',
	'temper-pr'
] as const;

const MANAGED_RANK = new Map<string, number>(MANAGED_KEY_ORDER.map((k, i) => [k, i]));

/**
 * Merge both meta tiers into one ordered property set (spec D2):
 * `doc_type` first, then managed keys in `MANAGED_KEY_ORDER`, then open keys
 * alphabetically. Null-valued keys are dropped — the substrate never stores a
 * null property value, so a null here means "absent", not "set to nothing".
 */
export function mergeProperties(
	managed: Record<string, unknown> | null | undefined,
	open: Record<string, unknown> | null | undefined,
	docType: string
): PropertyRow[] {
	const managedRows: PropertyRow[] = [];
	const openRows: PropertyRow[] = [];

	for (const [key, value] of Object.entries(managed ?? {})) {
		if (value === null || value === undefined) continue;
		if (MANAGED_RANK.has(key)) managedRows.push({ key, value, managed: true });
		else openRows.push({ key, value, managed: false });
	}
	for (const [key, value] of Object.entries(open ?? {})) {
		if (value === null || value === undefined) continue;
		openRows.push({ key, value, managed: false });
	}

	managedRows.sort((a, b) => MANAGED_RANK.get(a.key)! - MANAGED_RANK.get(b.key)!);
	openRows.sort((a, b) => a.key.localeCompare(b.key));

	return [{ key: 'doc_type', value: docType, managed: true }, ...managedRows, ...openRows];
}
