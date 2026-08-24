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
	/**
	 * The closed vocabulary this state may be changed to, when the surface is offering the
	 * change. Absent means **no control is offered** — either the reader may not change this
	 * resource, or the field carries no vocabulary, or none could be read.
	 *
	 * Never assembled here. It comes from `DocTypeDescription.enum_fields`
	 * (`GET /api/schema/doc-types/{name}`), derived from the doc-type's own schema.
	 *
	 * A row with `choices` and `value: null` is a state the work carries and this resource
	 * does not currently hold — offered so the reader can set it. It is the one place a
	 * null-valued row survives.
	 */
	choices?: readonly string[];
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
 * fields away, but the generated TS types them `T | null`, so both spellings arrive.) The one
 * exception is a state the work carries and this resource has not got: see `choices`.
 *
 * `enumFields` is `DocTypeDescription.enum_fields` — which values each field of this kind of
 * work takes, read from its own schema. Pass `null` to offer nothing: that is the shape for a
 * reader who may not change this resource, for a doc type with no schema, and for a read that
 * failed. Only the last is a degradation, and offering nothing is the honest response to all
 * three.
 */
export function mergeProperties(
	managed: Record<string, unknown> | null | undefined,
	open: Record<string, unknown> | null | undefined,
	docType: string,
	enumFields: Readonly<Record<string, readonly string[]>> | null = null,
): PropertyRow[] {
	const managedRows: PropertyRow[] = [];
	const openRows: PropertyRow[] = [];
	const offered = new Set(Object.keys(enumFields ?? {}));

	for (const [key, value] of Object.entries(managed ?? {})) {
		if (value === null || value === undefined) continue;
		const choices = enumFields?.[key];
		managedRows.push(
			choices ? { key, value, managed: true, choices } : { key, value, managed: true },
		);
		offered.delete(key);
	}
	// "No more, no FEWER." A state the kind of work carries but this resource does not hold
	// yet is still one of its states; decorating only the rows that already exist would
	// present a subset, and leave the reader unable to set what is missing.
	for (const key of offered) {
		// biome-ignore lint/style/noNonNullAssertion: `offered` is keyed from `enumFields`.
		managedRows.push({ key, value: null, managed: true, choices: enumFields![key] });
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
