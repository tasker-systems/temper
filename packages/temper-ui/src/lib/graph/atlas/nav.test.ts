// nav.test.ts
import { describe, expect, it } from 'vitest';
import {
	buildAscendUrl,
	buildDrillNodeUrl,
	buildDrillTerritoryUrl,
	buildScopeUrl,
	deriveTier,
	parseFocus,
	parseTeam
} from './nav';

const url = (qs: string) => new URL(`https://x/vault/@me/graph${qs}`);

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
		expect(buildScopeUrl(url('?team=old'), 'new').startsWith('/vault/@me/graph?')).toBe(true);
	});
});
