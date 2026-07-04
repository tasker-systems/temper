// nav.test.ts
import { describe, expect, it } from 'vitest';
import {
	buildAscendUrl,
	buildCogmapUrl,
	buildDrillNodeUrl,
	buildDrillTerritoryUrl,
	buildEdgeSelectUrl,
	buildFiltersUrl,
	buildHomeUrl,
	buildScopeUrl,
	clearSelectionUrl,
	deriveTier,
	parseCogmap,
	parseFilters,
	parseFocus,
	parseSelection,
	parseTeam,
	selectedElement
} from './nav';

const url = (qs: string) => new URL(`https://x/graph/@me${qs}`);

describe('parseFocus + deriveTier', () => {
	it('no focus param → none → tier 0', () => {
		const f = parseFocus(url('').searchParams);
		expect(f).toEqual({ kind: 'none' });
		expect(deriveTier(f)).toBe(0);
	});
	it('territory focus → tier 1', () => {
		const f = parseFocus(url('?focus=territory:abc').searchParams);
		expect(f).toEqual({ kind: 'territory', id: 'abc' });
		expect(deriveTier(f)).toBe(1);
	});
	it('node focus → tier 2', () => {
		const f = parseFocus(url('?focus=node:n1').searchParams);
		expect(f).toEqual({ kind: 'node', id: 'n1' });
		expect(deriveTier(f)).toBe(2);
	});
	it('malformed focus → none → tier 0', () => {
		expect(deriveTier(parseFocus(url('?focus=garbage').searchParams))).toBe(0);
	});
});

describe('parseTeam', () => {
	it('reads ?team, else null', () => {
		expect(parseTeam(url('?team=t1').searchParams)).toBe('t1');
		expect(parseTeam(url('').searchParams)).toBeNull();
	});
});

describe('URL builders', () => {
	it('buildScopeUrl sets team and CLEARS focus (re-scope resets to tier 0)', () => {
		const out = buildScopeUrl(url('?team=old&focus=node:n1'), 'new');
		const p = new URL(out, 'https://x').searchParams;
		expect(p.get('team')).toBe('new');
		expect(p.get('focus')).toBeNull();
	});
	it('buildDrillTerritoryUrl sets focus=territory:<id>, keeps team', () => {
		const out = buildDrillTerritoryUrl(url('?team=t1'), 'r9');
		const p = new URL(out, 'https://x').searchParams;
		expect(p.get('team')).toBe('t1');
		expect(p.get('focus')).toBe('territory:r9');
	});
	it('buildDrillNodeUrl sets focus=node:<id>', () => {
		const p = new URL(buildDrillNodeUrl(url('?team=t1'), 'n5'), 'https://x').searchParams;
		expect(p.get('focus')).toBe('node:n5');
	});
	it('buildAscendUrl clears focus', () => {
		const p = new URL(buildAscendUrl(url('?team=t1&focus=node:n5')), 'https://x').searchParams;
		expect(p.get('focus')).toBeNull();
		expect(p.get('team')).toBe('t1');
	});
	it('builders return path+query only (relative), preserving the graph pathname', () => {
		expect(buildScopeUrl(url('?team=old'), 'new').startsWith('/graph/@me?')).toBe(true);
	});
	it('buildHomeUrl clears BOTH team and focus (back to membership home)', () => {
		const p = new URL(buildHomeUrl(url('?team=t1&focus=node:n5')), 'https://x').searchParams;
		expect(p.get('team')).toBeNull();
		expect(p.get('focus')).toBeNull();
	});
});

describe('cogmap addressing', () => {
	it('parses ?cogmap=', () => {
		expect(parseCogmap(url('?cogmap=abc'))).toBe('abc');
		expect(parseCogmap(url(''))).toBeNull();
	});
	it('buildCogmapUrl sets cogmap and clears team+focus', () => {
		const out = buildCogmapUrl(url('?team=t1&focus=node:n1'), 'c9');
		expect(out).toContain('cogmap=c9');
		expect(out).not.toContain('team=');
		expect(out).not.toContain('focus=');
	});
	it('buildHomeUrl clears cogmap too', () => {
		expect(buildHomeUrl(url('?cogmap=c9'))).not.toContain('cogmap=');
	});
});

describe('edge selection (?sel)', () => {
	it('parses ?sel=edge:e1', () => {
		expect(parseSelection(url('?sel=edge:e1'))).toEqual({ kind: 'edge', id: 'e1' });
	});
	it('none when absent/malformed', () => {
		expect(parseSelection(url(''))).toEqual({ kind: 'none' });
		expect(parseSelection(url('?sel=node:n1'))).toEqual({ kind: 'none' }); // only edges use ?sel
	});
	it('buildEdgeSelectUrl sets ?sel, leaves ?focus/?team intact', () => {
		expect(buildEdgeSelectUrl(url('?team=t1&focus=node:n1'), 'e9')).toBe(
			'/graph/@me?team=t1&focus=node%3An1&sel=edge%3Ae9'
		);
	});
	it('clearSelectionUrl drops ?sel', () => {
		expect(clearSelectionUrl(url('?team=t1&sel=edge:e9'))).toBe('/graph/@me?team=t1');
	});
	it('selectedElement prefers edge sel, else focus node', () => {
		expect(selectedElement({ kind: 'node', id: 'n1' }, url('?sel=edge:e9'))).toEqual({
			kind: 'edge',
			id: 'e9'
		});
		expect(selectedElement({ kind: 'node', id: 'n1' }, url(''))).toEqual({ kind: 'node', id: 'n1' });
		expect(selectedElement({ kind: 'none' }, url(''))).toEqual({ kind: 'none' });
	});
});

describe('filters', () => {
	it('parses edge_kinds + doc_types CSV', () => {
		expect(parseFilters(url('?edge_kinds=derived,contains&doc_types=task,goal').searchParams)).toEqual({
			lensId: null,
			edgeKinds: ['derived', 'contains'],
			docTypes: ['task', 'goal']
		});
	});
	it('buildFiltersUrl sets/clears CSV params', () => {
		expect(buildFiltersUrl(url('?team=t1'), { edgeKinds: ['derived'] })).toBe(
			'/graph/@me?team=t1&edge_kinds=derived'
		);
		expect(buildFiltersUrl(url('?team=t1&edge_kinds=derived'), { edgeKinds: [] })).toBe('/graph/@me?team=t1');
	});
});
