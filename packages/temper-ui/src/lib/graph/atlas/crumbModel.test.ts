import { describe, it, expect } from 'vitest';
import { crumbModel } from './crumbModel';
import type { TeamScopeView } from '$lib/types/generated/graph_scope';

const scope: TeamScopeView = {
	team: { id: 'T', slug: 't', name: 'Engineering' },
	ancestors: [{ id: 'A', slug: 'a', name: 'Acme' }],
	zones: []
};

describe('crumbModel', () => {
	it('team scope, no focus → Atlas / Acme / Engineering', () => {
		const segs = crumbModel({ scope, cogmapName: null, focusPath: [], crumbTerritory: null, seedTitle: null });
		expect(segs.map((s) => [s.kind, s.label])).toEqual([
			['home', '⌂ Atlas'],
			['ancestor', 'Acme'],
			['team', 'Engineering']
		]);
	});
	it('territory focus adds the labeled territory hop', () => {
		const segs = crumbModel({
			scope, cogmapName: null,
			focusPath: [{ kind: 'territory', id: 'R' }],
			crumbTerritory: { id: 'R', label: 'Runbooks' }, seedTitle: null
		});
		expect(segs.at(-1)).toEqual({ kind: 'territory', label: 'Runbooks', focusPath: 'territory:R' });
	});
	it('node reached via a territory shows both hops', () => {
		const segs = crumbModel({
			scope, cogmapName: null,
			focusPath: [{ kind: 'territory', id: 'R' }, { kind: 'node', id: 'N' }],
			crumbTerritory: { id: 'R', label: 'Runbooks' }, seedTitle: 'Deploy pipeline'
		});
		expect(segs.slice(-2).map((s) => [s.kind, s.label, s.focusPath])).toEqual([
			['territory', 'Runbooks', 'territory:R'],
			['node', 'Deploy pipeline', 'territory:R,node:N']
		]);
	});
	it('node drilled straight from panorama has no territory hop', () => {
		const segs = crumbModel({
			scope, cogmapName: null,
			focusPath: [{ kind: 'node', id: 'N' }],
			crumbTerritory: null, seedTitle: 'Orphan doc'
		});
		expect(segs.map((s) => s.kind)).toEqual(['home', 'ancestor', 'team', 'node']);
		expect(segs.at(-1)).toEqual({ kind: 'node', label: 'Orphan doc', focusPath: 'node:N' });
	});
	it('cogmap scope → Atlas / <cogmap name>', () => {
		const segs = crumbModel({ scope: null, cogmapName: 'Team self-model', focusPath: [], crumbTerritory: null, seedTitle: null });
		expect(segs.map((s) => [s.kind, s.label])).toEqual([['home', '⌂ Atlas'], ['cogmap', 'Team self-model']]);
	});
	it('territory label falls back to a generic when unresolved', () => {
		const segs = crumbModel({
			scope, cogmapName: null,
			focusPath: [{ kind: 'territory', id: 'R' }],
			crumbTerritory: { id: 'R', label: null }, seedTitle: null
		});
		expect(segs.at(-1)?.label).toBe('Region');
	});
});
