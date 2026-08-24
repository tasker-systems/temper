// fixtures.test.ts — guards the three committed graph fixture bundles.
//
// The harness renders the real `GraphPage` and `AnalysisPage` against these files (see
// `src/routes/dev/graph/README.md`). Three failure modes are locked down here:
//
//   1. **Shape drift** — a view type gains or loses a field but the fixtures do not, so the harness
//      renders a stale shape and only a human notices. The two key maps below are pinned to their
//      types via `satisfies Record<keyof …, true>`, so adding or removing a field on
//      `GraphViewData` / `AnalysisViewData` fails `bun run check` at compile time.
//   2. **Personal-data leak** — a raw capture is committed instead of the sanitized bundle. The
//      guard is POSITIVE: every free-text value must be built from the sanitizer's own word bank.
//      A denylist only catches strings someone thought to list, and you cannot enumerate real prose.
//   3. **Scenario loss** — a re-capture silently drops a shape, so a screen the harness claims to
//      offer stops existing.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { WORDS } from '../../../scripts/graph-synthetic-vocabulary.mjs';
import {
	type AnalysisBundle,
	analysisScenarioNames,
	analysisViewFor,
	type CompositionScenario,
	type HarnessBundle,
	scenarioNames,
	viewFor,
} from './harness';
import type { AnalysisViewData, GraphViewData } from './view';

const load = <T>(name: string): T =>
	JSON.parse(readFileSync(join(import.meta.dirname, '../../test/fixtures/', name), 'utf8')) as T;

const harness = load<HarnessBundle>('graph-harness.json');
const analysis = load<AnalysisBundle>('graph-analysis-anchors.json');
const flagship = load<Record<string, unknown>>('graph-successor-flagship.json');

// ── 1. Shape drift ──────────────────────────────────────────────────────────────────────────────

/**
 * Pinned to `GraphViewData`: if the type gains or loses a field this object stops satisfying
 * `Record<keyof GraphViewData, true>` and `bun run check` fails, forcing the harness adapter (and
 * this list) back into lockstep with the type.
 */
const GRAPH_VIEW_KEYS = {
	owner: true,
	question: true,
	borrowedFrom: true,
	refusal: true,
	model: true,
	bound: true,
	readout: true,
	tooLittleStructure: true,
	placesAsked: true,
	selected: true,
	selectedExcerpt: true,
	selectedTrail: true,
} satisfies Record<keyof GraphViewData, true>;

const ANALYSIS_VIEW_KEYS = {
	owner: true,
	place: true,
	alsoNamed: true,
	choices: true,
	refusal: true,
	regions: true,
	metricsAvailable: true,
	emptiness: true,
	map: true,
} satisfies Record<keyof AnalysisViewData, true>;

describe('the harness builds a whole view for every scenario', () => {
	const names = scenarioNames(harness);

	it('offers every scenario the capture tool emits', () => {
		for (const n of [
			'question',
			'mapCharter',
			'mapColdStart',
			'contextEverything',
			'contextZeroRegion',
			'entry',
			'entryZeroRegion',
			'entryTooLittleStructure',
			'traversal',
		]) {
			expect(names, `missing scenario "${n}"`).toContain(n);
		}
	});

	it.each(scenarioNames(harness))('scenario "%s" builds the full GraphViewData key set', (name) => {
		expect(Object.keys(viewFor(harness, name)).sort()).toEqual(Object.keys(GRAPH_VIEW_KEYS).sort());
	});

	it.each(
		analysisScenarioNames(analysis),
	)('analysis anchor "%s" builds the full AnalysisViewData key set', (name) => {
		expect(Object.keys(analysisViewFor(analysis, name)).sort()).toEqual(
			Object.keys(ANALYSIS_VIEW_KEYS).sort(),
		);
	});
});

// ── 2. The positive leak guard ──────────────────────────────────────────────────────────────────

const BANK = new Set(WORDS.map((w) => w.toLowerCase()));

