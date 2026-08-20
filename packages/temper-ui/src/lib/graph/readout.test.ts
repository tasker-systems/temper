import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, test } from 'vitest';
import type { QueryResponse, StageTrace } from '$lib/types/generated/query';
import { buildReadout, describeReadout } from './readout';

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

		expect(Object.keys(r.groupings[0])).toEqual(['id']);
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
