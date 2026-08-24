import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import { type AnalysedRegion, analyseShape, METRICS } from '$lib/graph/analysis';
import type { AnalysisViewData } from '$lib/graph/view';
import { describeFailure, GaveUp } from '$lib/server/bounded';
import type {
	CogmapAnalyticsRow,
	CogmapRegionMetricsRow,
	CogmapRegionRow,
	ShapeEmptiness,
} from '$lib/types/generated/cognitive_maps';
import { sentenceOf } from '../../../test/sentence';
import AnalysisPage from './AnalysisPage.svelte';

/**
 * The receiver, rendered against the shapes the deployed substrate actually sends.
 *
 * This page exists because `displaced-structure-remains-reachable` requires it: Beat B took the
 * per-region measurements off the navigational canvas, and the clause is only sound if what was
 * displaced is *"available somewhere that declares itself as analysis rather than as the reader's
 * material."* So the two things worth pinning here are that the declaration is unconditional, and
 * that the displaced payload actually arrives.
 *
 * Everything below runs on the untrimmed capture — 907 groupings across a context and a cogmap.
 */
const fixture = JSON.parse(
	readFileSync(
		join(import.meta.dirname, '../../../test/fixtures/graph-analysis-anchors.json'),
		'utf8',
	),
) as {
	context: { shape: CogmapRegionRow[]; region_metrics: CogmapRegionMetricsRow[] };
	cogmap: {
		name: string;
		shape: CogmapRegionRow[];
		region_metrics: CogmapRegionMetricsRow[];
		analytics: CogmapAnalyticsRow;
	};
};

/**
 * Overrides in their **settled** form, for the three fields the load now streams.
 *
 * A test that only cares about content keeps writing `regions: analyseShape(...).regions` or
 * `map: null`, and this builder wraps it; a test that cares about the *state* of the read passes a
 * promise through untouched — never-settling for arriving, rejected for failed. That is what keeps
 * C1 and C3 expressible here without every call site learning what a promise is.
 */
type AnalysisMap = Awaited<AnalysisViewData['map']>;

type ViewOverrides = Partial<
	Omit<AnalysisViewData, 'regions' | 'metricsAvailable' | 'map' | 'emptiness'>
> & {
	regions?: AnalysedRegion[] | Promise<AnalysedRegion[]>;
	metricsAvailable?: boolean | Promise<boolean>;
	map?: AnalysisMap | Promise<AnalysisMap>;
	/** Settled like the three above, so a test writes the cause rather than a promise of one. */
	emptiness?: ShapeEmptiness | null | Promise<ShapeEmptiness | null>;
};

/**
 * The wrapper for the three fields whose promise is **unconditional**.
 *
 * `null` on `map` becomes `Promise.resolve(null)` rather than staying `null`: the inner null is the
 * only null there is. A test writing `map: null` means *this place published no map-level readout*,
 * which is a fact about the place; an outer null would be a fourth state on a field that already
 * carries three.
 */
const always = <T>(v: T | Promise<T>): Promise<T> =>
	v instanceof Promise ? v : Promise.resolve(v);

const base: AnalysisViewData = {
	owner: '@me',
	place: null,
	alsoNamed: [],
	choices: [],
	refusal: null,
	regions: Promise.resolve([]),
	metricsAvailable: Promise.resolve(true),
	emptiness: Promise.resolve(null),
	map: Promise.resolve(null),
};

const view = (over: ViewOverrides = {}): AnalysisViewData => {
	const { regions, metricsAvailable, map, emptiness, ...settled } = over;
	return {
		...base,
		...settled,
		regions: always(regions ?? []),
		metricsAvailable: always(metricsAvailable ?? true),
		// `=== undefined` rather than `??`, on the two fields where a caller passing `null` means it.
		map: always(map === undefined ? null : map),
		emptiness: always(emptiness === undefined ? null : emptiness),
	};
};

const contextView = (over: ViewOverrides = {}): AnalysisViewData =>
	view({
		place: { kind: 'context', ref: '@me/temper', title: '@me/temper' },
		...analyseShape(fixture.context.shape, fixture.context.region_metrics),
		...over,
	});

