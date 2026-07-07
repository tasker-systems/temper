// graph-reads.paths.test.ts
import { describe, expect, it } from 'vitest';
import {
	atlasHomePath,
	cogmapPanoramaPath,
	regionSlicePath,
	teamsListPath,
	trailPath
} from './graph-reads';

describe('graph API path builders', () => {
	it('R3 region slice', () => {
		expect(regionSlicePath('r5')).toBe('/api/graph/regions/r5/slice');
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
