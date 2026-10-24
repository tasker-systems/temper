import { describe, expect, it } from 'vitest';
import {
	CAPTION,
	degrees,
	edgeGeometry,
	frame,
	nodeRadius,
	pairEnds,
	pairStroke,
	placeCaptions,
	settle,
	stroke,
	truncate,
	UNWEIGHTED_WIDTH,
	widthOf,
} from './graph';

const nodes = ['a', 'b', 'c', 'd', 'loner'].map((id) => ({ id, core: id === 'a' }));
const edges = [
	{ source: 'a', target: 'b' },
	{ source: 'a', target: 'c' },
	{ source: 'c', target: 'd' },
	{ source: 'a', target: 'ghost' },
	{ source: 'd', target: 'd' },
];

const first = <T>(xs: T[]): T => {
	const x = xs[0];
	if (x === undefined) throw new Error('expected at least one');
	return x;
};

/** A star of `n` leaves round `n0`, with `coreOf` deciding which nodes hold the core. */
const star = (n: number, coreOf: (i: number) => boolean) => ({
	nodes: Array.from({ length: n + 1 }, (_, i) => ({ id: `n${i}`, core: coreOf(i) })),
	edges: Array.from({ length: n }, (_, i) => ({ source: 'n0', target: `n${i + 1}` })),
});

describe('degree', () => {
	it('counts neighbours among carried nodes, never a self-join or a repeated pair', () => {
		const repeated = [...edges, { source: 'b', target: 'a' }, { source: 'a', target: 'b' }];
		expect(Object.fromEntries(degrees(nodes, repeated))).toEqual({
			a: 2,
			b: 1,
			c: 2,
			d: 1,
			loner: 0,
		});
	});

	it('sizes a mark on its neighbours, so two relations to one node do not enlarge it', () => {
		const g = settle(
			[{ id: 'a' }, { id: 'b' }],
			[
				{ source: 'a', target: 'b', label: 'leads_to' },
				{ source: 'a', target: 'b', label: 'contradicts' },
			],
			'force',
		);
		expect(g.nodes.map((n) => n.degree)).toEqual([1, 1]);
	});

	it('bounds the mark, and a busier node never reads as smaller', () => {
		expect(nodeRadius(0)).toBe(7);
		expect(nodeRadius(100)).toBe(16);
		const radii = [0, 1, 5, 9, 25, 98].map(nodeRadius);
		expect([...radii].sort((a, b) => a - b)).toEqual(radii);
	});
});