const cogmapView = (over: ViewOverrides = {}): AnalysisViewData =>
	view({
		place: {
			kind: 'cogmap',
			ref: '019f2391-e001-7933-b88a-28fb92e56ac1',
			title: fixture.cogmap.name,
		},
		...analyseShape(fixture.cogmap.shape, fixture.cogmap.region_metrics),
		map: {
			telos: { id: fixture.cogmap.analytics.telos_resource_id, title: 'Temper — telos charter' },
			staleness: fixture.cogmap.analytics.staleness,
			regulation: fixture.cogmap.analytics.regulation,
		},
		...over,
	});

/**
 * Render, and wait for the one read to land.
 *
 * `render` is synchronous and returns while the page is still showing its arriving marker — which
 * is C1 from the other side, and the reason this helper exists rather than a `tick()` at twenty
 * call sites. The wait is on the groupings section, which lives in the `{:then}` branch and is
 * drawn for every place that has measurements.
 */
const painted = async (data: AnalysisViewData) => {
	const rendered = render(AnalysisPage, { data });
	await vi.waitFor(() => {
		expect(rendered.container.querySelector('.groupings')).not.toBeNull();
	});
	return rendered;
};

describe('the page declares what it is, before it shows anything', () => {
	it('says the content is the machine’s and not the reader’s', () => {
		render(AnalysisPage, { data: contextView() });
		const said = (screen.getByTestId('kind-declaration').textContent ?? '').replace(/\s+/g, ' ');

		expect(said).toMatch(/measurement of your work/i);
		expect(said).toMatch(/none of it is something you wrote/i);
	});

	it('declares itself even when there is nothing to measure', () => {
		// A page that announces its kind only when it has content announces nothing. Both the
		// refusal and the index must carry it too.
		render(AnalysisPage, {
			data: view({ refusal: { kind: 'no-place-resolved', named: 2 } }),
		});
		expect(screen.getByTestId('kind-declaration')).toBeTruthy();
	});

	it('the declaration is the first thing in the document', () => {
		const { container } = render(AnalysisPage, { data: cogmapView() });
		const first = container.querySelector('.analysis')?.firstElementChild;

		expect(first?.getAttribute('data-testid')).toBe('kind-declaration');
	});
});

describe('nothing on the page is presented as a normalised score', () => {
	it('no percentage, bar, meter or ratio appears anywhere', async () => {
		const { container } = await painted(cogmapView());

		expect(container.querySelector('progress')).toBeNull();
		expect(container.querySelector('meter')).toBeNull();
		expect(container.querySelector('[style*="width"]')).toBeNull();
		// The raw quantities run to 2342.2, so a % sign on one would be a claim that it is a
		// fraction. Scoped to the METRIC cells and not the whole table: a grouping's label is the
		// reader's own authored text and five of them legitimately contain a percent sign
		// ("Ingest throughput: embed is 94%, the network is 6%"). The rule governs how the machine
		// presents its own arithmetic, never what a person wrote.
		const figures = [...container.querySelectorAll('tbody td')];
		expect(figures.length).toBeGreaterThan(0);
		for (const cell of figures) {
			expect(cell.textContent).not.toContain('%');
			expect(cell.textContent).not.toContain('/');
		}
	});

	it('a raw figure appears beside the span THIS place measures', async () => {
		await painted(cogmapView());
		const legend = screen.getByTestId('metric-legend').textContent ?? '';

		expect(legend).toContain('here: 0 – 2342.2');
	});

	it('an uncomputed value is a dash and never a zero', async () => {
		const { container } = await painted(cogmapView());
		const absent = [...container.querySelectorAll('td.absent')];

		// Measured: 4 of the 406 groupings have no cohesion and no telos alignment.
		expect(absent.length).toBeGreaterThan(0);
		for (const cell of absent) expect(cell.textContent?.trim()).toBe('—');
	});
});

