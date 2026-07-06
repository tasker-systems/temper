import { describe, expect, it } from 'vitest';
import { flattenPayload } from './payloadRows';

describe('flattenPayload', () => {
	it('renders scalar keys as key/value rows', () => {
		expect(flattenPayload({ property_key: 'stage', weight: 1 })).toEqual([
			{ key: 'property_key', value: 'stage' },
			{ key: 'weight', value: '1' }
		]);
	});
	it('dot-paths nested objects', () => {
		expect(flattenPayload({ owner: { table: 'kb_resources', id: 'x' } })).toEqual([
			{ key: 'owner.table', value: 'kb_resources' },
			{ key: 'owner.id', value: 'x' }
		]);
	});
	it('json-encodes arrays and stringifies null', () => {
		expect(flattenPayload({ tags: ['a', 'b'], note: null })).toEqual([
			{ key: 'tags', value: '["a","b"]' },
			{ key: 'note', value: 'null' }
		]);
	});
	it('returns [] for non-objects', () => {
		expect(flattenPayload('nope')).toEqual([]);
		expect(flattenPayload(null)).toEqual([]);
	});
});
