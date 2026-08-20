import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { declareBounds } from '$lib/graph/bound';
import type { GraphPlan } from '$lib/graph/composition';
import { buildGraph } from '$lib/graph/model';
import { buildReadout } from '$lib/graph/readout';
import type { GraphViewData } from '$lib/graph/view';
import type { CogmapRegionRow } from '$lib/types/generated/cognitive_maps';
import type { QueryResponse } from '$lib/types/generated/query';
import { resetAppContext, setPage } from '../../../test/app-context';
import GraphPage from './GraphPage.svelte';

vi.mock('$app/stores', () => import('../../../test/app-context'));
vi.mock('$app/navigation', () => import('../../../test/app-context'));

/**
 * The surface, rendered against a response the deployed substrate actually sent.
 *
 * Beat A shipped 69 green tests and **zero callers**, and three defects fell out the moment its
 * output met a real server. This is the other half of that lesson: the modules are green, so the
 * question left is whether the thing they compose into renders at all — and whether what a reader
 * sees on it is what the clauses require.
 */
const fixture = JSON.parse(
	readFileSync(
		join(import.meta.dirname, '../../../test/fixtures/graph-successor-flagship.json'),
		'utf8',
	),
) as { response: QueryResponse; shape_rows: CogmapRegionRow[] };

const plan = {
	composition: { outcome: { returns: [] }, stages: [] },
	anchorsAsked: Array.from({ length: 12 }, (_, i) => ({ ref: `a${i}` })),
	anchorsAvailable: 12,
	surveyStages: Array.from({ length: 12 }, (_, i) => `s${i + 1}`),
	walkStage: 'w',
} as unknown as GraphPlan;

const view = (over: Partial<GraphViewData> = {}): GraphViewData => ({
	owner: '@me',
	question: 'what keeps a surface honest about what it left out?',
	borrowedFrom: null,
	refusal: null,
	model: buildGraph({ response: fixture.response, plan, seeds: [] }),
	bound: declareBounds(fixture.response, plan, null),
	readout: buildReadout(fixture.response, { rows: fixture.shape_rows, complete: true }),
	selected: null,
	selectedExcerpt: null,
	selectedTrail: null,
	...over,
});

beforeEach(() => {
	resetAppContext();
	setPage('/graph/@me?q=what+keeps+a+surface+honest', { owner: '@me' });
});

describe('the surface renders the reader’s own material', () => {
	it('draws one mark per node and one per deduped edge, and nothing else', () => {
		const { container } = render(GraphPage, { data: view() });
		const model = view().model;

		expect(container.querySelectorAll('.node-chip')).toHaveLength(model.nodes.length);
		expect(container.querySelectorAll('.edge')).toHaveLength(model.edges.length);
		// The whole vocabulary. A third mark class appearing here is the failure the clause names.
		const markClasses = new Set(
			[...container.querySelectorAll('svg g[class]')]
				.map((g) => g.getAttribute('class')?.split(' ')[0])
				.filter((c): c is string => !!c && c !== 'labels'),
		);
		expect([...markClasses].sort()).toEqual(['edge', 'node-chip']);
	});

	it('draws 101 edges, not the 1,973 via entries that produced them', () => {
		const { container } = render(GraphPage, { data: view() });

		expect(view().model.viaEntries).toBe(1973);
		expect(container.querySelectorAll('.edge')).toHaveLength(101);
	});
});

describe('the bound line is on screen and is not dismissible', () => {
	it('renders whether or not the view is partial', () => {
		render(GraphPage, { data: view() });

		expect(screen.getByTestId('bound-line').textContent).toMatch(/^Showing /);
	});

	it('states the applied funnel width and the places asked', () => {
		render(GraphPage, { data: view() });
		const line = screen.getByTestId('bound-line').textContent ?? '';

		expect(line).toContain('3 groupings per place');
		expect(line).toContain('12 of 12 places');
		expect(line).toContain('more exist');
	});

	it('carries no control that could hide it', () => {
		const { container } = render(GraphPage, { data: view() });
		const bound = container.querySelector('[data-testid="bound-line"]');

		expect(bound?.querySelector('button')).toBeNull();
		expect(bound?.querySelector('a')).toBeNull();
	});
});