describe('a quantity that does not vary is said once instead of tabulated', () => {
	it('internal tension gets no column for a context, and a sentence instead', async () => {
		const { container } = await painted(contextView());
		const headers = [...container.querySelectorAll('thead th')].map((h) => h.textContent);
		const legend = screen.getByTestId('metric-legend').textContent ?? '';

		expect(headers).not.toContain('Disagreement among the members');
		expect(legend).toContain('Every grouping here measures 0.');
	});

	it('the same quantity DOES get a column where it actually varies', async () => {
		// The cogmap's internal_tension spans 0 → 4.7, so the collapse is a property of the data
		// rather than a decision about the metric.
		const { container } = await painted(cogmapView());
		const headers = [...container.querySelectorAll('thead th')].map((h) => h.textContent);

		expect(headers).toContain('Disagreement among the members');
	});

	it('a collapsed quantity is still reported — nothing is dropped', async () => {
		await painted(contextView());
		const legend = screen.getByTestId('metric-legend').textContent ?? '';

		expect(legend).toContain('Disagreement among the members');
	});
});

describe('the displaced payload is here, whole', () => {
	it('member count, salience and coherence all reach the reader', async () => {
		// RegionHoverCard.svelte:17-19 — at 87ccd211, the last commit before Beat D deleted the
		// file — rendered exactly these three on the navigational canvas.
		const { container } = await painted(cogmapView());
		const headers = [...container.querySelectorAll('thead th')].map((h) => h.textContent);

		expect(headers).toContain('Resources in it');
		expect(headers).toContain('How strongly this place ranks it');
		expect(headers).toContain('How alike the members are');
	});

	it('every grouping the place publishes gets a row, with no top-N', async () => {
		const { container } = await painted(contextView());

		expect(container.querySelectorAll('tbody tr')).toHaveLength(501);
	});

	it('the count says whose order it is showing', async () => {
		await painted(contextView());
		expect(screen.getByTestId('grouping-count').textContent).toBe(
			'501 groupings, in the order this place itself ranks them.',
		);
	});
});

/**
 * The empty view, at the one door where the reader is a person.
 *
 * `[2026-08-24]` Until the envelope crossed this door the page spelled every empty read *"This
 * place has no groupings yet."*, whose *yet* asserts `never_clustered`. These render through the
 * real component rather than asserting on {@link describeGroupingCount} alone, because the unit
 * test cannot see the wiring — the field has to reach the template from the same awaited read as
 * the rows, and a page that computed the right sentence and rendered the old one would pass there.
 */
describe('an empty groupings list tells the person which cause they are in', () => {
	it('renders the cause the read carried, not a cause it guessed', async () => {
		const { container } = await painted(contextView({ regions: [], emptiness: 'nothing_visible' }));
		const said = container.querySelector('[data-testid="grouping-count"]')?.textContent ?? '';

		expect(said).toContain('has been grouped');
		expect(said).toContain('not evidence that you are missing access');
		// The defect, stated as the assertion that would have caught it.
		expect(said).not.toContain('no groupings yet');
	});

	/** Queried through each render's own `container`, so two pages can be compared in one test. */
	const emptyPageSays = async (emptiness: ShapeEmptiness) => {
		const { container } = await painted(contextView({ regions: [], emptiness }));
		return container.querySelector('[data-testid="grouping-count"]')?.textContent ?? '';
	};

	it('a never-clustered place and an unreadable one do not read alike', async () => {
		const never = await emptyPageSays('never_clustered');
		const denied = await emptyPageSays('unreadable_or_absent');

		expect(never).not.toBe(denied);
		expect(never).toContain('Nothing here is broken');
		expect(denied).toContain('cannot tell you which');
	});

	/**
	 * The cause is streamed from the SAME read as the rows, so it must not paint before them. If it
	 * arrived on its own promise a reader could be told WHY the list is empty while the list is
	 * still arriving — a cause attached to a row set nobody has seen yet.
	 *
	 * **The flush is what makes this bite.** A first draft asserted synchronously, immediately after
	 * `render`, and that could never fail: nothing promise-based has resolved at that point, so a
	 * hoisted `{#await data.emptiness}` block above the real await would have passed it. Draining the
	 * microtask queue lets `emptiness` — which settles now — paint if anything is wired to let it,
	 * while `regions` never settles. Only then is a null assertion evidence of anything.
	 */
	it('does not paint a cause while the rows are still arriving', async () => {
		const { container } = render(AnalysisPage, {
			data: contextView({
				regions: new Promise<AnalysedRegion[]>(() => {}), // never settles
				emptiness: 'never_clustered', // settles immediately
			}),
		});

		// Drain: anything awaiting only `emptiness` has had every chance to render by now.
		for (let i = 0; i < 8; i++) await Promise.resolve();
		await tick();

		expect(container.querySelector('[data-testid="grouping-count"]')).toBeNull();
		expect(container.querySelector('.groupings')).toBeNull();
	});
});

