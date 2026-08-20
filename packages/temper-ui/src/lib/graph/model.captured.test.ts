import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, test } from 'vitest';
import type { CogmapRegionRow } from '$lib/types/generated/cognitive_maps';
import type { QueryResponse } from '$lib/types/generated/query';
import type { GraphPlan } from './composition';
import { buildGraph } from './model';
import { buildReadout, describeGrouping, listGroupings } from './readout';

/**
 * The flagship entry, against a response the deployed substrate actually sent.
 *
 * Beat A shipped a composition builder with **69 green tests and zero callers**, and the first act
 * that POSTed its output at a real `/api/query` found three defects none of those tests could see.
 * So these assertions run against a captured wire response rather than a hand-built fixture: a
 * fixture cannot disagree with the shape its author imagined, and every number here is one that
 * was measured rather than assumed.
 *
 * The capture is TRIMMED and says so in its own `_trimmed` block — the survey arms are cut to
 * eight rows each because the untrimmed file is 3.9 MB. **The walk stage is whole**, which is what
 * matters: it carries every `via` entry, and the collapse is the thing under test.
 */
const fixture = JSON.parse(
	readFileSync(
		join(import.meta.dirname, '../../test/fixtures/graph-successor-flagship.json'),
		'utf8',
	),
) as {
	_trimmed: Record<string, unknown>;
	response: QueryResponse;
	shape_rows: CogmapRegionRow[];
};

const response = fixture.response;
const SURVEYS = Array.from({ length: 12 }, (_, i) => `s${i + 1}`);
const plan = {
	composition: { outcome: { returns: [] }, stages: [] },
	anchorsAsked: [],
	anchorsAvailable: 12,
	surveyStages: SURVEYS,
	walkStage: 'w',
} as unknown as GraphPlan;

describe('the composition the builder emits comes back answered', () => {
	test('all twelve surveys and the walk returned rows', () => {
		for (const s of [...SURVEYS, 'w']) {
			expect(response.returned?.[s], `stage ${s} is missing`).toBeDefined();
		}
	});

	test('every survey ran with the funnel width the builder named', () => {
		// The axis the client had to EARN: `applied_terms` defaults only `Limit`, so a survey
		// naming nothing runs at 3 and reports nothing at all.
		for (const s of SURVEYS) {
			expect(Number(response.returned?.[s]?.terms_applied?.regions)).toBe(3);
		}
	});

	test('only the walk can ever say complete or partial', () => {
		for (const s of SURVEYS) {
			expect(response.returned?.[s]?.extent.extent).toBe('indeterminate');
		}
		expect(response.returned?.w?.extent.extent).toBe('partial');
	});
});

describe('via collapses on the real walk', () => {
	const model = buildGraph({ response, plan, seeds: [] });

	test('1,973 entries become 101 edges — the measured 19.5x', () => {
		expect(model.viaEntries).toBe(1973);
		expect(model.edges).toHaveLength(101);
	});

	test('the worst-case degree is 25, not the 9 the acceptance criterion inherited', () => {
		// That 9 came from the PREDECESSOR's read of a different surface. This one's measured
		// worst case is 25 distinct edges on one node — 98 raw `via` entries before the collapse.
		const max = Math.max(...model.nodes.map((n) => n.degree));
		expect(max).toBe(25);
	});

	test('the walk stayed inside its published ceiling of 50', () => {
		expect(model.nodes.filter((n) => n.arm === 'walk').length).toBeLessThanOrEqual(50);
	});

	test('no edge is a stroke to a node that is not drawn', () => {
		const ids = new Set(model.nodes.map((n) => n.id));
		for (const e of model.edges) {
			expect(ids.has(e.source) && ids.has(e.target)).toBe(true);
		}
	});

	test('every edge names at least one seed it was reached from', () => {
		for (const e of model.edges) expect(e.seedIds.length).toBeGreaterThan(0);
	});
});

describe('the disclosed groupings resolve against the anchors’ shapes', () => {
	const readout = buildReadout(response, { rows: fixture.shape_rows, complete: true });

	test('nothing the response disclosed reads as re-derived', () => {
		// Measured on the full capture: 970 of 970 disclosed ids resolved, and 520 of those came
		// from CONTEXT anchors — so resolving only cogmaps would have rendered more than half the
		// panel as falsely re-derived, which is the exact false alarm the clause forbids.
		const missing = readout.groupings.filter((g) => g.name.state !== 'named');
		expect(missing).toEqual([]);
	});

	test('every resolved grouping carries the label its author wrote', () => {
		for (const g of readout.groupings) {
			expect(g.name.state === 'named' && g.name.label).toBeTruthy();
		}
	});

	test('no rendered grouping sentence names a score or a salience', () => {
		for (const g of readout.groupings) {
			const said = describeGrouping(g);
			expect(said).not.toMatch(/\d+\.\d+/);
		}
	});

	test('the listing is bounded and the remainder is declared', () => {
		const { shown, withheld } = listGroupings(readout);
		expect(shown.length + withheld).toBe(readout.groupings.length);
	});
});

describe('no derived thing reaches the canvas', () => {
	const model = buildGraph({ response, plan, seeds: [] });

	test('every node is a resource the reader owns a row for', () => {
		for (const n of model.nodes) {
			expect(n.resource.id).toBe(n.id);
			expect(typeof n.resource.title).toBe('string');
		}
	});

	test('no node carries a region id, a score, or a salience', () => {
		// Asserted on the node's FIELDS, not on its serialized text: titles are the reader's own
		// prose and legitimately contain words like "register" and "regions". A sweep over the
		// JSON blob reported a leak that was a reader's sentence — the derived-structure rule is
		// about what the surface attaches, never about what the reader wrote.
		for (const n of model.nodes) {
			const fields = Object.keys(n).filter((k) => k !== 'resource');
			expect(fields.sort()).toEqual([
				'arm',
				'degree',
				'doc_type',
				'excerpt',
				'home',
				'id',
				'title',
			]);
		}
	});
});
