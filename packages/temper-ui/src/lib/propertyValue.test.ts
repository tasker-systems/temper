import { describe, expect, it } from 'vitest';
import { classifyValue } from './propertyValue';

describe('classifyValue', () => {
	it('classifies a string as scalar', () => {
		expect(classifyValue('resolved')).toEqual({ kind: 'scalar', text: 'resolved' });
	});

	it('classifies numbers and booleans as scalar, stringified', () => {
		expect(classifyValue(0)).toEqual({ kind: 'scalar', text: '0' });
		expect(classifyValue(false)).toEqual({ kind: 'scalar', text: 'false' });
	});

	it('classifies null as a scalar em-dash, never a crash', () => {
		expect(classifyValue(null)).toEqual({ kind: 'scalar', text: '—' });
	});

	it('classifies an object with its key count', () => {
		const v = { node_label: 'question', status: 'resolved' };
		expect(classifyValue(v)).toEqual({
			kind: 'object',
			entries: [
				['node_label', 'question'],
				['status', 'resolved']
			],
			summary: '{2 keys}'
		});
	});

	it('singularizes a one-key object', () => {
		expect(classifyValue({ a: 1 })).toMatchObject({ summary: '{1 key}' });
	});

	it('classifies an array with its length', () => {
		expect(classifyValue(['a', 'b'])).toEqual({
			kind: 'array',
			items: ['a', 'b'],
			summary: '[2]'
		});
	});

	it('treats an empty object/array as scalar — nothing to expand into', () => {
		expect(classifyValue({})).toEqual({ kind: 'scalar', text: '{}' });
		expect(classifyValue([])).toEqual({ kind: 'scalar', text: '[]' });
	});

	it('preserves insertion order of object entries', () => {
		const v = { zebra: 1, alpha: 2 };
		expect(classifyValue(v)).toMatchObject({
			entries: [
				['zebra', 1],
				['alpha', 2]
			]
		});
	});

	it('handles the real nested [theme] facet', () => {
		const facet = {
			node_label: 'theme',
			status: 'active',
			priority: 'high',
			slices_shipped: ['T1 team read', 'T2 invitations'],
			next_slice: 'T3 resource ownership transfer'
		};
		const got = classifyValue(facet);
		expect(got).toMatchObject({ kind: 'object', summary: '{5 keys}' });
		// the nested array is left raw — the component recurses into it
		expect(classifyValue((got as { entries: [string, unknown][] }).entries[3][1])).toMatchObject({
			kind: 'array',
			summary: '[2]'
		});
	});
});