describe('what a place does not have is declared, not fabricated', () => {
	it('a context says why it has no charter rather than reporting a failure', async () => {
		await painted(contextView());
		const said = screen.getByTestId('map-absent').textContent ?? '';

		expect(said).toContain('a context has neither');
		expect(said).not.toMatch(/error|failed|not found/i);
		expect(screen.queryByTestId('staleness')).toBeNull();
	});

	it('an empty regulation set reads as a fact about the map', async () => {
		// Measured: every readable map returns []. This is the routine case, not an edge case.
		await painted(cogmapView());

		expect(screen.getByTestId('regulation').textContent).toBe(
			'No concepts have been set to regulate this map.',
		);
	});

	it('a declined map-level read is unavailable, never an error', async () => {
		await painted(cogmapView({ map: null }));
		const said = screen.getByTestId('map-absent').textContent ?? '';

		expect(said).toContain('not available');
		expect(said).not.toMatch(/error|failed/i);
	});

	it('a metrics read that did not answer says unknown, and does not zero the columns', async () => {
		const data = cogmapView({
			...analyseShape(fixture.cogmap.shape, null),
			map: null,
		});
		const { container } = await painted(data);

		expect(screen.getByTestId('metrics-unavailable').textContent).toContain('unknown');
		// The surface tier's own two survive — they never came from the read that failed.
		const headers = [...container.querySelectorAll('thead th')].map((h) => h.textContent);
		expect(headers).toContain('Resources in it');
		expect(headers).toContain('How strongly this place ranks it');
	});
});

describe('the door answers an address it cannot resolve, and one with no address at all', () => {
	it('a named place that does not resolve is refused, never widened', () => {
		render(AnalysisPage, { data: view({ refusal: { kind: 'no-place-resolved', named: 1 } }) });

		expect(screen.getByText(/Nothing to measure for that place/)).toBeTruthy();
		expect(screen.queryByRole('table')).toBeNull();
	});

	it('no address at all offers the places the reader can read', () => {
		const data = view({
			choices: [
				{ kind: 'context' as const, ref: '@me/temper', title: '@me/temper' },
				{ kind: 'cogmap' as const, ref: 'abc', title: 'Temper — self-cognition' },
			],
		});
		const { container } = render(AnalysisPage, { data });

		expect(container.querySelectorAll('.choices a')).toHaveLength(2);
	});

	it('places the reader also named are linked, not silently dropped', () => {
		const data = cogmapView({
			alsoNamed: [{ kind: 'context', ref: '@me/temper', title: '@me/temper' }],
		});
		render(AnalysisPage, { data });

		expect(screen.getByTestId('also-named').textContent).toContain('@me/temper');
	});
});

/**
 * C1–C4 on the region this route streams: **the measurements**.
 *
 * What the page owes a reader while that read is in flight is everything that does not depend on
 * it — the declaration, the title of the place being measured, the also-named line — and it owes
 * them a region that says, in words, that the rest is still coming. The map-level section and the
 * groupings section are two views of one read, so they share one marker.
 */
