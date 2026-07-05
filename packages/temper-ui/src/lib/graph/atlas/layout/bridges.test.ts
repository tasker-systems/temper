import { describe, it, expect } from 'vitest';
import { bridgeGeometry } from './bridges';
import type { Bridge } from '$lib/types/generated/graph_territory';

const bridges: Bridge[] = [
	{ source_territory: 'A', target_territory: 'B', edge_count: 5 },
	{ source_territory: 'A', target_territory: 'Z', edge_count: 2 } // Z has no position
];

describe('bridgeGeometry', () => {
	it('maps positioned territory pairs to line segments', () => {
		const pos = new Map([
			['A', { x: 0, y: 0 }],
			['B', { x: 10, y: 20 }]
		]);
		expect(bridgeGeometry(bridges, pos)).toEqual([{ x1: 0, y1: 0, x2: 10, y2: 20, edgeCount: 5 }]);
	});
	it('drops bridges whose endpoints are not both positioned', () => {
		const pos = new Map([['A', { x: 0, y: 0 }]]);
		expect(bridgeGeometry(bridges, pos)).toEqual([]);
	});
});
