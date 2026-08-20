import { describe, expect, test } from 'vitest';
import type { CogmapRow } from '$lib/types/generated/cognitive_maps';
import type { ContextRowWithCounts } from '$lib/types/generated/context';
import { questionFor, readableAnchors, resolveAnchors } from './entry';

const ctxRow = (slug: string, n: number, owner = '@me'): ContextRowWithCounts =>
	({
		id: `ctx-${slug}`,
		name: slug,
		slug,
		owner_ref: owner,
		resource_count: n,
	}) as unknown as ContextRowWithCounts;

const mapRow = (id: string, over: Partial<CogmapRow> = {}): CogmapRow =>
	({
		id,
		name: `map ${id}`,
		owner_ref: '+team',
		team_ids: [],
		region_count: 3,
		resource_count: 40,
		telos_resource_id: `telos-${id}`,
		charter_statement: `understand ${id}`,
		...over,
	}) as unknown as CogmapRow;

describe('readableAnchors', () => {
	test('a cogmap anchor is keyed by the same string the URL carries', () => {
		// `GraphAnchorRef` publishes "the uuid for a cogmap". If this diverged, a `map:` anchor in the
		// address bar could never match a readable row and every named cogmap door would refuse.
		const [anchor] = readableAnchors({ contexts: [], cogmaps: [mapRow('abc')] });

		expect(anchor).toEqual({ kind: 'cogmap', id: 'abc', ref: 'abc', resourceCount: 40 });
	});

	test('a context anchor is the whole decorated ref, owner included', () => {
		const [anchor] = readableAnchors({ contexts: [ctxRow('ops', 5, '+acme')], cogmaps: [] });

		expect(anchor.ref).toBe('+acme/ops');
	});

	test('a resource_count arriving as a JSON number is not mangled', () => {
		// The generated type says bigint; res.json() gives a number. Both must land as a number.
		const rows = readableAnchors({
			contexts: [ctxRow('a', 2066)],
			cogmaps: [mapRow('m', { resource_count: 817 })],
		});

		expect(rows.map((r) => r.resourceCount)).toEqual([2066, 817]);
	});
});

describe('resolveAnchors', () => {
	const readable = readableAnchors({
		contexts: [ctxRow('temper', 2066), ctxRow('ops', 5, '+acme')],
		cogmaps: [mapRow('m1')],
	});

	test('no `in` asks every readable anchor', () => {
		const r = resolveAnchors(readable, []);

		expect(r.entry).toBe('unaddressed');
		expect(r.entry === 'unaddressed' && r.anchors).toHaveLength(3);
	});

	test('a named anchor resolves across kinds', () => {
		const r = resolveAnchors(readable, [
			{ kind: 'context', ref: '@me/temper' },
			{ kind: 'cogmap', ref: 'm1' },
		]);

		expect(r.entry).toBe('named');
		expect(r.entry === 'named' && r.anchors.map((a) => a.ref)).toEqual(['@me/temper', 'm1']);
	});

	test('a kind mismatch does not resolve — `map:@me/temper` is not the context', () => {
		const r = resolveAnchors(readable, [{ kind: 'cogmap', ref: '@me/temper' }]);

		expect(r.entry).toBe('none-resolved');
	});

	test('a named place that does not resolve is DROPPED but still counted', () => {
		const r = resolveAnchors(readable, [
			{ kind: 'context', ref: '@me/temper' },
			{ kind: 'context', ref: '@me/deleted' },
		]);

		expect(r.entry).toBe('named');
		if (r.entry !== 'named') return;
		expect(r.anchors).toHaveLength(1);
		// The denominator is what was NAMED. "1 of 2 places" declares the drop; "1 of 1" hides it.
		expect(r.available).toBe(2);
	});

	test('naming only unresolvable places REFUSES rather than widening to everything', () => {
		// The dangerous case, and the reason this is a distinct outcome. An empty anchor list with a
		// question makes the builder emit `find-about-anywhere` — search everything I can see — so a
		// link naming one deleted context would answer across the whole corpus while the bound line
		// truthfully reported every place asked.
		const r = resolveAnchors(readable, [{ kind: 'context', ref: '@me/gone' }]);

		expect(r).toEqual({ entry: 'none-resolved', named: 1 });
	});
});

describe('questionFor', () => {
	const maps = [mapRow('m1'), mapRow('m2', { charter_statement: null })];
	const anchorFor = (id: string) => readableAnchors({ contexts: [], cogmaps: [mapRow(id)] });

	test("a reader's own question is used verbatim and borrows nothing", () => {
		expect(questionFor('why is this slow', anchorFor('m1'), maps)).toEqual({
			text: 'why is this slow',
			borrowedFrom: null,
		});
	});

	test('one cogmap and no question surveys under that map’s charter', () => {
		const q = questionFor(null, anchorFor('m1'), maps);

		expect(q.text).toBe('understand m1');
		expect(q.borrowedFrom).toEqual({ id: 'm1', name: 'map m1', telosResourceId: 'telos-m1' });
	});

	test('a map with no authored charter asks nothing, rather than asking an empty question', () => {
		expect(questionFor(null, anchorFor('m2'), maps).text).toBeNull();
	});

	test('a context never borrows a question — it has no declared telos to borrow', () => {
		const ctxAnchor = readableAnchors({ contexts: [ctxRow('temper', 10)], cogmaps: [] });

		expect(questionFor(null, ctxAnchor, maps).text).toBeNull();
	});

	test('two anchors borrow nothing, even when both are maps with charters', () => {
		// A charter is one map's purpose. Surveying a second map under the first's telos would be
		// asking a question its author never asked of it.
		const two = readableAnchors({ contexts: [], cogmaps: [mapRow('m1'), mapRow('m3')] });

		expect(questionFor(null, two, maps).text).toBeNull();
	});
});
