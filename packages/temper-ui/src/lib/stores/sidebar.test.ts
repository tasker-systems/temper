import { describe, it, expect } from 'vitest';
import { defaultCollapsed } from './sidebar.svelte';

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
