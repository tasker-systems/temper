// graph-reads.paths.test.ts
import { describe, expect, it } from 'vitest';
import { resourceEdgesPath, resourceRowPath, teamsListPath, trailPath } from './graph-reads';

// The nine `/api/graph/*` path builders that lived here were deleted with their callers in
// Beat D; the endpoints survive with none. See graph-reads.ts for why that leftover is named.
describe('graph API path builders', () => {
	it('R5 element trail', () => {
		expect(trailPath('node', 'n1')).toBe('/api/graph/elements/node/n1/trail');
		expect(trailPath('edge', 'e1')).toBe('/api/graph/elements/edge/e1/trail');
	});
	it('teams list', () => {
		expect(teamsListPath()).toBe('/api/teams');
	});
	it('builds the resource row path', () => {
		expect(resourceRowPath('019f420c-cf01-7bc1-87c9-09684b0fa69e')).toBe(
			'/api/resources/019f420c-cf01-7bc1-87c9-09684b0fa69e',
		);
	});
	it('builds the resource edges path', () => {
		expect(resourceEdgesPath('019f420c-cf01-7bc1-87c9-09684b0fa69e')).toBe(
			'/api/resources/019f420c-cf01-7bc1-87c9-09684b0fa69e/edges',
		);
	});
});