/**
 * Every key whose value the sanitizer replaces. Kept beside the sanitizer's own list rather than
 * derived from it, because the two answer different questions — the sanitizer asks *what must I
 * replace*, and this asks *what must already have been replaced*. A key that falls off one list and
 * not the other is exactly the drift worth failing on.
 */
const SANITIZED_KEYS = new Set([
	'title',
	'excerpt',
	'cogmap_name',
	'context_name',
	'owner_handle',
	'context_slug',
	'slug',
]);

/** Is every word in this value drawn from the bank? */
const fromBank = (v: string): boolean =>
	v
		.toLowerCase()
		.split(/[^a-z0-9]+/)
		.filter(Boolean)
		.every((w) => BANK.has(w));

interface Leak {
	path: string;
	key: string;
	value: string;
}

function leaks(node: unknown, key = '', path: string[] = [], out: Leak[] = []): Leak[] {
	if (Array.isArray(node)) {
		for (const v of node) leaks(v, key, path, out);
	} else if (node && typeof node === 'object') {
		for (const [k, v] of Object.entries(node)) leaks(v, k, [...path, k], out);
	} else if (typeof node === 'string' && node) {
		// Our own annotations about the fixture are prose we wrote, and are not captured data.
		if (path.some((p) => p.startsWith('_'))) return out;
		const isRegionLabel = key === 'label' && !path.includes('via') && !path.includes('edges');
		if ((SANITIZED_KEYS.has(key) || isRegionLabel) && !fromBank(node)) {
			out.push({ path: path.join('>'), key, value: node });
		}
	}
	return out;
}

describe.each([
	['graph-harness.json', harness as unknown],
	['graph-analysis-anchors.json', analysis as unknown],
	['graph-successor-flagship.json', flagship as unknown],
])('%s carries no personal data', (_name, bundle) => {
	it('every sanitized free-text value is built from the synthetic word bank', () => {
		const found = leaks(bundle);
		expect(
			found.slice(0, 5),
			`${found.length} value(s) outside the word bank — was a raw capture committed?`,
		).toEqual([]);
	});

	it('carries the sanitizer provenance stamp', () => {
		expect((bundle as { _sanitized?: { synthetic?: boolean } })._sanitized?.synthetic).toBe(true);
	});
});

// ── 3. The shapes the bundle stands on ──────────────────────────────────────────────────────────

