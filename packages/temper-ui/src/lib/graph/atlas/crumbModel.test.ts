import { describe, it, expect } from 'vitest';
import { crumbModel } from './crumbModel';

describe('crumbModel', () => {
	it('home only, no cogmap, no focus → just Atlas', () => {
		const segs = crumbModel({
			cogmapName: null,
			focusPath: [],
			crumbTerritory: null,
			seedTitle: null,
			scopeFilter: null
		});
		expect(segs.map((s) => [s.kind, s.label])).toEqual([['home', '⌂ Atlas']]);
	});
	it('a committed scope filter (no cogmap) adds a scope segment after home', () => {
		const segs = crumbModel({
			cogmapName: null,
			focusPath: [],
			crumbTerritory: null,
			seedTitle: null,
			scopeFilter: '+tasker'
		});
		expect(segs.map((s) => [s.kind, s.label])).toEqual([
			['home', '⌂ Atlas'],
			['scope', '+tasker']
		]);
	});
	it('a scope filter is suppressed once a cogmap is entered', () => {
		const segs = crumbModel({
			cogmapName: 'Team self-model',
			focusPath: [],
			crumbTerritory: null,
			seedTitle: null,
			scopeFilter: '+tasker'
		});
		expect(segs.map((s) => s.kind)).toEqual(['home', 'cogmap']);
	});
	it('cogmap scope, no focus → Atlas / <cogmap name>', () => {
		const segs = crumbModel({
			cogmapName: 'Team self-model',
			focusPath: [],
			crumbTerritory: null,
			seedTitle: null,
			scopeFilter: null
		});
		expect(segs.map((s) => [s.kind, s.label])).toEqual([
			['home', '⌂ Atlas'],
			['cogmap', 'Team self-model']
		]);
	});
	it('territory focus adds the labeled territory hop', () => {
		const segs = crumbModel({
			cogmapName: 'Team self-model',
			focusPath: [{ kind: 'territory', id: 'R' }],
			crumbTerritory: { id: 'R', label: 'Runbooks' },
			seedTitle: null,
			scopeFilter: null
		});
		expect(segs.at(-1)).toEqual({ kind: 'territory', label: 'Runbooks', focusPath: 'territory:R' });
	});
	it('node reached via a territory shows both hops', () => {
		const segs = crumbModel({
			cogmapName: 'Team self-model',
			focusPath: [
				{ kind: 'territory', id: 'R' },
				{ kind: 'node', id: 'N' }
			],
			crumbTerritory: { id: 'R', label: 'Runbooks' },
			seedTitle: 'Deploy pipeline',
			scopeFilter: null
		});
		expect(segs.slice(-2).map((s) => [s.kind, s.label, s.focusPath])).toEqual([
			['territory', 'Runbooks', 'territory:R'],
			['node', 'Deploy pipeline', 'territory:R,node:N']
		]);
	});
	it('node drilled straight from panorama has no territory hop', () => {
		const segs = crumbModel({
			cogmapName: 'Team self-model',
			focusPath: [{ kind: 'node', id: 'N' }],
			crumbTerritory: null,
			seedTitle: 'Orphan doc',
			scopeFilter: null
		});
		expect(segs.map((s) => s.kind)).toEqual(['home', 'cogmap', 'node']);
		expect(segs.at(-1)).toEqual({ kind: 'node', label: 'Orphan doc', focusPath: 'node:N' });
	});
	it('territory label falls back to a generic when unresolved', () => {
		const segs = crumbModel({
			cogmapName: 'Team self-model',
			focusPath: [{ kind: 'territory', id: 'R' }],
			crumbTerritory: { id: 'R', label: null },
			seedTitle: null,
			scopeFilter: null
		});
		expect(segs.at(-1)?.label).toBe('Region');
	});
});
