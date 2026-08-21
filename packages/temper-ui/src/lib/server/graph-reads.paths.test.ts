// graph-reads.paths.test.ts
import { describe, expect, it } from 'vitest';
import { traversePath } from './graph-query';
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

// The successor surface's own reads live in `graph-query.ts`; this one is here because the file
// above is where a built path is pinned, and a path built wrong is the whole of the failure.
describe('the traversal path', () => {
	it('spells the seeds COMMA-SEPARATED, not as repeated params', () => {
		// The two halves of this grammar disagree on purpose and must be joined here. The page
		// spells the list `?from=a&from=b` (`params.getAll('from')` in `vault-url.ts`); the endpoint
		// spells it `?from=a,b` (`q.from.split(',')` in `handlers/graph.rs`). Passing the page's
		// repeated form through hands the service one unparseable uuid and 400s every hop.
		expect(traversePath(['a', 'b'], 1)).toBe('/api/graph/traverse?from=a%2Cb&depth=1');
	});

	it('omits depth entirely when the caller names none, rather than writing a default in', () => {
		// The default is the handler's: `depth: Option<i32>` … `unwrap_or(1)`. A second copy here
		// would be two spellings of one rule with nothing linking them.
		expect(traversePath(['a'], null)).toBe('/api/graph/traverse?from=a');
	});
});
