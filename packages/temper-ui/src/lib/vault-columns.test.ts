import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { MANAGED_KEY_ORDER } from './properties';
import { columnsFor, KIND_KEYS } from './vault-columns';

const schemaProps = (kind: string): string[] => {
	const path = new URL(
		`../../../../crates/temper-workflow/schemas/${kind}.schema.json`,
		import.meta.url,
	);
	return Object.keys(JSON.parse(readFileSync(path, 'utf8')).properties ?? {});
};

describe('KIND_KEYS drift guard', () => {
	it('names only keys the kind actually declares', () => {
		for (const [kind, keys] of Object.entries(KIND_KEYS)) {
			const declared = schemaProps(kind);
			for (const key of keys) {
				expect(declared, `${kind}.schema.json must declare ${key}`).toContain(key);
			}
		}
	});

	it('names only keys scoped to that kind and no other', () => {
		expect(schemaProps('goal')).not.toContain('temper-stage');
		expect(schemaProps('task')).not.toContain('temper-status');
	});

	it('uses keys that MANAGED_KEY_ORDER knows how to order', () => {
		for (const keys of Object.values(KIND_KEYS)) {
			for (const key of keys) {
				expect(MANAGED_KEY_ORDER).toContain(key);
			}
		}
	});
});

describe('columnsFor', () => {
	it('shows Type on a mixed set', () => {
		expect(columnsFor(null).map((c) => c.id)).toEqual([
			'title',
			'context_name',
			'doc_type_name',
			'updated',
		]);
	});

	it('drops Type and reveals stage for an all-task set', () => {
		expect(columnsFor('task').map((c) => c.id)).toEqual([
			'title',
			'context_name',
			'temper-stage',
			'updated',
		]);
	});

	it('drops Type and reveals status for an all-goal set', () => {
		expect(columnsFor('goal').map((c) => c.id)).toEqual([
			'title',
			'context_name',
			'temper-status',
			'updated',
		]);
	});

	it('drops Type for a kind with no managed keys of its own', () => {
		expect(columnsFor('research').map((c) => c.id)).toEqual(['title', 'context_name', 'updated']);
	});

	// status is absent from ResourceSortField, so offering a sort would overstate.
	it('marks the status column unsortable', () => {
		expect(columnsFor('goal').find((c) => c.id === 'temper-status')!.sort).toBe(false);
	});

	it('marks the stage column sortable', () => {
		expect(columnsFor('task').find((c) => c.id === 'temper-stage')!.sort).toBe(true);
	});

	// The id is the managed_meta key; the sort field the door accepts is `stage`.
	// Sending `sort=temper-stage` is rejected as an invalid ResourceSortField.
	it('carries a sortKey for the stage column, distinct from its id', () => {
		expect(columnsFor('task').find((c) => c.id === 'temper-stage')!.sortKey).toBe('stage');
	});

	it('gives no sortKey to columns whose id is already the sort field', () => {
		expect(columnsFor(null).find((c) => c.id === 'title')!.sortKey).toBeUndefined();
	});
});
