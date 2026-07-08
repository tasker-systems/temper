import { describe, expect, it } from 'vitest';
import { nodeMarkShape } from './marks';

describe('nodeMarkShape', () => {
	it('renders cogmap facets (ideas) as circles', () => {
		expect(nodeMarkShape('cogmap')).toBe('circle');
	});

	it('renders context resources (the builder axis) as document-squares', () => {
		expect(nodeMarkShape('context')).toBe('square');
	});
});
