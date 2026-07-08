// graph-reads.paths.test.ts
import { describe, expect, it } from 'vitest';
import {
	atlasHomePath,
	cogmapPanoramaPath,
	regionCompositionPath,
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
	it('atlasHomePath', () => {
		expect(atlasHomePath()).toBe('/api/graph/home');
	});
	it('cogmapPanoramaPath', () => {
		expect(cogmapPanoramaPath('c1')).toBe('/api/graph/cogmaps/c1/panorama');
		expect(cogmapPanoramaPath('c1', 'l2')).toBe('/api/graph/cogmaps/c1/panorama?lens_id=l2');
	});
});
