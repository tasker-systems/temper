import { describe, expect, it } from 'vitest';
import { defaultFcoseConfig } from './layout';

describe('defaultFcoseConfig', () => {
	it('uses fcose as the layout name', () => {
		expect(defaultFcoseConfig().name).toBe('fcose');
	});

	it('requests proof-quality deterministic placement', () => {
		expect(defaultFcoseConfig().quality).toBe('proof');
	});

	it('pins the prototype-matching tuning values', () => {
		const c = defaultFcoseConfig();
		expect(c.idealEdgeLength).toBe(180);
		expect(c.nodeRepulsion).toBe(25_000);
		expect(c.gravity).toBe(0.15);
		expect(c.nodeSeparation).toBe(180);
		expect(c.numIter).toBe(3500);
	});

	it('returns fresh objects (safe to mutate)', () => {
		const a = defaultFcoseConfig();
		const b = defaultFcoseConfig();
		a.numIter = 10;
		expect(b.numIter).toBe(3500);
	});
});
