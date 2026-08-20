import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, test } from 'vitest';
import type { CogmapRegionRow } from '$lib/types/generated/cognitive_maps';
import type { QueryResponse, StageTrace } from '$lib/types/generated/query';
import {
	buildReadout,
	describeGrouping,
	describeReadout,
	describeWithheld,
	disclosedRegionIds,
	GROUPINGS_LISTED,
	listGroupings,
} from './readout';

/** One row as `anchor_shape_select` answers, for either anchor kind. */
const region = (o: {
	id: string;
	label?: string | null;
	members?: number;
	salience?: number;
}): CogmapRegionRow =>
	({
		region_id: o.id,
		lens_id: 'lens',
		salience: o.salience ?? 1,
		content_cohesion: null,
		label: o.label ?? null,
		member_count: o.members ?? 0,
	}) as unknown as CogmapRegionRow;

const trace = (opts: {
	stage: string;
	act: string;
	groupings?: { id: string; score: number }[];
	inputIds?: number;
	unusable?: number;
}): StageTrace =>
	({
		stage: opts.stage,
		act: opts.act,
		disposition: 'answered',
		refusal: null,
		inputs: [],
		extent: { extent: 'complete' },
		terms_applied: {},
		narrowed_by: [],
		disclosed_regions: (opts.groupings ?? []).map((g) => ({
			region_id: g.id,
			region_score: g.score,
		})),
		input_ids: BigInt(opts.inputIds ?? 0),
		input_unusable: BigInt(opts.unusable ?? 0),
		produced_ids: 0n,
	}) as unknown as StageTrace;

const response = (stages: StageTrace[]): QueryResponse =>
	({ returned: {}, trace: { stages } }) as unknown as QueryResponse;

describe('the readout reads the disclosure the response actually carries', () => {
	test('collects the groupings each stage disclosed', () => {
		const r = buildReadout(
			response([
				trace({
					stage: 's1',
					act: 'survey',
					groupings: [
						{ id: 'g-1', score: 0.9 },
						{ id: 'g-2', score: 0.4 },
					],
				}),
				trace({ stage: 'w', act: 'follow-from' }),
			]),
		);

		expect(r.groupings.map((g) => g.id)).toEqual(['g-1', 'g-2']);
	});

	test('keeps the order the response disclosed them in', () => {
		const r = buildReadout(
			response([
				trace({
					stage: 's1',
					act: 'survey',
					groupings: [
						{ id: 'low', score: 0.1 },
						{ id: 'high', score: 0.99 },
					],
				}),
			]),
		);

		expect(r.groupings.map((g) => g.id)).toEqual(['low', 'high']);
	});

	test('spans every stage that disclosed one', () => {
		const r = buildReadout(
			response([
				trace({ stage: 's1', act: 'survey', groupings: [{ id: 'a', score: 1 }] }),
				trace({ stage: 's2', act: 'survey', groupings: [{ id: 'b', score: 1 }] }),
			]),
		);

		expect(r.groupings).toHaveLength(2);
	});

	test('accounts for every stage, including the ones whose rows were not returned', () => {
		const r = buildReadout(
			response([
				trace({ stage: 'm1', act: 'find-resources-with', inputIds: 1 }),
				trace({ stage: 'w', act: 'follow-from', inputIds: 240, unusable: 28 }),
			]),
		);

		expect(r.stages).toHaveLength(2);
		expect(r.stages[1]).toMatchObject({ stage: 'w', handed: 240, unusable: 28 });
	});

	test('carries no disclosure when the response carried none', () => {
		const r = buildReadout(response([trace({ stage: 'w', act: 'follow-from' })]));

		expect(r.groupings).toEqual([]);
	});
});