describe('the captured corpus shapes', () => {
	it('entry: the first screen, with an unconnected population worth judging', () => {
		// The trimmed flagship reads 19% degree-zero where the wire reads 51%, which is why this
		// bundle re-captures rather than reusing it. Asserted as a KIND — "there is a band, and it is
		// not everything" — not a magnitude, so a re-capture does not churn this.
		const model = viewFor(harness, 'entry');
		expect(model.placesAsked).toEqual([]);
		return model.model.then((m) => {
			const connected = new Set(m.edges.flatMap((e) => [e.source, e.target]));
			const unconnected = m.nodes.filter((n) => !connected.has(n.id)).length;
			expect(unconnected).toBeGreaterThan(0);
			expect(unconnected).toBeLessThan(m.nodes.length);
		});
	});

	it('entryTooLittleStructure is the ONLY scenario that reaches rung 2', async () => {
		for (const name of scenarioNames(harness)) {
			const verdict = await viewFor(harness, name).tooLittleStructure;
			if (name === 'entryTooLittleStructure') expect(verdict).not.toBeNull();
			else expect(verdict, `${name} reached rung 2`).toBeNull();
		}
	});

	it('both extents are represented — a partial answer and a complete one', async () => {
		const extents = await Promise.all(
			['question', 'contextEverything', 'mapColdStart', 'contextZeroRegion'].map(
				async (n) => (await viewFor(harness, n).bound)?.followedOn?.extent?.extent ?? null,
			),
		);
		expect(extents).toContain('partial');
		expect(extents).toContain('complete');
	});

	it('§2.3 on a zero-region context names no grouping and STILL draws a graph', async () => {
		// `the-unstructured-reader-is-never-worse-off`, in the form spec §6 claims it: 355 resources,
		// zero regions, and a real graph — because `find-resources-with → follow-from` touches no
		// region at all.
		const readout = await viewFor(harness, 'contextZeroRegion').readout;
		expect(readout?.groupings ?? []).toHaveLength(0);
		expect((await viewFor(harness, 'contextZeroRegion').model).nodes.length).toBeGreaterThan(0);
	});

	it('§2.2 on a zero-region MAP draws nothing — the asymmetry, pinned so a fix breaks it', async () => {
		// `[found — 2026-08-22, by this harness, before anyone looked at a screen]`
		//
		// Spec §6 asserts `the-unstructured-reader-is-never-worse-off` with: *"A reader with many
		// resources and zero regions still gets a graph — §2.3's path needs no region at all."* That
		// is true of §2.3 and **false of §2.2**, and the fixture is the evidence.
		//
		// A map with no question borrows its charter (`questionFor`), and a question routes through
		// `survey`, which is REGION-scored. On a map that has never materialized a region the survey
		// returns `disposition: 'empty'` with no hits, the union is empty, and `follow-from` seeds
		// from nothing — so the walk reports `complete` over zero rows. The reader who names their
		// least-organized map and asks nothing gets a blank canvas, and the bound line truthfully
		// says the walk completed.
		//
		// This is asserted, rather than skipped, so that **fixing it fails this test**. A test that
		// becomes a failure when the defect is repaired is doing its job; one that quietly tolerated
		// either answer would let the repair land unnoticed and let the regression land unnoticed too.
		// Anchored in the RESPONSE, not only in the model. A probe that broke the adapter's
		// composition path made the model-only assertion pass — so on its own it could not tell the
		// finding apart from the harness being broken, which is the one thing it must distinguish.
		const returned = (harness.mapColdStart as CompositionScenario).response.returned ?? {};
		expect(Object.keys(returned)).toHaveLength(2);
		for (const [stage, body] of Object.entries(returned)) {
			expect(body?.disposition, `stage ${stage} was not empty`).toBe('empty');
		}

		const cold = viewFor(harness, 'mapColdStart');
		expect((await cold.model).nodes).toHaveLength(0);
		// And the surface cannot fall back to rung 2 here: that verdict belongs to the entry read,
		// which is keyed on `eligible`, and a composition has no such axis.
		expect(await cold.tooLittleStructure).toBeNull();
	});

	/**
	 * **The authored scenarios are scaffolding, and this is what keeps them honest.**
	 *
	 * They exist because the capture beneath this bundle predates the envelope and production still
	 * answers a bare array, so no real read can supply an `emptiness` until this branch deploys.
	 * Two things have to stay true while they live here: they must cover every arm (or /dev/analysis
	 * silently stops showing one), and they must not have contaminated the capture.
	 */
	it('the authored scenarios cover every ShapeEmptiness arm', () => {
		const authored = analysisScenarioNames(analysis).filter((n) => n.startsWith('authored_'));
		const arms = new Set(
			authored.map((n) => (analysis[n] as { emptiness?: string }).emptiness ?? '(none)'),
		);

		expect([...arms].sort()).toEqual([
			'lens_narrowed',
			'never_clustered',
			'nothing_visible',
			'unreadable_or_absent',
		]);
		expect((analysis._authored as { authored?: boolean })?.authored, 'must be stamped').toBe(true);
	});

	/**
	 * The captured scenarios must carry NO `emptiness`. `cogmap_never_materialized` is the tempting
	 * one — its staleness proves it was never materialized, so `'never_clustered'` would even be the
	 * right value — and writing it in would state what a read said when the read did not say it.
	 */
	it('no captured scenario was given an emptiness the capture never observed', () => {
		for (const n of analysisScenarioNames(analysis).filter((k) => !k.startsWith('authored_'))) {
			expect(analysis[n], `captured scenario "${n}"`).not.toHaveProperty('emptiness');
		}
	});

	it('analysis carries an anchor that has never materialized a region', () => {
		const names = analysisScenarioNames(analysis);
		const empty = names.filter((n) => (analysis[n] as { shape: unknown[] }).shape.length === 0);
		expect(empty.length, 'no zero-region anchor in the analysis bundle').toBeGreaterThan(0);
	});
});