describe('derived structure appears in exactly one place, and a reader can confirm by looking', () => {
	it('the readout names the groupings the answer drew on', () => {
		const { container } = render(GraphPage, { data: view() });
		const why = container.querySelector('.why');

		expect(why?.textContent).toContain('These came from');
		expect(why?.querySelectorAll('.groupings li').length).toBeGreaterThan(0);
	});

	it('no grouping name is ever drawn as a mark', () => {
		const { container } = render(GraphPage, { data: view() });
		const labels = [...container.querySelectorAll('svg text')].map((t) => t.textContent ?? '');
		const grouped = [...(container.querySelectorAll('.groupings li') ?? [])].map(
			(li) => li.textContent ?? '',
		);

		for (const g of grouped) {
			expect(labels).not.toContain(g);
		}
	});

	it('the canvas never renders a score or a salience', () => {
		const { container } = render(GraphPage, { data: view() });
		const svg = container.querySelector('svg')?.textContent ?? '';

		expect(svg).not.toMatch(/\d\.\d{3,}/);
	});
});

describe('labels are placed, not carpeted', () => {
	it('captions a bounded set and still lists every node accessibly', () => {
		const { container } = render(GraphPage, { data: view() });
		const model = view().model;

		const captions = container.querySelectorAll('.labels text');
		expect(captions.length).toBeGreaterThan(0);
		expect(captions.length).toBeLessThan(model.nodes.length);
		// Dropping a caption drops no ROW: every node is still named in the a11y mirror.
		expect(container.querySelectorAll('.graph-a11y li')).toHaveLength(model.nodes.length);
	});
});

describe('a refusal is a refusal, never a widened answer', () => {
	it('names what the reader asked for and offers a way on', () => {
		render(GraphPage, {
			data: view({ refusal: { kind: 'no-place-resolved', named: 2 }, bound: null, readout: null }),
		});

		expect(screen.getByText(/Nothing to show for those places/)).toBeTruthy();
		expect(screen.getByText(/See everything you can read/)).toBeTruthy();
	});

	it('draws no canvas at all, rather than an empty one that looks like an answer', () => {
		const { container } = render(GraphPage, {
			data: view({ refusal: { kind: 'nothing-to-ask' }, bound: null, readout: null }),
		});

		expect(container.querySelector('svg')).toBeNull();
		expect(container.querySelector('[data-testid="bound-line"]')).toBeNull();
	});
});

describe('the rail opens on a node — and only on a node', () => {
	const selected = () => {
		const model = view().model;
		// The busiest node, so its neighbour list is non-trivial.
		return model.nodes.reduce((a, b) => (b.degree > a.degree ? b : a));
	};

	it('shows the resource, where it lives, and how it was reached', () => {
		const node = selected();
		render(GraphPage, { data: view({ selected: node.id, selectedExcerpt: 'A first paragraph.' }) });
		const rail = screen.getByTestId('node-rail');

		expect(rail.textContent).toContain(node.title);
		expect(rail.textContent).toContain('A first paragraph.');
		expect(rail.textContent).toContain('IN');
		expect(rail.textContent).toContain('HOW');
	});

	it('lists neighbours from the graph on screen, with no second read', () => {
		const node = selected();
		render(GraphPage, { data: view({ selected: node.id }) });

		expect(screen.getByTestId('node-rail').textContent).toContain(`NEIGHBORS · ${node.degree}`);
	});

	it('offers a way into the resource itself', () => {
		const node = selected();
		render(GraphPage, { data: view({ selected: node.id }) });

		expect(screen.getByTestId('view-full-resource').getAttribute('href')).toBe(
			`/vault/r/${node.id}`,
		);
	});

	it('a sel naming a node this answer does not contain opens nothing', () => {
		// Rather than a panel describing a resource that is not on screen.
		render(GraphPage, { data: view({ selected: 'not-in-this-answer' }) });

		expect(screen.queryByTestId('node-rail')).toBeNull();
	});
});