describe('a score is never presented to the reader', () => {
	test('the grouping carries no score field at all', () => {
		const r = buildReadout(
			response([trace({ stage: 's1', act: 'survey', groupings: [{ id: 'g', score: 0.87 }] })]),
		);

		expect(Object.keys(r.groupings[0]).sort()).toEqual(['id', 'name']);
		expect(JSON.stringify(r.groupings[0])).not.toContain('0.87');
	});

	test('a resolved grouping carries its member count but never its salience', () => {
		// `salience` is measured at 69.5 on `@me/temper` — not a fraction, not a score, and
		// nothing on this surface may imply it is either. `member_count` is safe where it is
		// not: it counts the reader's own readable material.
		const r = buildReadout(
			response([trace({ stage: 's1', act: 'survey', groupings: [{ id: 'g', score: 0.87 }] })]),
			{
				rows: [region({ id: 'g', label: 'A grouping', members: 12, salience: 69.53 })],
				complete: true,
			},
		);

		expect(r.groupings[0].name).toEqual({ state: 'named', label: 'A grouping', memberCount: 12 });
		expect(JSON.stringify(r.groupings[0])).not.toContain('69');
		expect(describeGrouping(r.groupings[0])).not.toContain('69');
	});

	test('no rendered string carries the number', () => {
		const r = buildReadout(
			response([trace({ stage: 's1', act: 'survey', groupings: [{ id: 'g', score: 0.87 }] })]),
		);

		expect(describeReadout(r)).not.toContain('0.87');
		expect(describeReadout(r)).not.toContain('87');
	});
});

describe('no internal vocabulary is load-bearing', () => {
	const forbidden = ['region', 'salience', 'wayfind', 'survey'];

	const everyDescription = () =>
		[
			buildReadout(response([])),
			buildReadout(response([trace({ stage: 'w', act: 'follow-from' })])),
			buildReadout(
				response([trace({ stage: 's1', act: 'survey', groupings: [{ id: 'g', score: 1 }] })]),
			),
			buildReadout(
				response([
					trace({
						stage: 's1',
						act: 'survey',
						groupings: [
							{ id: 'a', score: 1 },
							{ id: 'b', score: 0.5 },
							{ id: 'c', score: 0.2 },
						],
					}),
				]),
			),
		].map(describeReadout);

	test.each(forbidden)('no description contains %s', (word) => {
		for (const d of everyDescription()) expect(d.toLowerCase()).not.toContain(word);
	});

	test('the description says how many groupings the answer came from', () => {
		const r = buildReadout(
			response([
				trace({
					stage: 's1',
					act: 'survey',
					groupings: [
						{ id: 'a', score: 1 },
						{ id: 'b', score: 1 },
						{ id: 'c', score: 1 },
					],
				}),
			]),
		);

		expect(describeReadout(r)).toBe('These came from 3 groupings of your work.');
	});

	test('one grouping reads as one, not as 1 groupings', () => {
		const r = buildReadout(
			response([trace({ stage: 's1', act: 'survey', groupings: [{ id: 'a', score: 1 }] })]),
		);

		expect(describeReadout(r)).toBe('These came from 1 grouping of your work.');
	});

	test('no grouping at all says so, rather than saying zero', () => {
		expect(describeReadout(buildReadout(response([])))).toBe(
			'These were not drawn from any grouping of your work.',
		);
	});
});

describe('derived structure is confined to the readout — the type-level half', () => {
	const dir = join(import.meta.dirname, '.');
	const modules = readdirSync(dir).filter((f) => f.endsWith('.ts') && !f.endsWith('.test.ts'));

	test('there is more than one module here, so this sweep is not vacuous', () => {
		expect(modules.length).toBeGreaterThan(1);
		expect(modules).toContain('readout.ts');
	});

	test.each([
		'RegionHit',
		'RegionDisclosure',
		'disclosed_regions',
		'region_score',
		'region_id',
	])('only the readout names %s', (symbol) => {
		for (const file of modules) {
			if (file === 'readout.ts') continue;
			const source = readFileSync(join(dir, file), 'utf8');
			// Comments may discuss the constraint; code may not reach for the thing.
			const code = source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
			expect(code, `${file} reaches for ${symbol}`).not.toContain(symbol);
		}
	});
});