describe('the page declares what its own read is doing', () => {
	/** Never settles, so anything that waits for it never paints. */
	const pending = () => new Promise<AnalysedRegion[]>(() => {});

	/** Rejects, with §5.3's *other* catch attached — the one that is not the template's. */
	const broken = (): Promise<AnalysedRegion[]> => {
		const p = Promise.reject(new Error('503'));
		p.catch(() => {});
		return p;
	};

	it('C1: the place it is measuring is named while the measurements are still in flight', () => {
		const { container } = render(AnalysisPage, {
			data: cogmapView({ regions: pending() }),
		});

		// Everything here is decided above the read, so none of it may wait on one.
		expect(screen.getByTestId('kind-declaration')).toBeTruthy();
		expect(screen.getByRole('heading', { level: 1 }).textContent).toBe(fixture.cogmap.name);
		expect(container.querySelector('table')).toBeNull();
	});

	it('C2: and the region that is waiting says so in words, not only in colour', () => {
		const { container } = render(AnalysisPage, {
			data: cogmapView({ regions: pending() }),
		});
		const arriving = container.querySelector('[data-testid="region-arriving"]');

		expect(arriving).not.toBeNull();
		// The sentence, with the decorative glyph stripped — what reaches the accessibility tree.
		expect(sentenceOf(arriving)).toBe('Loading measurements…');
	});

	it('one read, one marker: the map-level section waits with the groupings', () => {
		// They are two views of a single read. Two arriving markers would tell the reader those
		// regions could disagree about whether it answered, and they cannot.
		const { container } = render(AnalysisPage, {
			data: cogmapView({ regions: pending() }),
		});

		expect(container.querySelectorAll('[data-testid="region-arriving"]')).toHaveLength(1);
		expect(container.querySelector('.map-level')).toBeNull();
		expect(container.querySelector('.groupings')).toBeNull();
	});

	it('C3: measurements that will not read say so, and do NOT read as still arriving', async () => {
		const { container } = render(AnalysisPage, { data: cogmapView({ regions: broken() }) });

		await vi.waitFor(() => {
			expect(container.querySelector('[data-testid="region-failed"]')).not.toBeNull();
		});
		// The perpetual-skeleton bug: a read that will not resolve must stop presenting as one that
		// has not resolved YET.
		expect(container.querySelector('[data-testid="region-arriving"]')).toBeNull();
		// And the page still says which place it was measuring — that never depended on the read.
		expect(screen.getByRole('heading', { level: 1 }).textContent).toBe(fixture.cogmap.name);
	});

	/**
	 * The refusal on the second call site, and a different shape from the rail's: a page-level region
	 * rather than one panel of several.
	 *
	 * The rejection is what `handleError` hands the client runtime, not a `GaveUp` instance — the
	 * class does not survive serialisation, so a test that threw the instance would be asserting a
	 * discriminator the browser never receives. That the hook's output really travels is witnessed in
	 * `src/hooks.server.test.ts`, against SvelteKit's own serialiser.
	 */
	const stopped = (): Promise<AnalysedRegion[]> => {
		const p = Promise.reject(describeFailure(new GaveUp('measurements', 8000), 'Internal Error'));
		p.catch(() => {});
		return p as Promise<AnalysedRegion[]>;
	};

	it('C4: measurements the system gave up on do not present like ones that failed', async () => {
		const gaveUp = render(AnalysisPage, { data: cogmapView({ regions: stopped() }) });
		await vi.waitFor(() => {
			expect(gaveUp.container.querySelector('[data-testid="region-gave-up"]')).not.toBeNull();
		});
		const stoppedWords = sentenceOf(
			gaveUp.container.querySelector('[data-testid="region-gave-up"]'),
		);
		// Read BEFORE unmounting: a detached container answers `null` to everything, which would make
		// this the inert assertion the README warns about rather than the perpetual-skeleton check.
		const stillArriving = gaveUp.container.querySelector('[data-testid="region-arriving"]');
		gaveUp.unmount();

		const failed = render(AnalysisPage, { data: cogmapView({ regions: broken() }) });
		await vi.waitFor(() => {
			expect(failed.container.querySelector('[data-testid="region-failed"]')).not.toBeNull();
		});
		const failedWords = sentenceOf(failed.container.querySelector('[data-testid="region-failed"]'));

		// It names the read, it is not the failure's sentence, and it is not the skeleton either.
		expect(stoppedWords.toLowerCase()).toContain('measurements');
		expect(stoppedWords).not.toBe(failedWords);
		expect(stillArriving).toBeNull();
	});

	it('C4: a failed read does not present like a place with nothing measured', async () => {
		const empty = await painted(cogmapView({ regions: [], map: null }));
		const emptyWords = (empty.container.querySelector('.groupings')?.textContent ?? '')
			.replace(/\s+/g, ' ')
			.trim();
		empty.unmount();

		const failed = render(AnalysisPage, { data: cogmapView({ regions: broken() }) });
		await vi.waitFor(() => {
			expect(failed.container.querySelector('[data-testid="region-failed"]')).not.toBeNull();
		});
		const failedWords = sentenceOf(failed.container.querySelector('[data-testid="region-failed"]'));

		// Differential, per spec §3.3, and on the SENTENCE with the glyph stripped — so neither a
		// redesign of either state nor a one-channel difference satisfies it.
		expect(emptyWords).not.toBe('');
		expect(failedWords).not.toBe('');
		expect(failedWords).not.toBe(emptyWords);
		// The load-bearing half: a failed read must never be spelled as an absence (spec §5.1).
		expect(failed.container.querySelector('[data-testid="grouping-count"]')).toBeNull();
	});

	it('a refusal is the answer, so it renders before the read rather than behind it', () => {
		// The two addressed refusals are decided above every read, and a refusal arriving behind a
		// loading marker would be a delay dressed as an answer.
		render(AnalysisPage, {
			data: view({ refusal: { kind: 'nothing-to-analyse' }, regions: pending() }),
		});

		expect(screen.getByText(/There is nothing here yet/)).toBeTruthy();
		expect(screen.queryByTestId('region-arriving')).toBeNull();
	});
});

