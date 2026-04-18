import { describe, expect, it } from 'vitest';
import { buildForceConfig } from './force-config';

describe('buildForceConfig', () => {
	it('centers the layout on the viewport midpoint', () => {
		const cfg = buildForceConfig({ width: 800, height: 600 });
		expect(cfg.centerX).toBe(400);
		expect(cfg.centerY).toBe(300);
	});

	it('exposes charge/linkDistance/collisionPadding tunables', () => {
		const cfg = buildForceConfig({ width: 800, height: 600 });
		expect(typeof cfg.charge).toBe('number');
		expect(cfg.charge).toBeLessThan(0); // repulsion
		expect(cfg.linkDistance).toBeGreaterThan(0);
		expect(cfg.collisionPadding).toBeGreaterThanOrEqual(0);
	});

	it('produces consistent output for the same viewport', () => {
		const a = buildForceConfig({ width: 800, height: 600 });
		const b = buildForceConfig({ width: 800, height: 600 });
		expect(a).toEqual(b);
	});

	it('rescales center with different viewports', () => {
		const cfg = buildForceConfig({ width: 1200, height: 800 });
		expect(cfg.centerX).toBe(600);
		expect(cfg.centerY).toBe(400);
	});
});
