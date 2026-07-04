// graph-reads.paths.test.ts
import { describe, expect, it } from 'vitest';
import {
	atlasHomePath,
	cogmapPanoramaPath,
	neighborhoodSlicePath,
	regionSlicePath,
	teamScopePath,
	teamsListPath,
	territoriesPath,
	trailPath
} from './graph-reads';

describe('graph API path builders', () => {
	it('R1 team scope', () => {
		expect(teamScopePath('t1')).toBe('/api/teams/t1/graph-scope');
	});
	it('R2 territories, optional lens', () => {
		expect(territoriesPath('t1', null)).toBe('/api/teams/t1/graph/territories');
		expect(territoriesPath('t1', 'lens9')).toBe('/api/teams/t1/graph/territories?lens_id=lens9');
	});
	it('R3 region slice', () => {
		expect(regionSlicePath('r5')).toBe('/api/graph/regions/r5/slice');
	});
	it('R4 neighborhood slice (POST target)', () => {
		expect(neighborhoodSlicePath('t1')).toBe('/api/teams/t1/graph/slice');
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