describe('the groupings are marks on their own axes, not only rows in a table', () => {
	/**
	 * `[from the reader session, 2026-08-21, repeated 2026-08-22]` *"the long, long grouping table is
	 * interesting but probably the wrong visual register for that much data density about things
	 * folks don't really understand. a visualization that they could hover over pieces to understand
	 * the grouping would be better."*
	 *
	 * Wiring, not appearance — nothing here may claim the plot is legible (`src/test/README.md`).
	 * What it pins is that every grouping reaches the picture, that a quantity with no spread is
	 * said rather than drawn, and that pointing at one strip lights the same grouping on all of them,
	 * which is the property a table cannot have.
	 */
	it('every grouping the place publishes is a mark — no top-N in the picture either', async () => {
		const { container } = await painted(contextView());
		const strips = container.querySelectorAll('[data-testid="grouping-strips"] svg');
		expect(strips.length).toBeGreaterThan(0);
		for (const svg of strips) {
			// 501 groupings; a strip carries one mark per grouping that HAS a value for it, and the
			// rest are declared in that strip's own "N of M have no value for this" line.
			expect(svg.querySelectorAll('circle').length).toBeGreaterThan(400);
		}
	});

	it('a quantity with no spread is SAID, never drawn as a row of identical marks', async () => {
		// `internal_tension` is identically 0 across all 501 groupings of the captured context. A
		// strip of 501 marks stacked on one point is a picture of nothing that invites a reader to
		// look for structure in it — the same argument that keeps it out of the table's columns.
		const { container } = await painted(contextView());
		const flat = container.querySelector('[data-testid="flat-internal_tension"]');
		expect(flat).not.toBeNull();
		expect(sentenceOf(flat)).toContain('Every grouping here measures 0');
	});

	it('each strip is labelled with the span THIS place measures, never a 0–100 scale', async () => {
		// The sound half of what §8.2 was reaching for, kept on its merits: the axes are the real
		// ones, so no two strips share a scale and a mark's position is never a rank.
		const { container } = await painted(contextView());
		const scales = [...container.querySelectorAll('[data-testid="grouping-strips"] .scale')];
		expect(scales.length).toBeGreaterThan(0);
		for (const s of scales) expect(s.textContent).not.toMatch(/%/);
	});

	it('the median is ticked WHERE IT IS, not in the middle of the strip', async () => {
		// `[found by looking, 2026-08-22]` The median started as the centre item of a
		// space-between row, so every strip printed it at 50% of the width whatever its value.
		// Measured on this fixture: `centrality` and `reference_standing` both said "median 0" at
		// the halfway mark while the real median sits hard against the left end — a label claiming
		// a position it does not have, wrong on all six strips, by up to half the width.
		//
		// No test could have caught it: jsdom computes no layout, so this asserts the DECLARED
		// position rather than the rendered one, which is the most a test here may claim.
		const { container } = await painted(contextView());
		const labels = [...container.querySelectorAll('[data-testid="grouping-strips"] .median-label')];
		expect(labels.length).toBeGreaterThan(0);

		const lefts = labels.map((l) => (l as HTMLElement).style.left);
		for (const left of lefts) expect(left).toMatch(/%$/);
		// The bug's signature is every strip agreeing on one position. A median that genuinely sat
		// at the same place on all six would be a coincidence worth failing on and looking at.
		expect(new Set(lefts).size).toBeGreaterThan(1);
		expect(lefts.every((l) => l === '50%')).toBe(false);
	});

	it('a median that lands ON a bound is said once, by that bound', async () => {
		// `centrality` and `reference_standing` both have a median of 0, which IS their minimum.
		// A floating "median 0" stacked on the "0" end label would read as two facts about two
		// places; it is one fact about one.
		const { container } = await painted(contextView());
		const ends = [
			...container.querySelectorAll('[data-testid="grouping-strips"] .scale .is-median'),
		];
		expect(ends.length).toBeGreaterThan(0);
		for (const e of ends) expect(e.textContent).toMatch(/median/);
	});

	it('each quantity says what it tells you AND what it does not', async () => {
		const { container } = await painted(contextView());
		const readings = [...container.querySelectorAll('[data-testid="grouping-strips"] .reading')];
		expect(readings.length).toBe(METRICS.length);
		// The second half is the one that exists because a gloss alone invited readers to supply a
		// stronger meaning than the number carries.
		for (const r of readings) expect(r.textContent).toMatch(/does not/i);
	});

	it('pointing at one strip lights the same grouping on every strip — what a table cannot do', async () => {
		const { container } = await painted(contextView());
		const readout = container.querySelector('[data-testid="strip-readout"]');
		expect(sentenceOf(readout)).toContain('Point at a strip');

		const svg = container.querySelector('[data-testid="grouping-strips"] svg') as SVGElement;
		// jsdom gives every element a zero-size box, so the pointer maths resolves to the first
		// mark rather than one chosen by position. That is fine for this assertion: what is under
		// test is that ONE grouping becomes active everywhere, not which one.
		await fireEvent.pointerMove(svg, { clientX: 10, clientY: 10 });

		const strips = container.querySelectorAll('[data-testid="grouping-strips"] svg');
		for (const s of strips) {
			expect(s.querySelectorAll('circle.lit').length).toBeLessThanOrEqual(1);
		}
		const lit = container.querySelectorAll('[data-testid="grouping-strips"] circle.lit');
		expect(lit.length).toBeGreaterThan(1); // the same grouping, on more than one strip
		expect(sentenceOf(readout)).not.toContain('Point at a strip');
	});

	it('the figures are kept and reachable, not deleted', async () => {
		// The strips answer "what is this telling me"; the table answers "what exactly does this one
		// measure". Behind a disclosure, and still every row in the document.
		const { container } = await painted(contextView());
		expect(container.querySelector('details.figures')).not.toBeNull();
		expect(container.querySelectorAll('tbody tr')).toHaveLength(501);
	});
});
