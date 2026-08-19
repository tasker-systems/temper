/**
 * Shared fixtures over GENERATED types. `ResourceView` is emitted by ts-rs
 * (`$lib/types/generated/resource_view.ts`), so a field added on the Rust side changes every
 * literal that spells the type out in full. One factory with a `Partial` override is the only
 * shape that survives that regeneration; three hand-copied object literals do not.
 */
import type { ResourceView } from '$lib/types/generated/resource_view';

/**
 * The all-null managed tier. The hoisted `stage`/`seq`/`mode`/`effort` columns are gone; every
 * managed value lives in this always-present tier under its canonical `temper-*` name. Exported
 * so a test can spread it and override one key — `managed_meta` is a total type, so a bare
 * `{ 'temper-stage': 'design' }` override would not typecheck.
 */
export const MANAGED: ResourceView['managed_meta'] = {
	'temper-stage': null,
	'temper-mode': null,
	'temper-effort': null,
	'temper-status': null,
	'temper-seq': null,
	'temper-branch': null,
	'temper-pr': null,
	'temper-llm-model': null,
	'temper-llm-run': null,
	'temper-provenance': null,
};

export const ROW_ID = '019f420c-cf01-7bc1-87c9-09684b0fa69e';

/**
 * One resource row, overridable per test.
 *
 * **This is not the wire shape.** It assigns `cogmap_id: null`, which is what `ResourceRow`
 * puts on the wire and NOT what `ResourceView` does — `ResourceView` omits the key entirely.
 * That discrepancy is why the always-true comparison in `HomeChip` survived review: every test
 * fed it the old shape. Reach for `withoutKey` when the absence of a key is the thing under test.
 */
export function makeRow(partial: Partial<ResourceView> = {}): ResourceView {
	return {
		id: ROW_ID,
		ref: `t-${ROW_ID}`,
		kb_context_id: '00000000-0000-0000-0003-000000000001',
		origin_uri: '',
		title: 'T',
		originator_profile_id: '00000000-0000-0000-0000-000000000001',
		owner_profile_id: '00000000-0000-0000-0000-000000000001',
		is_active: true,
		created: '2026-07-08T00:00:00Z',
		updated: '2026-07-08T00:00:00Z',
		context_name: 'Temper',
		doc_type_name: 'task',
		owner_handle: 'j-cole-taylor',
		context_slug: 'temper',
		context_owner_ref: '@j-cole-taylor',
		context_ref: '@j-cole-taylor/temper',
		cogmap_id: null,
		cogmap_name: null,
		body_hash: null,
		ingest_state: 'complete',
		body_storage: 'derived',
		managed_meta: { ...MANAGED },
		open_meta: null,
		content: null,
		...partial,
	};
}

/**
 * Drop a key entirely, rather than setting it to null — see the `cogmap_id` note on `makeRow`.
 */
export function withoutKey<K extends keyof ResourceView>(row: ResourceView, key: K): ResourceView {
	const { [key]: _omitted, ...rest } = row;
	return rest as ResourceView;
}
