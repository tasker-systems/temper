import { describe, expect, it } from 'vitest';
import { nodeMarkShape } from './marks';

// `groupByAxis`'s test went with the function in Beat D — see marks.ts for why the a11y
// mirror now groups by arm rather than by axis.
describe('nodeMarkShape', () => {
	it('renders cogmap facets (ideas) as circles', () => {
		expect(nodeMarkShape('cogmap')).toBe('circle');
	});

	it('renders context resources (the builder axis) as document-squares', () => {
		expect(nodeMarkShape('context')).toBe('square');
	});
});
