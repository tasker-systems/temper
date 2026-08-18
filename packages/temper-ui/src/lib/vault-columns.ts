/**
 * One column as `wx-svelte-grid` expects it (`VaultGrid.svelte:34-40`). `id` is
 * both the grid's cell key and, for managed columns, the `managed_meta` lookup
 * key Task 6 reads cell data with — so it stays a `temper-*` name even where
 * that differs from what the door will sort by.
 */
export interface VaultColumn {
	id: string;
	header: string;
	width?: number;
	flexgrow?: number;
	sort: boolean;
	/** The `ResourceSortField` name to send, when it differs from `id`. */
	sortKey?: string;
}

/**
 * The one or zero managed keys, per kind, that earn a dedicated grid column
 * when the visible set narrows to that kind alone. Deliberately a strict
 * subset of what each kind's JSON Schema declares (`crates/temper-workflow/schemas/`)
 * — most managed keys never get a column. `temper-mode`/`temper-effort` are
 * excluded on purpose (spec D7): they are pre-work estimates revised during
 * the work, so a column ranked on them would rank by a stale prediction.
 *
 * The drift guard in `vault-columns.test.ts` keeps this honest against the
 * schemas without demanding equality — `MANAGED_KEY_ORDER` (`properties.ts:21`)
 * plays the same role for the property list and never got one.
 */
export const KIND_KEYS: Readonly<Record<string, readonly string[]>> = {
	task: ['temper-stage'],
	goal: ['temper-status'],
};

/**
 * The sort field to send for a managed key's column, when it differs from the
 * `temper-*` id. Only keys present here are sortable at all — `temper-status`
 * has no entry because `status` isn't in `ResourceSortField`, and offering a
 * sort the door will reject would be a header that lies.
 */
const SORT_KEY_BY_MANAGED_KEY: Readonly<Record<string, string>> = {
	'temper-stage': 'stage',
};

const HEADER_BY_MANAGED_KEY: Readonly<Record<string, string>> = {
	'temper-stage': 'Stage',
	'temper-status': 'Status',
};

/**
 * Build the managed-key column for one kind's dedicated key, in the shape
 * `VaultGrid.svelte` passes to the grid. `id` stays the `temper-*` name (Task
 * 6 reads `r.managed_meta[id]`); `sortKey` carries the door-facing name only
 * when the key is actually sortable.
 */
function managedColumn(key: string): VaultColumn {
	const sortKey = SORT_KEY_BY_MANAGED_KEY[key];
	return {
		id: key,
		header: HEADER_BY_MANAGED_KEY[key] ?? key,
		width: 100,
		sort: sortKey !== undefined,
		...(sortKey !== undefined ? { sortKey } : {}),
	};
}

/**
 * Derive the vault grid's columns for the currently visible set. `kind` is
 * the shared `temper-type` when every visible row is the same kind, or `null`
 * when the set is mixed.
 *
 * On a mixed set, `doc_type_name` (the "Type" column) is the only way to tell
 * rows apart, so it stays. Once the set narrows to one kind, Type is
 * redundant — every row would show the same value — so it drops in favor of
 * that kind's one distinguishing managed column, if it has one (`KIND_KEYS`).
 * A kind with no dedicated key (e.g. `research`) still drops Type; it just
 * doesn't gain a replacement.
 */
export function columnsFor(kind: string | null): VaultColumn[] {
	const columns: VaultColumn[] = [
		{ id: 'title', header: 'Title', flexgrow: 1, sort: true },
		{ id: 'context_name', header: 'Context', width: 140, sort: true },
	];

	if (kind === null) {
		columns.push({ id: 'doc_type_name', header: 'Type', width: 120, sort: true });
	} else {
		for (const key of KIND_KEYS[kind] ?? []) {
			columns.push(managedColumn(key));
		}
	}

	columns.push({ id: 'updated', header: 'Updated', width: 110, sort: true });
	return columns;
}