describe('settling', () => {
	it('places the connected nodes and lists the unconnected ones instead', () => {
		const g = settle(nodes, edges, 'force');
		expect(g.nodes.map((n) => n.id)).toEqual(['a', 'b', 'c', 'd']);
		expect(g.unconnected).toEqual(['loner']);
		expect(g.edges).toHaveLength(3);
		expect(g.selfJoined).toEqual([{ source: 'd', target: 'd' }]);
		for (const n of g.nodes) expect(Number.isFinite(n.x) && Number.isFinite(n.y)).toBe(true);
	});

	it('lists the unconnected in the order they came, never ranked', () => {
		const given = ['z', 'a', 'm', 'b'].map((id) => ({ id }));
		const g = settle(given, [{ source: 'a', target: 'b' }], 'force');
		expect(g.unconnected).toEqual(['z', 'm']);
		expect(g.nodes.length + g.unconnected.length).toBe(given.length);
	});

	it('lands in the same place every time', () => {
		const place = () => settle(nodes, edges, 'radial').nodes.map((n) => [n.x, n.y]);
		expect(place()).toEqual(place());
	});

	it('settles in the plane it is given', () => {
		const plane = { width: 1040, height: 620 };
		const g = settle(nodes, edges, 'force', plane);
		const cx = g.nodes.reduce((s, n) => s + n.x, 0) / g.nodes.length;
		const cy = g.nodes.reduce((s, n) => s + n.y, 0) / g.nodes.length;
		expect(Math.abs(cx - 520)).toBeLessThan(20);
		expect(Math.abs(cy - 310)).toBeLessThan(20);
	});

	it('pulls core nodes toward the centre when the input holds both core and ring', () => {
		const { nodes: many, edges: spokes } = star(11, (i) => i === 0);
		const g = settle(many, spokes, 'radial');
		expect(g.radial).toBe(true);
		const cx = g.nodes.reduce((s, n) => s + n.x, 0) / g.nodes.length;
		const cy = g.nodes.reduce((s, n) => s + n.y, 0) / g.nodes.length;
		const dist = (id: string) => {
			const n = g.nodes.find((m) => m.id === id);
			return n ? Math.hypot(n.x - cx, n.y - cy) : Number.NaN;
		};
		for (const n of many.slice(1)) expect(dist('n0')).toBeLessThan(dist(n.id));
	});

	it('settles as force when a radial layout is asked of an input with no contrast', () => {
		// Every node core — the web's entry read — or none: a radial force would pull them all to
		// one radius and pack the drawing into a disc.
		for (const coreOf of [() => true, () => false]) {
			const { nodes: many, edges: spokes } = star(11, coreOf);
			const asked = settle(many, spokes, 'radial');
			expect(asked.radial).toBe(false);
			expect(asked.nodes).toEqual(settle(many, spokes, 'force').nodes);
		}
	});

	it('keeps separate clusters near enough to read', () => {
		const pairs = Array.from({ length: 20 }, (_, i) => [`p${i}a`, `p${i}b`] as const);
		const g = settle(
			pairs.flat().map((id) => ({ id })),
			pairs.map(([source, target]) => ({ source, target })),
			'force',
		);
		const f = frame(g.nodes, []);
		expect(Math.max(f.x2 - f.x1, f.y2 - f.y1)).toBeLessThan(800);
	});
});

describe('one line per pair', () => {
	it('draws a repeated pair once, with every edge on it', () => {
		const g = settle(
			[{ id: 'a' }, { id: 'b' }],
			[
				{ source: 'a', target: 'b', edgeKind: 'leads_to' as const },
				{ source: 'b', target: 'a', label: 'contradicts', edgeKind: 'express' as const },
			],
			'force',
		);
		expect(g.edges).toHaveLength(1);
		const pair = first(g.edges);
		expect(pair.edges.map((e) => e.reversed)).toEqual([false, true]);
		expect(pairEnds(pair.edges)).toEqual({ atSource: true, atTarget: true });
		expect(pairStroke(pair.edges)).toMatchObject({ role: 'contradicts', cross: true });
		expect(pairStroke([{ edge: { label: 'derived_from' } }, { edge: {} }])).toEqual({
			role: 'structural',
			dash: null,
			width: UNWEIGHTED_WIDTH,
			cross: false,
		});
		expect(pairEnds([{ edge: { edgeKind: 'near' as const }, reversed: false }])).toEqual({
			atSource: false,
			atTarget: false,
		});
	});

	it('draws a pair of dashing edges dashed, and never thinner than its heaviest relation', () => {
		const dashed = pairStroke([
			{ edge: { edgeKind: 'leads_to' as const, weight: 0.5 } },
			{ edge: { edgeKind: 'near' as const } },
		]);
		expect(dashed.dash).toBe('7 4');
		expect(dashed.width).toBe(widthOf(0.5));
	});
});

