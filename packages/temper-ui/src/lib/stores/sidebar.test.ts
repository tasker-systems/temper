import { describe, expect, it } from 'vitest';
import { defaultCollapsed, parseCollapsedGroups, toggleCollapsedGroup } from './sidebar.svelte';

describe('defaultCollapsed', () => {
	it('collapses on graph routes', () => {
		expect(defaultCollapsed('/graph/@me')).toBe(true);
		expect(defaultCollapsed('/graph/@me?team=T')).toBe(true);
	});
	it('stays expanded elsewhere', () => {
		expect(defaultCollapsed('/vault/all')).toBe(false);
		expect(defaultCollapsed('/teams')).toBe(false);
	});
});

describe('parseCollapsedGroups', () => {
	it('reads nothing stored as nothing collapsed', () => {
		expect(parseCollapsedGroups(null)).toEqual([]);
	});
	it('reads a stored list back', () => {
		expect(parseCollapsedGroups('["+platform","@alice"]')).toEqual(['+platform', '@alice']);
	});
	it('falls open on a corrupt preference rather than hiding places', () => {
		expect(parseCollapsedGroups('{')).toEqual([]);
		expect(parseCollapsedGroups('"+platform"')).toEqual([]);
		expect(parseCollapsedGroups('[1,"+platform",null]')).toEqual(['+platform']);
	});
});

describe('toggleCollapsedGroup', () => {
	it('collapses one group without touching the others', () => {
		expect(toggleCollapsedGroup(['+platform'], '+research')).toEqual(['+platform', '+research']);
	});
	it('expands a collapsed group', () => {
		expect(toggleCollapsedGroup(['+platform', '+research'], '+platform')).toEqual(['+research']);
	});
	it('leaves the input untouched', () => {
		const before = ['+platform'];
		toggleCollapsedGroup(before, '+research');
		expect(before).toEqual(['+platform']);
	});
});
