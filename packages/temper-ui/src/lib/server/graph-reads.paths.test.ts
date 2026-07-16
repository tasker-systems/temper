// graph-reads.paths.test.ts
import { describe, expect, it } from 'vitest';
import {
	atlasHomePath,
	cogmapPanoramaPath,
	contextCompositionPath,
	contextPanoramaPath,
	regionCompositionPath,
	resourceEdgesPath,
	teamsListPath,
	trailPath
} from './graph-reads';

describe('graph API path builders', () => {
	it('Beat D region composition (single + union)', () => {
		expect(regionCompositionPath(['r1'])).toBe('/api/graph/regions/composition?ids=r1&depth=1');
		expect(regionCompositionPath(['r1', 'r2'], 1)).toBe(
			'/api/graph/regions/composition?ids=r1,r2&depth=1'
		);
	});
	it('R5 element trail', () => {
		expect(trailPath('node', 'n1')).toBe('/api/graph/elements/node/n1/trail');
		expect(trailPath('edge', 'e1')).toBe('/api/graph/elements/edge/e1/trail');
	});
	it('teams list', () => {
		expect(teamsListPath()).toBe('/api/teams');
	});
	it('builds the resource edges path', () => {
		expect(resourceEdgesPath('019f420c-cf01-7bc1-87c9-09684b0fa69e')).toBe(
			'/api/resources/019f420c-cf01-7bc1-87c9-09684b0fa69e/edges'
		);
	});
	it('atlasHomePath', () => {
		expect(atlasHomePath()).toBe('/api/graph/home');
	});
	it('cogmapPanoramaPath', () => {
		expect(cogmapPanoramaPath('c1')).toBe('/api/graph/cogmaps/c1/panorama');
		expect(cogmapPanoramaPath('c1', 'l2')).toBe('/api/graph/cogmaps/c1/panorama?lens_id=l2');
	});
	it('builds the context panorama path, percent-encoding the ref', () => {
		expect(contextPanoramaPath('@me/temper', 'doc_type')).toBe(
			'/api/graph/contexts/panorama?context_ref=%40me%2Ftemper&group_by=doc_type'
		);
	});
	it('builds a container composition path', () => {
		expect(contextCompositionPath('@me/temper', { kind: 'container', id: 'abc' }, 1)).toBe(
			'/api/graph/contexts/composition?context_ref=%40me%2Ftemper&container=abc&depth=1'
		);
	});
	it('builds a bucket composition path, encoding the group value and forwarding container_depth', () => {
		expect(
			contextCompositionPath(
				'@me/temper',
				{ kind: 'bucket', groupKey: 'doc_type', value: 'session' },
				1
			)
		).toBe(
			'/api/graph/contexts/composition?context_ref=%40me%2Ftemper&group=doc_type%3Asession&depth=1&container_depth=2'
		);
	});
});
