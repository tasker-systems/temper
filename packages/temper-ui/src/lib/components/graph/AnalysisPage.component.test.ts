import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { analyseShape } from '$lib/graph/analysis';
import type { AnalysisViewData } from '$lib/graph/view';
import type {
	CogmapAnalyticsRow,
	CogmapRegionMetricsRow,
	CogmapRegionRow,
} from '$lib/types/generated/cognitive_maps';
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

const base: AnalysisViewData = {
	owner: '@me',
	place: null,
	alsoNamed: [],
	choices: [],
	refusal: null,
	regions: [],
	metricsAvailable: true,
	map: null,
};

const contextView = (over: Partial<AnalysisViewData> = {}): AnalysisViewData => ({
	...base,
	place: { kind: 'context', ref: '@me/temper', title: '@me/temper' },
	...analyseShape(fixture.context.shape, fixture.context.region_metrics),
	...over,
});

const cogmapView = (over: Partial<AnalysisViewData> = {}): AnalysisViewData => ({
	...base,
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
			data: { ...base, refusal: { kind: 'no-place-resolved', named: 2 } },
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
	it('no percentage, bar, meter or ratio appears anywhere', () => {
		const { container } = render(AnalysisPage, { data: cogmapView() });

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

	it('a raw figure appears beside the span THIS place measures', () => {
		render(AnalysisPage, { data: cogmapView() });
		const legend = screen.getByTestId('metric-legend').textContent ?? '';

		expect(legend).toContain('here: 0 – 2342.2');
	});

	it('an uncomputed value is a dash and never a zero', () => {
		const { container } = render(AnalysisPage, { data: cogmapView() });
		const absent = [...container.querySelectorAll('td.absent')];

		// Measured: 4 of the 406 groupings have no cohesion and no telos alignment.
		expect(absent.length).toBeGreaterThan(0);
		for (const cell of absent) expect(cell.textContent?.trim()).toBe('—');
	});
});

describe('a quantity that does not vary is said once instead of tabulated', () => {
	it('internal tension gets no column for a context, and a sentence instead', () => {
		const { container } = render(AnalysisPage, { data: contextView() });
		const headers = [...container.querySelectorAll('thead th')].map((h) => h.textContent);
		const legend = screen.getByTestId('metric-legend').textContent ?? '';

		expect(headers).not.toContain('Disagreement among the members');
		expect(legend).toContain('Every grouping here measures 0.');
	});

	it('the same quantity DOES get a column where it actually varies', () => {
		// The cogmap's internal_tension spans 0 → 4.7, so the collapse is a property of the data
		// rather than a decision about the metric.
		const { container } = render(AnalysisPage, { data: cogmapView() });
		const headers = [...container.querySelectorAll('thead th')].map((h) => h.textContent);

		expect(headers).toContain('Disagreement among the members');
	});

	it('a collapsed quantity is still reported — nothing is dropped', () => {
		render(AnalysisPage, { data: contextView() });
		const legend = screen.getByTestId('metric-legend').textContent ?? '';

		expect(legend).toContain('Disagreement among the members');
	});
});

describe('the displaced payload is here, whole', () => {
	it('member count, salience and coherence all reach the reader', () => {
		// RegionHoverCard.svelte:17-19 rendered exactly these three on the navigational canvas.
		const { container } = render(AnalysisPage, { data: cogmapView() });
		const headers = [...container.querySelectorAll('thead th')].map((h) => h.textContent);

		expect(headers).toContain('Resources in it');
		expect(headers).toContain('How strongly this place ranks it');
		expect(headers).toContain('How alike the members are');
	});

	it('every grouping the place publishes gets a row, with no top-N', () => {
		const { container } = render(AnalysisPage, { data: contextView() });

		expect(container.querySelectorAll('tbody tr')).toHaveLength(501);
	});

	it('the count says whose order it is showing', () => {
		render(AnalysisPage, { data: contextView() });
		expect(screen.getByTestId('grouping-count').textContent).toBe(
			'501 groupings, in the order this place itself ranks them.',
		);
	});
});

describe('what a place does not have is declared, not fabricated', () => {
	it('a context says why it has no charter rather than reporting a failure', () => {
		render(AnalysisPage, { data: contextView() });
		const said = screen.getByTestId('map-absent').textContent ?? '';

		expect(said).toContain('a context has neither');
		expect(said).not.toMatch(/error|failed|not found/i);
		expect(screen.queryByTestId('staleness')).toBeNull();
	});

	it('an empty regulation set reads as a fact about the map', () => {
		// Measured: every readable map returns []. This is the routine case, not an edge case.
		render(AnalysisPage, { data: cogmapView() });

		expect(screen.getByTestId('regulation').textContent).toBe(
			'No concepts have been set to regulate this map.',
		);
	});

	it('a declined map-level read is unavailable, never an error', () => {
		render(AnalysisPage, { data: cogmapView({ map: null }) });
		const said = screen.getByTestId('map-absent').textContent ?? '';

		expect(said).toContain('not available');
		expect(said).not.toMatch(/error|failed/i);
	});

	it('a metrics read that did not answer says unknown, and does not zero the columns', () => {
		const data = cogmapView({
			...analyseShape(fixture.cogmap.shape, null),
			map: null,
		});
		const { container } = render(AnalysisPage, { data });

		expect(screen.getByTestId('metrics-unavailable').textContent).toContain('unknown');
		// The surface tier's own two survive — they never came from the read that failed.
		const headers = [...container.querySelectorAll('thead th')].map((h) => h.textContent);
		expect(headers).toContain('Resources in it');
		expect(headers).toContain('How strongly this place ranks it');
	});
});

describe('the door answers an address it cannot resolve, and one with no address at all', () => {
	it('a named place that does not resolve is refused, never widened', () => {
		render(AnalysisPage, { data: { ...base, refusal: { kind: 'no-place-resolved', named: 1 } } });

		expect(screen.getByText(/Nothing to measure for that place/)).toBeTruthy();
		expect(screen.queryByRole('table')).toBeNull();
	});

	it('no address at all offers the places the reader can read', () => {
		const data = {
			...base,
			choices: [
				{ kind: 'context' as const, ref: '@me/temper', title: '@me/temper' },
				{ kind: 'cogmap' as const, ref: 'abc', title: 'Temper — self-cognition' },
			],
		};
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