describe('captions', () => {
	it('drops a caption that would collide rather than moving it', () => {
		const captions = placeCaptions([
			{ id: 'a', x: 0, y: 0, degree: 3, label: 'First' },
			{ id: 'b', x: 2, y: 1, degree: 1, label: 'Second' },
		]);
		expect(captions.map((c) => c.id)).toEqual(['a']);
	});

	it('breaks a tie in code-unit order, the same on every machine', () => {
		// `localeCompare` ranks 'a' before 'B'; code-unit order ranks 'B' (0x42) first. The two
		// captions collide, so whichever ranks first is the one drawn.
		const tie = [
			{ id: 'a', x: 0, y: 0, degree: 2, label: 'lower' },
			{ id: 'B', x: 3, y: 0, degree: 2, label: 'upper' },
		];
		expect(placeCaptions(tie).map((c) => c.id)).toEqual(['B']);
		expect(placeCaptions([...tie].reverse())).toEqual(placeCaptions(tie));
	});

	it('takes a surface its own room', () => {
		const apart = Array.from({ length: 30 }, (_, i) => ({
			id: `n${i}`,
			x: i * 400,
			y: 0,
			degree: 1,
			label: `node ${i}`,
		}));
		expect(placeCaptions(apart)).toHaveLength(CAPTION.max);
		expect(placeCaptions(apart, { max: 14 })).toHaveLength(14);
	});

	it('truncates by character, never splitting one', () => {
		expect(truncate('Short', 20)).toBe('Short');
		expect(truncate('A very long node title here', 10)).toBe('A very lo…');
		expect(truncate('ab😀cd', 4)).toBe('ab😀…');
	});

	it('frames every mark and caption it draws', () => {
		const g = settle(nodes, edges, 'force');
		const f = frame(g.nodes, []);
		for (const n of g.nodes) {
			expect(n.x).toBeGreaterThan(f.x1);
			expect(n.x).toBeLessThan(f.x2);
		}
	});
});

describe('edge geometry', () => {
	const s = { id: 's', degree: 0, x: 0, y: 0 };
	const t = { id: 't', degree: 0, x: 100, y: 0 };

	it('ends an edge at the rims, so a head is never under the mark it points at', () => {
		const fwd = edgeGeometry(s, t, { atSource: false, atTarget: true });
		expect(fwd.x1).toBeGreaterThan(nodeRadius(0));
		expect(fwd.x2).toBeLessThan(100 - nodeRadius(0));
		expect(fwd.heads).toHaveLength(1);
		expect(first(fwd.heads).startsWith(`M${fwd.x2.toFixed(2)},`)).toBe(true);
		expect(edgeGeometry(s, t, { atSource: false, atTarget: false }).heads).toHaveLength(0);
	});

	it('ends where a surface says a mark that is not a circle ends', () => {
		const g = edgeGeometry(s, t, { atSource: false, atTarget: true }, { source: 10, target: 20 });
		expect([g.x1, g.x2]).toEqual([10, 80]);
	});
});

describe('the edge grammar', () => {
	it('reads its stroke from the relation it carries', () => {
		// A `derived_from` label colours and dashes first, whatever the kind says.
		expect(stroke({ label: 'derived_from', edgeKind: 'express' })).toMatchObject({
			role: 'derived',
			dash: '7 4',
		});
		expect(stroke({ label: 'contradicts' })).toMatchObject({ role: 'contradicts', cross: true });
		expect(stroke({}).cross).toBe(false);
		// Otherwise the dash is the kind's.
		expect(stroke({}).dash).toBeNull();
		expect(stroke({ edgeKind: 'leads_to' }).dash).toBe('7 4');
		expect(stroke({ edgeKind: 'express' }).dash).toBe('1 4');
		expect(stroke({ edgeKind: 'near' }).dash).toBe('4 4');
		// Arrowheads follow polarity; a `near` relation heads neither end.
		expect(stroke({})).toMatchObject({ atSource: false, atTarget: true });
		expect(stroke({ polarity: 'inverse' })).toMatchObject({ atSource: true, atTarget: false });
		expect(stroke({ edgeKind: 'near' })).toMatchObject({ atSource: false, atTarget: false });
	});

	it('spreads weights across the range they take, and a default weight does not dominate', () => {
		// Weights are asserted at 1.0 and only lowered by decay: (0, 1].
		expect(stroke({ weight: 0.2 }).width).toBeLessThan(stroke({ weight: 1 }).width);
		expect(stroke({ weight: 1 }).width).toBe(2.5);
		expect(stroke({ weight: 0 }).width).toBe(1);
		expect(stroke({ weight: 99 }).width).toBe(2.5);
		expect(stroke({ weight: -3 }).width).toBe(1);
		// Absent, the unweighted width is stated, not defaulted to a weight.
		expect(stroke({}).width).toBe(UNWEIGHTED_WIDTH);
		expect(stroke({ weight: null }).width).toBe(UNWEIGHTED_WIDTH);
	});
});
