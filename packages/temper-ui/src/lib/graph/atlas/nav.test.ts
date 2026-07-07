// nav.test.ts
import { describe, expect, it } from 'vitest';
import {
	activeFilterCount,
	buildAscendUrl,
	buildCogmapUrl,
	buildDrillNodeUrl,
	buildDrillTerritoryUrl,
	buildEdgeSelectUrl,
	buildFiltersUrl,
	buildHomeLensUrl,
	buildHomeUrl,
	clearHomeLensUrl,
	buildPanoramaUrl,
	clearSelectionUrl,
	deriveTier,
	parseCogmap,
	parseFilters,
	parseFocus,
	parseFocusPath,
	parseHomeLens,
	parseSelection,
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

describe('URL builders', () => {
	it('buildDrillTerritoryUrl sets focus=territory:<id>, keeps other params', () => {
		const out = buildDrillTerritoryUrl(url('?lens_id=L1'), 'r9');
		const p = new URL(out, 'https://x').searchParams;
		expect(p.get('lens_id')).toBe('L1');
		expect(p.get('focus')).toBe('territory:r9');
	});
	it('buildDrillNodeUrl sets focus=node:<id>', () => {
		const p = new URL(buildDrillNodeUrl(url(''), 'n5'), 'https://x').searchParams;
		expect(p.get('focus')).toBe('node:n5');
	});
	it('buildAscendUrl clears focus, keeps other params', () => {
		const p = new URL(buildAscendUrl(url('?lens_id=L1&focus=node:n5')), 'https://x').searchParams;
		expect(p.get('focus')).toBeNull();
		expect(p.get('lens_id')).toBe('L1');
	});
	it('builders return path+query only (relative), preserving the graph pathname', () => {
		expect(buildCogmapUrl(url('?cogmap=old'), 'new').startsWith('/graph/@me?')).toBe(true);
	});
	it('buildHomeUrl clears focus (back to membership home)', () => {
		const p = new URL(buildHomeUrl(url('?focus=node:n5')), 'https://x').searchParams;
		expect(p.get('focus')).toBeNull();
	});
});

describe('cogmap addressing', () => {
	it('parses ?cogmap=', () => {
		expect(parseCogmap(url('?cogmap=abc'))).toBe('abc');
		expect(parseCogmap(url(''))).toBeNull();
	});
	it('buildCogmapUrl sets cogmap and clears focus', () => {
		const out = buildCogmapUrl(url('?focus=node:n1'), 'c9');
		expect(out).toContain('cogmap=c9');
		expect(out).not.toContain('focus=');
	});
	it('buildHomeUrl clears cogmap too', () => {
		expect(buildHomeUrl(url('?cogmap=c9'))).not.toContain('cogmap=');
	});
	it('buildPanoramaUrl clears focus + sel but KEEPS cogmap scope + filters', () => {
		const out = buildPanoramaUrl(url('?cogmap=c9&focus=territory:r9&sel=edge:e2&lens_id=L1'));
		const p = new URL(out, 'https://x').searchParams;
		expect(p.get('focus')).toBeNull();
		expect(p.get('sel')).toBeNull();
		expect(p.get('cogmap')).toBe('c9');
		expect(p.get('lens_id')).toBe('L1');
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
	it('buildEdgeSelectUrl sets ?sel, leaves ?focus intact', () => {
		expect(buildEdgeSelectUrl(url('?focus=node:n1'), 'e9')).toBe(
			'/graph/@me?focus=node%3An1&sel=edge%3Ae9'
		);
	});
	it('clearSelectionUrl drops ?sel', () => {
		expect(clearSelectionUrl(url('?lens_id=L1&sel=edge:e9'))).toBe('/graph/@me?lens_id=L1');
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
		expect(buildFiltersUrl(url('?cogmap=c1'), { edgeKinds: ['derived'] })).toBe(
			'/graph/@me?cogmap=c1&edge_kinds=derived'
		);
		expect(buildFiltersUrl(url('?cogmap=c1&edge_kinds=derived'), { edgeKinds: [] })).toBe('/graph/@me?cogmap=c1');
	});
});

describe('activeFilterCount', () => {
	it('is 0 with no filters', () => expect(activeFilterCount(url(''))).toBe(0));
	it('counts each active dimension', () =>
		expect(activeFilterCount(url('?lens_id=L&edge_kinds=contains,near&doc_types=note'))).toBe(3));
	it('empty CSV params do not count', () => expect(activeFilterCount(url('?edge_kinds='))).toBe(0));
});

describe('focus-as-path', () => {
	it('parses an empty path', () => {
		expect(parseFocusPath(url(''))).toEqual([]);
	});
	it('parses a territory→node path', () => {
		expect(parseFocusPath(url('?focus=territory:R,node:N'))).toEqual([
			{ kind: 'territory', id: 'R' },
			{ kind: 'node', id: 'N' }
		]);
	});
	it('parseFocus returns the leaf segment', () => {
		expect(parseFocus(url('?focus=territory:R,node:N').searchParams)).toEqual({ kind: 'node', id: 'N' });
		expect(parseFocus(url('?focus=territory:R').searchParams)).toEqual({ kind: 'territory', id: 'R' });
	});
	it('drillNode appends when a territory leaf is present', () => {
		expect(buildDrillNodeUrl(url('?focus=territory:R'), 'N')).toBe(
			'/graph/@me?focus=territory%3AR%2Cnode%3AN'
		);
	});
	it('drillNode sets directly when drilled from panorama', () => {
		expect(buildDrillNodeUrl(url(''), 'N')).toBe('/graph/@me?focus=node%3AN');
	});
	it('drillNode replaces a trailing node leaf while KEEPING a territory prefix', () => {
		expect(buildDrillNodeUrl(url('?focus=territory:R,node:N'), 'N2')).toBe(
			'/graph/@me?focus=territory%3AR%2Cnode%3AN2'
		);
	});
	it('drillNode replaces a bare node leaf with no prefix', () => {
		expect(buildDrillNodeUrl(url('?focus=node:N'), 'N2')).toBe(
			'/graph/@me?focus=node%3AN2'
		);
	});
	it('drillTerritory sets the first hop', () => {
		expect(buildDrillTerritoryUrl(url(''), 'R')).toBe('/graph/@me?focus=territory%3AR');
	});
	it('ascend pops one segment', () => {
		expect(buildAscendUrl(url('?focus=territory:R,node:N'))).toBe(
			'/graph/@me?focus=territory%3AR'
		);
		expect(buildAscendUrl(url('?focus=territory:R'))).toBe('/graph/@me');
	});
});

describe('home lens (?home)', () => {
	const u = (s: string) => new URL(`https://x.test/graph/@me${s}`);
	it('parseHomeLens: absent → null, valid → value, garbage → null', () => {
		expect(parseHomeLens(u(''))).toBeNull();
		expect(parseHomeLens(u('?home=build'))).toBe('build');
		expect(parseHomeLens(u('?home=research'))).toBe('research');
		expect(parseHomeLens(u('?home=nope'))).toBeNull();
	});
	it('buildHomeLensUrl sets ?home preserving path; clear removes it', () => {
		expect(buildHomeLensUrl(u(''), 'build')).toBe('/graph/@me?home=build');
		expect(clearHomeLensUrl(u('?home=research'))).toBe('/graph/@me');
	});
});
