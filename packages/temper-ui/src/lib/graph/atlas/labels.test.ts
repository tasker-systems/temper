import { describe, it, expect } from 'vitest';
import { labelAnchors, truncateLabel } from './labels';

describe('labelAnchors', () => {
	const nodes = [
		{ id: 'seed', degree: 1 },
		{ id: 'a', degree: 9 },
		{ id: 'b', degree: 7 },
		{ id: 'c', degree: 3 },
		{ id: 'd', degree: 2 }
	];
	it('always includes the seed plus the top-K by degree', () => {
		const set = labelAnchors(nodes, 'seed', 2);
		expect(set.has('seed')).toBe(true);
		expect(set.has('a')).toBe(true);
		expect(set.has('b')).toBe(true);
		expect(set.has('c')).toBe(false);
	});
	it('does not double-count the seed if it is high-degree', () => {
		const set = labelAnchors([{ id: 'seed', degree: 99 }, { id: 'a', degree: 5 }, { id: 'b', degree: 4 }], 'seed', 2);
		expect(set).toEqual(new Set(['seed', 'a', 'b']));
	});
});

describe('truncateLabel', () => {
	it('leaves short titles', () => expect(truncateLabel('Short', 20)).toBe('Short'));
	it('truncates with an ellipsis', () => expect(truncateLabel('A very long node title here', 10)).toBe('A very lo…'));
});
