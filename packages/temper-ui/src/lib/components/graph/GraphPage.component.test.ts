import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { declareBounds } from '$lib/graph/bound';
import type { GraphPlan } from '$lib/graph/composition';
import { buildEntryGraph, buildGraph, COMPOSITION_ARMS } from '$lib/graph/model';
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

/**
 * The arm headings, read out of the accessibility list rather than off the whole page.
 *
 * Scoped because *Why these* also renders an `<h2>`, and an arm heading assertion that swept it up
 * would be measuring the wrong thing. These headings ARE the arm legend as a reader meets it.
 */
const armHeadings = (container: HTMLElement): (string | null)[] =>
	[...(container.querySelector('.graph-a11y')?.querySelectorAll('h2') ?? [])].map(
		(h) => h.textContent,
	);

const view = (over: Partial<GraphViewData> = {}): GraphViewData => ({
	owner: '@me',
	question: 'what keeps a surface honest about what it left out?',
	borrowedFrom: null,
	refusal: null,
	model: buildGraph({ response: fixture.response, plan, seeds: [] }),
	bound: declareBounds(fixture.response, plan, null),
	readout: buildReadout(fixture.response, { rows: fixture.shape_rows, complete: true }),
	placesAsked: [
		{ kind: 'context', ref: '@me/temper', title: '@me/temper' },
		{
			kind: 'cogmap',
			ref: '019f2391-e001-7933-b88a-28fb92e56ac1',
			title: 'Temper — self-cognition',
		},
	],
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

	it('rung 2 says which instrument is right and hands the reader its door', () => {
		// Spec §6. An earlier draft had this rung fall back to drawing the recency page — 200 dots
		// and hope. Rejected: dots a reader cannot use are not more honest than a sentence saying
		// the graph is the wrong instrument here, and the sentence is the one that respects
		// `the-unstructured-reader-is-never-worse-off`. Their need is greater, so they get a working
		// door rather than an empty canvas.
		const { container } = render(GraphPage, {
			data: view({
				refusal: { kind: 'too-little-structure', inScope: 42 },
				bound: null,
				readout: null,
			}),
		});

		expect(screen.getByText(/A graph is not the right view for this yet/)).toBeTruthy();
		// It must say how much they HAVE — the reader is not empty-handed, and a sentence implying
		// they are would be a false claim about their corpus.
		expect(screen.getByText(/42 resources/)).toBeTruthy();
		expect(screen.getByText(/Browse them as a list/)).toBeTruthy();
		expect(container.querySelector('svg')).toBeNull();
	});

	it('the rung is VISIBLE — it does not render like the other refusals', () => {
		// "A reader on rung 2 is looking at a different claim than one on rung 1. Swapping silently
		// is `legibility-is-never-bought-with-silent-omission`."
		render(GraphPage, {
			data: view({
				refusal: { kind: 'too-little-structure', inScope: 42 },
				bound: null,
				readout: null,
			}),
		});
		expect(screen.queryByText(/There is nothing here yet/)).toBeNull();
		expect(screen.queryByText(/Nothing to show for/)).toBeNull();
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

describe('the unconnected field is declared rather than scattered', () => {
	it('the fixture has an unconnected population — but NOT the real one, and says which', () => {
		// **This file cannot witness the ratio the ruling was made on.** The capture's trim rule
		// keeps every survey hit a via entry references, which keeps the CONNECTED hits by
		// construction and only 4 arbitrary unconnected ones per stage. So this reads 10 of 52
		// (19%) where the live response reads 80 of 155 (51%) — recorded in the fixture's own
		// `_trimmed.degree_zero_NOT_witnessable`. Same species as the 101-edge collapse that block
		// already names: a trim preserving one property destroyed another.
		//
		// What IS witnessable here is that the field exists, is populated, and is captioned truly.
		// The population itself is asserted against the wire, not against this file.
		const model = view().model;
		const zero = model.nodes.filter((n) => n.degree === 0);

		expect(zero.length).toBeGreaterThan(0);
		expect(zero.length).toBeLessThan(model.nodes.length);
	});

	it('says how many are unconnected, in the reader’s own words', () => {
		render(GraphPage, { data: view() });
		const model = view().model;
		const zero = model.nodes.filter((n) => n.degree === 0).length;

		const caption = screen.getByTestId('unconnected-caption').textContent ?? '';
		expect(caption).toBe(
			`${zero} of these ${model.nodes.length} are not connected to anything else in this answer.`,
		);
		// no-internal-vocabulary-is-load-bearing reaches the canvas chrome too.
		for (const word of ['degree', 'orphan', 'node']) {
			expect(caption.toLowerCase()).not.toContain(word);
		}
	});

	it('still draws every one of them — the field is a place, not a bound', () => {
		const { container } = render(GraphPage, { data: view() });
		const model = view().model;

		expect(container.querySelectorAll('.node-chip')).toHaveLength(model.nodes.length);
	});

	it('adds no mark class — the field costs nothing from a vocabulary of two', () => {
		// The guard above already asserts this for the page as a whole; this one names WHY it is
		// re-asserted here, so a future field that reaches for a third mark fails on purpose.
		const { container } = render(GraphPage, { data: view() });
		const markClasses = new Set(
			[...container.querySelectorAll('svg g[class]')]
				.map((g) => g.getAttribute('class')?.split(' ')[0])
				.filter((c): c is string => !!c && c !== 'labels'),
		);

		expect([...markClasses].sort()).toEqual(['edge', 'node-chip']);
	});

	it('a fully connected answer gets no caption and no field at all', () => {
		const data = view();
		const connected = {
			...data,
			model: { ...data.model, nodes: data.model.nodes.filter((n) => n.degree > 0) },
		};

		render(GraphPage, { data: connected });
		expect(screen.queryByTestId('unconnected-caption')).toBeNull();
	});
});

describe('the receiver is reachable from the readout, not merely built', () => {
	it('every place the answer drew on links to its own measurements', () => {
		// `displaced-structure-remains-reachable` says displaced structure *remains available*, and
		// available means the reader can get there without being told a URL. A receiver nothing
		// links to would satisfy the clause's letter and none of its point.
		const { container } = render(GraphPage, { data: view() });
		const links = [...container.querySelectorAll('[data-testid="measured-links"] a')];

		expect(links).toHaveLength(2);
		expect(links[0].getAttribute('href')).toBe('/graph/@me/analysis?in=ctx%3A%40me%2Ftemper');
		expect(links[1].getAttribute('href')).toBe(
			'/graph/@me/analysis?in=map%3A019f2391-e001-7933-b88a-28fb92e56ac1',
		);
	});

	it('an answer drawn from no place offers no link rather than an empty row', () => {
		render(GraphPage, { data: view({ placesAsked: [] }) });
		expect(screen.queryByTestId('measured-links')).toBeNull();
	});
});

describe('the entry read tells the truth about its band', () => {
	/**
	 * `[observed on production — 2026-08-21]` The entry canvas drew `Maintenance` — corpus degree
	 * 87, the most-connected resource in the corpus — in the band captioned *"not connected to
	 * anything else"*, and the accessibility list called it `0 links`. Measured across the whole
	 * band: 26 marks, **every one of them at corpus degree ≥ 11**, because 11 IS the cut.
	 *
	 * Options 1 and 2 (pull in the hub's neighbours; re-rank on induced degree) were measured and
	 * cannot reach these nodes at a drawable size — `Maintenance` needs K=739, all 26 need K=2499.
	 * The repair is therefore to the TELLING, and this is what witnesses it at the render.
	 *
	 * @see internal/superpowers/specs/2026-08-21-hub-stranding-is-a-telling-failure-design.md
	 */
	const entryNode = (id: string, degree: number, title = id.toUpperCase()) => ({
		id,
		title,
		doc_type: 'goal',
		home: 'context' as const,
		degree,
		salience: null,
		excerpt: null,
		stage: null,
		home_id: 'ctx-1',
		updated: '2026-08-20T10:00:00Z',
	});

	// Two connected marks and one stranded hub — the production shape at K=130, in miniature.
	const entryModel = buildEntryGraph(
		{
			nodes: [entryNode('a', 12), entryNode('b', 3), entryNode('c', 87, 'Maintenance')],
			edges: [
				{
					id: 'a-b',
					source: 'a',
					target: 'b',
					edge_kind: 'contains' as const,
					polarity: 'forward' as const,
					label: null,
					weight: 2,
				},
			],
			bounds: { drawn: 3, eligible: 2499, in_scope: 3583, truncated: true },
		},
		new Map([['ctx-1', '@me/temper']]),
	);

	const entryView = () =>
		view({ model: entryModel, question: '', readout: null } as Partial<GraphViewData>);

	it('the caption says what the band IS connected to, not that it is connected to nothing', () => {
		render(GraphPage, { data: entryView() });

		const caption = screen.getByTestId('unconnected-caption').textContent ?? '';
		expect(caption).toBe(
			'1 of these 3 is not connected to anything else drawn here — but it connects to 87 things elsewhere in your corpus.',
		);
		// no-internal-vocabulary-is-load-bearing still governs the chrome.
		for (const word of ['degree', 'orphan', 'node']) {
			expect(caption.toLowerCase()).not.toContain(word);
		}
	});

	it('the accessibility list stops asserting 0 links about a hub', () => {
		render(GraphPage, { data: entryView() });

		const row = screen.getByRole('link', { name: /Maintenance/ }).textContent ?? '';
		expect(row).toContain('0 drawn here · 87 in your corpus');
		expect(row).not.toContain('0 links');
	});

	it('a mark with strokes on screen still reads as links — §5.3 stands where it always did', () => {
		render(GraphPage, { data: entryView() });

		expect(screen.getByRole('link', { name: /^A —/ }).textContent).toContain('1 link');
	});

	/**
	 * D1 — the witness a reader filed, at the render level.
	 *
	 * The complaint was a hover card reading `REACHED — in the places you asked about` on a screen
	 * where the question box was empty, with *Why these* three inches away saying *"No question was
	 * asked."* Both on screen at once, contradicting each other. `[observed on production —
	 * 2026-08-21]` on all 130 cards and on the accessibility list's single heading group.
	 *
	 * It is asserted over the WHOLE rendered entry surface rather than over one string, because
	 * fixing the string is what produced instances two and three. What makes this hold is that
	 * `GraphA11yList`, `NodeRail` and `nodeMeta` no longer know any arm's name — they read the
	 * label off `model.arms`, and the entry read declares its own.
	 *
	 * @see internal/superpowers/specs/2026-08-21-the-handoff-and-the-arm-vocabulary-design.md §1, §2
	 */
	it('asserts NOTHING about a question, anywhere on the unaddressed entry', () => {
		const { container } = render(GraphPage, { data: entryView() });

		const rendered = (container.textContent ?? '').toLowerCase();
		expect(rendered).not.toContain('you asked');
		expect(rendered).not.toContain('asked about');
	});

	it("and the words it does use are the ones this read declared, not another read's", () => {
		const { container } = render(GraphPage, { data: entryView() });

		const headings = armHeadings(container);
		expect(headings).toEqual(
			entryModel.arms
				.filter((a) => entryModel.nodes.some((n) => n.arm === a.key))
				.map((a) => `${a.label} · ${entryModel.nodes.filter((n) => n.arm === a.key).length}`),
		);
		expect(headings).toEqual(['What your work is built around · 3']);
	});

	it('draws NO ring, because one arm across every mark distinguishes nothing', () => {
		const { container } = render(GraphPage, { data: entryView() });

		expect(new Set(entryModel.nodes.map((n) => n.arm)).size).toBe(1);
		expect(container.querySelectorAll('.arm-ring')).toHaveLength(0);
		// The marks themselves are untouched — what was withdrawn is an encoding, not a node.
		expect(container.querySelectorAll('.node-chip')).toHaveLength(entryModel.nodes.length);
	});

	it('and the field is still a place, not a bound — every mark is drawn', () => {
		const { container } = render(GraphPage, { data: entryView() });
		const markClasses = new Set(
			[...container.querySelectorAll('svg g[class]')]
				.map((g) => g.getAttribute('class')?.split(' ')[0])
				.filter((c): c is string => !!c && c !== 'labels'),
		);

		expect([...markClasses].sort()).toEqual(['edge', 'node-chip']);
	});
});

describe('two edges between the same pair do not blank the page', () => {
	/**
	 * `[observed on production — 2026-08-21]` Opening the rail on 5 of the entry read's 130 marks
	 * threw `each_key_duplicate` and rendered a **blank page**. The rail keyed its neighbour list
	 * on `other.id + label + dir`, which drops the kind — and **43 of the 275 edges share a pair**,
	 * because a resource can `relates_to` another AND `derived_from` it. Where two of those
	 * coalesce to the same displayed label, the keys were identical.
	 *
	 * This is not entry-specific: the composition rail could collide the same way. What made it
	 * reachable is that the entry read only started opening its rail in #744, over 130 marks.
	 *
	 * The fixture is the measured shape — same pair, same direction, same effective label,
	 * different kind — not an invented one.
	 */
	/** An `AtlasNode` as the entry read returns them — no `ResourceView` behind it. */
	const pairNode = (id: string, title: string) => ({
		id,
		title,
		doc_type: 'goal',
		home: 'context' as const,
		degree: 2,
		salience: null,
		excerpt: null,
		stage: null,
		home_id: 'ctx-1',
		updated: '2026-08-20T10:00:00Z',
	});

	const paired = buildEntryGraph(
		{
			nodes: [pairNode('a', 'Focus'), pairNode('b', 'Both ways')],
			edges: [
				{
					id: 'e1',
					source: 'a',
					target: 'b',
					edge_kind: 'leads_to' as const,
					polarity: 'forward' as const,
					label: 'supports',
					weight: 1,
				},
				{
					id: 'e2',
					source: 'a',
					target: 'b',
					edge_kind: 'contains' as const,
					polarity: 'forward' as const,
					label: 'supports',
					weight: 1,
				},
			],
			bounds: { drawn: 2, eligible: 2, in_scope: 2, truncated: false },
		},
		new Map([['ctx-1', '@me/temper']]),
	);

	const pairedView = () =>
		view({
			model: paired,
			question: '',
			readout: null,
			selected: 'a',
		} as Partial<GraphViewData>);

	it('opens the rail instead of throwing each_key_duplicate', () => {
		render(GraphPage, { data: pairedView() });

		expect(screen.getByTestId('node-rail')).toBeTruthy();
	});

	it('and lists BOTH — they read alike and are two different edges', () => {
		render(GraphPage, { data: pairedView() });

		const rail = screen.getByTestId('node-rail').textContent ?? '';
		expect(rail).toContain('NEIGHBORS · 2');
		// Not deduped away: all 275 production edges are distinct under the four-field identity,
		// so collapsing them would drop a real relationship rather than a duplicate.
		expect(rail.match(/supports/g)).toHaveLength(2);
	});
});

describe('the ring is withdrawn only where it distinguishes nothing', () => {
	it('an answer with more than one arm still rings — this is not a blanket removal', () => {
		const model = view().model;
		// Guards the test above from passing vacuously: if the flagship ever collapsed to one arm,
		// "no rings on the entry read" would stop being evidence of anything.
		expect(new Set(model.nodes.map((n) => n.arm)).size).toBeGreaterThan(1);

		const { container } = render(GraphPage, { data: view() });
		expect(container.querySelectorAll('.arm-ring').length).toBeGreaterThan(0);
	});

	it('and rings exactly the marks that are not a walk — the encoding is unchanged', () => {
		// Counted WITHOUT consulting `model.arms`, deliberately. The canvas now decides the ring
		// from the read's declaration, so a count taken from that same declaration agrees with the
		// canvas whatever the declaration says — it would pass on a screen ringing everything.
		// This is the figure #741 shipped, stated independently, so a wrong declaration moves it.
		const model = view().model;
		const notWalk = model.nodes.filter((n) => n.arm !== 'walk').length;

		const { container } = render(GraphPage, { data: view() });
		expect(container.querySelectorAll('.arm-ring')).toHaveLength(notWalk);
	});

	it('and the READ declares which arm was reached — the canvas no longer assumes it', () => {
		// The other half, and the half the render assertion above cannot reach. `coreOf` and the
		// ring used to hard-code `!== 'walk'`: a global check about a global enum, which is what no
		// per-view vocabulary could satisfy. The check moved into the read; this pins what it says.
		expect(COMPOSITION_ARMS.map((a) => [a.key, a.reached])).toEqual([
			['seed', false],
			['survey', false],
			['walk', true],
		]);
	});

	it('the composition still names its own three arms — D1 changed no word of it', () => {
		const { container } = render(GraphPage, { data: view() });
		const model = view().model;

		const headings = armHeadings(container);
		expect(headings).toEqual(
			model.arms
				.map((a) => [a, model.nodes.filter((x) => x.arm === a.key).length] as const)
				.filter(([, n]) => n > 0)
				.map(([a, n]) => `${a.label} · ${n}`),
		);
		// This fixture is seeded with no rows of the reader's own, so the `seed` arm is DECLARED and
		// empty — and correctly draws no heading. A legend count would have drawn one, which is why
		// `armsDistinguish` and this list both derive from the marks and not from `model.arms`.
		expect(model.arms.map((a) => a.key)).toContain('seed');
		expect(headings.some((h) => h?.startsWith('In the places you asked about'))).toBe(false);
		// And neither read can reach the other's sentence.
		expect(headings.some((h) => h?.includes('What your work is built around'))).toBe(false);
	});
});