describe('a stale grouping reference is re-derived — never an error, never the reader’s mistake', () => {
	const disclosed = (id: string) =>
		response([trace({ stage: 's1', act: 'survey', groupings: [{ id, score: 1 }] })]);

	test('an id the lookup found is named by the label its author wrote', () => {
		const r = buildReadout(disclosed('g'), {
			rows: [region({ id: 'g', label: 'Bounds and their disclosure', members: 4 })],
			complete: true,
		});

		expect(describeGrouping(r.groupings[0])).toBe('Bounds and their disclosure · 4 resources');
	});

	test('an id a COMPLETE lookup did not find reads as re-derived', () => {
		const r = buildReadout(disclosed('minted-since'), {
			rows: [region({ id: 'some-other', label: 'Still here' })],
			complete: true,
		});

		expect(r.groupings[0].name).toEqual({ state: 're-derived' });
		expect(describeGrouping(r.groupings[0])).toBe('This grouping has been re-derived.');
	});

	test('re-derived never reads as an error or as something the reader did', () => {
		const r = buildReadout(disclosed('gone'), { rows: [], complete: true });
		const said = describeGrouping(r.groupings[0]).toLowerCase();

		for (const blame of ['error', 'invalid', 'not found', 'failed', 'you ', 'your']) {
			expect(said).not.toContain(blame);
		}
	});

	test('an INCOMPLETE lookup never claims re-derived — it has not got the evidence', () => {
		// The distinction this whole union exists for. One shape read that did not answer must
		// not turn every unfound id into a claim that the grouping is gone.
		const r = buildReadout(disclosed('unknown'), { rows: [], complete: false });

		expect(r.groupings[0].name).toEqual({ state: 'unchecked' });
		expect(describeGrouping(r.groupings[0])).not.toContain('re-derived');
	});

	test('a resolved grouping with no authored label is described, never machine-named', () => {
		const id = '019f5733-8edb-7571-9583-ab1bcd3ab86d';
		const r = buildReadout(disclosed(id), {
			rows: [region({ id, label: null, members: 7 })],
			complete: true,
		});

		expect(describeGrouping(r.groupings[0])).toBe('An unnamed grouping · 7 resources');
		// The id is machine identity and never becomes a name the reader is shown.
		expect(describeGrouping(r.groupings[0])).not.toContain(id);
	});

	test('the ids the route must resolve are every disclosed one, deduplicated in order', () => {
		const r = response([
			trace({
				stage: 's1',
				act: 'survey',
				groupings: [
					{ id: 'a', score: 1 },
					{ id: 'b', score: 1 },
				],
			}),
			trace({ stage: 's2', act: 'survey', groupings: [{ id: 'a', score: 1 }] }),
		]);

		expect(disclosedRegionIds(r)).toEqual(['a', 'b']);
	});

	test('nothing disclosed asks for no lookup at all', () => {
		expect(disclosedRegionIds(response([trace({ stage: 'w', act: 'follow-from' })]))).toEqual([]);
	});
});

describe('no internal vocabulary is load-bearing — the per-grouping strings', () => {
	// The surface's OWN words only. An authored label is the reader's material and may say
	// anything; these fixtures deliberately carry none of the forbidden words so the sweep
	// tests the sentence the surface builds rather than the name the reader gave.
	test.each([
		'region',
		'salience',
		'wayfind',
		'survey',
	])('no grouping sentence contains %s', (word) => {
		const said = [
			{
				id: 'a',
				name: { state: 'named' as const, label: 'Bounds and disclosure', memberCount: 4 },
			},
			{ id: 'b', name: { state: 'named' as const, label: null, memberCount: 1 } },
			{ id: 'c', name: { state: 're-derived' as const } },
			{ id: 'd', name: { state: 'unchecked' as const } },
		].map(describeGrouping);

		for (const s of said) expect(s.toLowerCase()).not.toContain(word);
	});
});

describe('the listing is bounded, and says how much it is not listing', () => {
	const many = (n: number) =>
		response([
			trace({
				stage: 's1',
				act: 'survey',
				groupings: Array.from({ length: n }, (_, i) => ({ id: `g${i}`, score: 1 })),
			}),
		]);

	test('a short listing withholds nothing', () => {
		const { shown, withheld } = listGroupings(buildReadout(many(4)));

		expect(shown).toHaveLength(4);
		expect(withheld).toBe(0);
	});

	test('the measured worst case lists a bounded set and declares the remainder', () => {
		// 970 disclosed, measured against the deployed substrate on 2026-08-20.
		const r = buildReadout(many(970));
		const { shown, withheld } = listGroupings(r);

		expect(shown).toHaveLength(GROUPINGS_LISTED);
		expect(withheld).toBe(970 - GROUPINGS_LISTED);
		// The two halves must agree: the count sentence states the TRUE total, so the listing
		// cannot quietly imply the total is what it showed.
		expect(describeReadout(r)).toContain('970');
		expect(describeWithheld(withheld)).toBe('958 more groupings are not listed here.');
	});

	test('exactly one withheld reads as one, not as 1 groupings', () => {
		expect(describeWithheld(1)).toBe('1 more grouping is not listed here.');
	});

	test('the listing keeps the order the response disclosed', () => {
		const { shown } = listGroupings(buildReadout(many(30)), 3);

		expect(shown.map((g) => g.id)).toEqual(['g0', 'g1', 'g2']);
	});
});
