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
 * Editorial render order for the managed run — **a ranking, not the set.**
 *
 * It used to be both, and being both is what made it a restatement: a key added to
 * `MANAGED_PROPERTY_KEYS` (`crates/temper-substrate/src/keys.rs:42`) that nobody
 * remembered to add here rendered in the OPEN run — untinted, alphabetical, on the wrong
 * side of the rule — with nothing detecting it.
 *
 * Membership is not this file's to decide and never needed to be. The server splits the
 * tiers at readback (`temper-substrate/src/readback/mod.rs:437`, via
 * `is_managed_property_key`), and `ManagedMeta` is a closed typed record with
 * `deny_unknown_fields`, so **every key arriving in `mergeProperties`' `managed` argument
 * is managed by construction**. The old membership branch here could not fire.
 *
 * What remains is a preference about sequence — workflow state first, provenance last —
 * which the server does not own and does not intend to: the Rust const's order is declared
 * *not meaningful*. A managed key this list has no opinion about is still managed; it sorts
 * after the ranked ones, alphabetically, inside the managed run.
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
	'temper-pr',
] as const;

const MANAGED_RANK = new Map<string, number>(MANAGED_KEY_ORDER.map((k, i) => [k, i]));

/** Rank for the managed run: ranked keys in `MANAGED_KEY_ORDER`, then the rest. */
const UNRANKED = MANAGED_KEY_ORDER.length;

/**
 * Merge both meta tiers into one ordered property set (spec D2):
 * `doc_type` first, then the managed run, then open keys alphabetically.
 *
 * **Which tier a key belongs to is decided by which argument it arrives in**, not by any
 * list here — see `MANAGED_KEY_ORDER`. Within the managed run, editorially ranked keys lead
 * and the rest follow alphabetically.
 *
 * Null-valued keys are dropped — the substrate never stores a null property value, so a
 * null here means "absent", not "set to nothing". (`ManagedMeta` serializes its unset
 * fields away, but the generated TS types them `T | null`, so both spellings arrive.)
 */
export function mergeProperties(
	managed: Record<string, unknown> | null | undefined,
	open: Record<string, unknown> | null | undefined,
	docType: string,
): PropertyRow[] {
	const managedRows: PropertyRow[] = [];
	const openRows: PropertyRow[] = [];

	for (const [key, value] of Object.entries(managed ?? {})) {
		if (value === null || value === undefined) continue;
		managedRows.push({ key, value, managed: true });
	}
	for (const [key, value] of Object.entries(open ?? {})) {
		if (value === null || value === undefined) continue;
		openRows.push({ key, value, managed: false });
	}

	managedRows.sort((a, b) => {
		const ra = MANAGED_RANK.get(a.key) ?? UNRANKED;
		const rb = MANAGED_RANK.get(b.key) ?? UNRANKED;
		return ra === rb ? a.key.localeCompare(b.key) : ra - rb;
	});
	openRows.sort((a, b) => a.key.localeCompare(b.key));

	return [{ key: 'doc_type', value: docType, managed: true }, ...managedRows, ...openRows];
}
