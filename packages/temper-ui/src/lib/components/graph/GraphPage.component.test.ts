import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { type BoundDeclaration, declareBounds, declareTraversalBounds } from '$lib/graph/bound';
import type { GraphPlan } from '$lib/graph/composition';
import {
	buildEntryGraph,
	buildGraph,
	buildTraversal,
	COMPOSITION_ARMS,
	type GraphModel,
} from '$lib/graph/model';
import { buildReadout, type Readout } from '$lib/graph/readout';
import type { GraphRefusal, GraphViewData } from '$lib/graph/view';
import { describeFailure, GaveUp } from '$lib/server/bounded';
import type { CogmapRegionRow } from '$lib/types/generated/cognitive_maps';
import type { EventTrail } from '$lib/types/generated/element_trail';
import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';
import type { QueryResponse } from '$lib/types/generated/query';
import { resetAppContext, setPage } from '../../../test/app-context';
import { sentenceOf } from '../../../test/sentence';
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

/**
 * Overrides in their **settled** form, for every field the load now streams.
 *
 * A test that only cares about content keeps writing `selectedExcerpt: 'A first paragraph.'` or
 * `model: entryModel`, and this builder wraps it; a test that cares about the *state* of the read
 * passes a promise through untouched — never-settling for arriving, rejected for failed. That is
 * what keeps C1 and C3 expressible here without 45 call sites learning what a promise is.
 */
type TooLittle = Extract<GraphRefusal, { kind: 'too-little-structure' }>;

type ViewOverrides = Partial<
	Omit<
		GraphViewData,
		| 'model'
		| 'bound'
		| 'readout'
		| 'tooLittleStructure'
		| 'selected'
		| 'selectedExcerpt'
		| 'selectedTrail'
	>
> & {
	model?: GraphModel | Promise<GraphModel>;
	bound?: BoundDeclaration | null | Promise<BoundDeclaration | null>;
	readout?: Readout | null | Promise<Readout | null>;
	tooLittleStructure?: TooLittle | null | Promise<TooLittle | null>;
	selected?: string | null | Promise<string | null>;
	selectedExcerpt?: string | null | Promise<string | null>;
	selectedTrail?: EventTrail | null | Promise<EventTrail>;
};

/**
 * The default answer, built once and named — `view().model` is a **promise** now, and a test that
 * wants the marks wants the model rather than the field.
 */
const flagship = buildGraph({ response: fixture.response, plan, seeds: [] });

/**
 * The wrapper for the two RAIL reads, where `null` stays `null`.
 *
 * On `selectedExcerpt` and `selectedTrail` the outer null means **nothing is selected** — decided
 * from the address before any read runs, and a different fact from a read that answered with
 * nothing. See `GraphViewData`'s comment for the three-way distinction.
 */
const streamed = <T>(v: T | null | undefined | Promise<T>): Promise<T> | null =>
	v === null || v === undefined ? null : v instanceof Promise ? v : Promise.resolve(v);

/**
 * The wrapper for the fields whose promise is **unconditional** — `model`, `bound`, `readout`,
 * `selected` and `tooLittleStructure`.
 *
 * `null` becomes `Promise.resolve(null)` here rather than staying `null`, and that is the whole
 * difference from {@link streamed}: on these five the inner null is the only null there is. A test
 * writing `bound: null` means *this answer declared no bounds*, which is a fact about the answer;
 * an outer null would be a fourth state on a field that already carries three.
 */
const always = <T>(v: T | Promise<T>): Promise<T> =>
	v instanceof Promise ? v : Promise.resolve(v);

const view = (over: ViewOverrides = {}): GraphViewData => {
	const {
		model,
		bound,
		readout,
		tooLittleStructure,
		selected,
		selectedExcerpt,
		selectedTrail,
		...settled
	} = over;
	return {
		owner: '@me',
		question: 'what keeps a surface honest about what it left out?',
		borrowedFrom: null,
		refusal: null,
		placesAsked: [
			{ kind: 'context', ref: '@me/temper', title: '@me/temper' },
			{
				kind: 'cogmap',
				ref: '019f2391-e001-7933-b88a-28fb92e56ac1',
				title: 'Temper — self-cognition',
			},
		],
		...settled,
		model: always(model ?? flagship),
		// `=== undefined` rather than `??` on the two fields where a caller passing `null` means it.
		bound: always(bound === undefined ? declareBounds(fixture.response, plan, null) : bound),
		readout: always(
			readout === undefined
				? buildReadout(fixture.response, { rows: fixture.shape_rows, complete: true })
				: readout,
		),
		tooLittleStructure: always(tooLittleStructure ?? null),
		selected: always(selected ?? null),
		selectedExcerpt: streamed(selectedExcerpt),
		selectedTrail: streamed(selectedTrail),
	};
};

/**
 * Render, and wait for the one read to land.
 *
 * `render` is synchronous and returns while the page is still showing its arriving marker — which
 * is C1 from the other side, and the reason this helper exists rather than a `tick()` at forty call
 * sites. The wait is on the accessibility list, which lives in the `{:then}` branch and is drawn
 * for every answer that has marks.
 */
const painted = async (data: GraphViewData) => {
	const rendered = render(GraphPage, { data });
	await vi.waitFor(() => {
		expect(rendered.container.querySelector('.graph-a11y')).not.toBeNull();
	});
	return rendered;
};

/**
 * The sentence the rail's HISTORY region is saying, once it reaches the state named by `marker`.
 *
 * Scoped to the history section on every render: the excerpt is resolved-and-null in these tests, so
 * a rail-wide assertion would be reading that region's words instead of this one's. It unmounts
 * before returning, so two states can be compared without two pages being on screen at once.
 */
const historyOf = async (
	trail: EventTrail | Promise<EventTrail> | Promise<never>,
	marker: string,
): Promise<string> => {
	const { unmount } = await painted(
		view({
			selected: selected().id,
			selectedExcerpt: Promise.resolve(null),
			selectedTrail: trail as EventTrail | Promise<EventTrail>,
		}),
	);
	const section = (await screen.findByTestId('node-rail')).querySelector('.history');
	await vi.waitFor(() => {
		expect(section?.querySelector(`[data-testid="${marker}"]`)).not.toBeNull();
	});
	const region = section?.querySelector(`[data-testid="${marker}"]`);
	unmount();
	return sentenceOf(region);
};

/** Rung 2, which replaces the canvas rather than joining it — so it waits on its own sentence. */
const rungTwo = async (inScope: number) => {
	const rendered = render(GraphPage, {
		data: view({ tooLittleStructure: { kind: 'too-little-structure', inScope } }),
	});
	await screen.findByText(/A graph is not the right view for this yet/);
	return rendered;
};

/** The busiest node in the fixture, so its neighbour list is non-trivial. */
const selected = () => flagship.nodes.reduce((a, b) => (b.degree > a.degree ? b : a));

beforeEach(() => {
	resetAppContext();
	setPage('/graph/@me?q=what+keeps+a+surface+honest', { owner: '@me' });
});

describe('the surface renders the reader’s own material', () => {
	it('draws one mark per node and one per deduped edge, and nothing else', async () => {
		const { container } = await painted(view());
		const model = flagship;

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

	it('draws 101 edges, not the 1,973 via entries that produced them', async () => {
		const { container } = await painted(view());

		expect(flagship.viaEntries).toBe(1973);
		expect(container.querySelectorAll('.edge')).toHaveLength(101);
	});
});

describe('the bound line is on screen and is not dismissible', () => {
	it('renders whether or not the view is partial', async () => {
		await painted(view());

		expect(screen.getByTestId('bound-line').textContent).toMatch(/^Showing /);
	});

	it('states the applied funnel width and the places asked', async () => {
		await painted(view());
		const line = screen.getByTestId('bound-line').textContent ?? '';

		expect(line).toContain('3 groupings per place');
		expect(line).toContain('12 of 12 places');
		expect(line).toContain('more exist');
	});

	it('carries no control that could hide it', async () => {
		const { container } = await painted(view());
		const bound = container.querySelector('[data-testid="bound-line"]');

		expect(bound?.querySelector('button')).toBeNull();
		expect(bound?.querySelector('a')).toBeNull();
	});
});

describe('derived structure appears in exactly one place, and a reader can confirm by looking', () => {
	it('the readout names the groupings the answer drew on', async () => {
		const { container } = await painted(view());
		const why = container.querySelector('.why');

		expect(why?.textContent).toContain('These came from');
		expect(why?.querySelectorAll('.groupings li').length).toBeGreaterThan(0);
	});

	it('no grouping name is ever drawn as a mark', async () => {
		const { container } = await painted(view());
		const labels = [...container.querySelectorAll('svg text')].map((t) => t.textContent ?? '');
		const grouped = [...(container.querySelectorAll('.groupings li') ?? [])].map(
			(li) => li.textContent ?? '',
		);

		for (const g of grouped) {
			expect(labels).not.toContain(g);
		}
	});

	it('the canvas never renders a score or a salience', async () => {
		const { container } = await painted(view());
		const svg = container.querySelector('svg')?.textContent ?? '';

		expect(svg).not.toMatch(/\d\.\d{3,}/);
	});
});

describe('labels are placed, not carpeted', () => {
	it('captions a bounded set and still lists every node accessibly', async () => {
		const { container } = await painted(view());
		const model = flagship;

		const captions = container.querySelectorAll('.labels text');
		expect(captions.length).toBeGreaterThan(0);
		expect(captions.length).toBeLessThan(model.nodes.length);
		// Dropping a caption drops no ROW: every node is still named in the a11y mirror.
		expect(container.querySelectorAll('.graph-a11y li')).toHaveLength(model.nodes.length);
	});
});

describe('a refusal is a refusal, never a widened answer', () => {
	it('names what the reader asked for and offers a way on', async () => {
		// A refusal renders with the page chrome and starts no read, so there is nothing to wait
		// for — which is the contract, stated by the shape of the call.
		render(GraphPage, {
			data: view({ refusal: { kind: 'no-place-resolved', named: 2 }, bound: null, readout: null }),
		});

		expect(screen.getByText(/Nothing to show for those places/)).toBeTruthy();
		expect(screen.getByText(/See everything you can read/)).toBeTruthy();
	});

	it('draws no canvas at all, rather than an empty one that looks like an answer', async () => {
		const { container } = render(GraphPage, {
			data: view({ refusal: { kind: 'nothing-to-ask' }, bound: null, readout: null }),
		});

		expect(container.querySelector('svg')).toBeNull();
		expect(container.querySelector('[data-testid="bound-line"]')).toBeNull();
	});

	it('rung 2 says which instrument is right and hands the reader its door', async () => {
		// Spec §6. An earlier draft had this rung fall back to drawing the recency page — 200 dots
		// and hope. Rejected: dots a reader cannot use are not more honest than a sentence saying
		// the graph is the wrong instrument here, and the sentence is the one that respects
		// `the-unstructured-reader-is-never-worse-off`. Their need is greater, so they get a working
		// door rather than an empty canvas.
		const { container } = await rungTwo(42);

		expect(screen.getByText(/A graph is not the right view for this yet/)).toBeTruthy();
		// It must say how much they HAVE — the reader is not empty-handed, and a sentence implying
		// they are would be a false claim about their corpus.
		expect(screen.getByText(/42 resources/)).toBeTruthy();
		expect(screen.getByText(/Browse them as a list/)).toBeTruthy();
		expect(container.querySelector('svg')).toBeNull();
	});

	it('the rung is VISIBLE — it does not render like the other refusals', async () => {
		// "A reader on rung 2 is looking at a different claim than one on rung 1. Swapping silently
		// is `legibility-is-never-bought-with-silent-omission`."
		await rungTwo(42);
		expect(screen.queryByText(/There is nothing here yet/)).toBeNull();
		expect(screen.queryByText(/Nothing to show for/)).toBeNull();
	});
});

describe('the rail opens on a node — and only on a node', () => {
	it('shows the resource, where it lives, and how it was reached', async () => {
		const node = selected();
		await painted(view({ selected: node.id, selectedExcerpt: 'A first paragraph.' }));
		const rail = screen.getByTestId('node-rail');

		expect(rail.textContent).toContain(node.title);
		expect(rail.textContent).toContain('IN');
		expect(rail.textContent).toContain('HOW');
		// The excerpt is streamed, so it lands a microtask later than the frame around it — which is
		// C1 stated from the other side.
		expect(await screen.findByText('A first paragraph.')).toBeTruthy();
	});

	it('lists neighbours from the graph on screen, with no second read', async () => {
		const node = selected();
		await painted(view({ selected: node.id }));

		expect(screen.getByTestId('node-rail').textContent).toContain(`NEIGHBORS · ${node.degree}`);
	});

	it('offers a way into the resource itself', async () => {
		const node = selected();
		await painted(view({ selected: node.id }));

		expect(screen.getByTestId('view-full-resource').getAttribute('href')).toBe(
			`/vault/r/${node.id}`,
		);
	});

	it('a sel naming a node this answer does not contain opens nothing', async () => {
		// Rather than a panel describing a resource that is not on screen.
		await painted(view({ selected: 'not-in-this-answer' }));

		expect(screen.queryByTestId('node-rail')).toBeNull();
	});
});

/**
 * C1 and C3, on the panel the goal's first reports were filed against.
 *
 * The rail is the case streaming was chosen for: everything it shows except the excerpt and the
 * history comes from `model.nodes`, which is already in `data`. So the frame paints fully populated
 * with exactly two regions arriving, and the contract is that the frame does not wait for them.
 *
 * Spec §5.1 recorded the defect these replace — `trail ? trailModel(trail) : []` collapsed a failed
 * read and a genuinely empty one onto the same rendering, so the third test here is the one that
 * matters most: it is the *failed vs empty* pair the register's negative face had missed.
 */
describe('the rail declares what is still arriving', () => {
	/** Never settles, so a frame that waits for it never paints. */
	const pending = () => new Promise<never>(() => {});

	it('C1: the rail frame and title paint while its reads are still in flight', async () => {
		const node = selected();
		const { container } = await painted(
			view({ selected: node.id, selectedExcerpt: pending(), selectedTrail: pending() }),
		);
		const rail = container.querySelector('[data-testid="node-rail"]');

		expect(rail).not.toBeNull();
		// Both from the model, already held — no read stands between the reader and either.
		expect(rail?.textContent).toContain(node.title);
		expect(rail?.textContent).toContain(`NEIGHBORS · ${node.degree}`);
		expect(rail?.querySelector('[data-testid="region-arriving"]')).not.toBeNull();
	});

	it('C3: a failed trail read says so, and does NOT read as still arriving', async () => {
		const node = selected();
		const failed = Promise.reject(new Error('503'));
		// The global constraint (spec §5.3), in the test too: the `{:catch}` that renders the failure
		// is a different mechanism from the `.catch()` that keeps an unhandled rejection from
		// crashing the process, and having one does not give you the other.
		failed.catch(() => {});

		const { container } = await painted(
			view({
				selected: node.id,
				selectedExcerpt: Promise.resolve(null),
				selectedTrail: failed,
			}),
		);
		const rail = await screen.findByTestId('node-rail');

		await vi.waitFor(() => {
			expect(rail.querySelector('[data-testid="region-failed"]')).not.toBeNull();
		});
		// The perpetual-skeleton bug, asserted directly: a read that will not resolve must stop
		// presenting as one that has not resolved YET.
		expect(rail.querySelector('[data-testid="region-arriving"]')).toBeNull();
		expect(rail.textContent?.toLowerCase()).toContain('history');
		expect(container.querySelectorAll('.node-chip').length).toBeGreaterThan(0);
	});

	it('C4: a trail that came back empty does not present like one that failed', async () => {
		const failed = Promise.reject(new Error('503'));
		failed.catch(() => {});

		const empty = await historyOf({ events: [] } as unknown as EventTrail, 'region-empty');
		const broken = await historyOf(failed, 'region-failed');

		// Differential, per spec §3.3: a difference in the SENTENCE, with the decorative glyph
		// stripped, so neither a redesign of either state nor a one-channel difference satisfies it.
		// Asserted on the sentence rather than on the section because the section also carries the
		// row count, which differs on its own and would keep this green with identical wording.
		expect(empty).not.toBe('');
		expect(empty).not.toBe(broken);
	});

	/**
	 * The refusal reaching a reader — the wiring half of spec §5.4.
	 *
	 * Both rejections are minted by the **production** hook rather than hand-written here, and that
	 * is the point rather than tidiness: `GaveUp` is a class, a class does not survive serialisation,
	 * and a test that rejected with the instance would be asserting a discriminator the browser never
	 * receives. What is thrown here is what `handleError` hands the client runtime.
	 *
	 * That the hook's output really does travel is a separate claim, and it is witnessed in
	 * `bounded.test.ts` against SvelteKit's own serialiser — not here, where both halves share a
	 * process.
	 */
	it('C4: a trail the system gave up on does not present like one that failed', async () => {
		const stoppedFor = Promise.reject(
			describeFailure(new GaveUp('history', 8000), 'Internal Error'),
		);
		stoppedFor.catch(() => {});
		const failed = Promise.reject(describeFailure(new Error('503'), 'Internal Error'));
		failed.catch(() => {});

		const stopped = await historyOf(stoppedFor, 'region-gave-up');
		const broken = await historyOf(failed, 'region-failed');

		// It says which read, and it does not say what a failure says.
		expect(stopped.toLowerCase()).toContain('history');
		expect(stopped).not.toBe(broken);
		// And it is not the perpetual skeleton either: a give-up is over.
		expect(stopped).not.toBe('');
	});
});

/**
 * C1, C3 and C4 on the region this task streams: **the answer itself**.
 *
 * The rail's block above says the same things about the panel; this one says them about the canvas,
 * the bound line and *Why these*, which are three views of a single read and therefore one region
 * with one marker. What the page owes a reader while that read is in flight is everything that does
 * not depend on it — the ask box, the borrowed-charter line, the refusal, the chrome — and it owes
 * them a region that says, in words, that the rest is still coming.
 */
describe('the page declares what its own read is doing', () => {
	/** Never settles, so anything that waits for it never paints. */
	const pending = () => new Promise<GraphModel>(() => {});

	/** Rejects, with §5.3's *other* catch attached — the one that is not the template's. */
	const broken = (): Promise<GraphModel> => {
		const p = Promise.reject(new Error('503'));
		p.catch(() => {});
		return p;
	};

	it('C1: the ask box and the page chrome paint while the model is still in flight', async () => {
		const { container } = render(GraphPage, { data: view({ model: pending() }) });

		// Everything here is decided above the read, so none of it may wait on one.
		expect(container.querySelector('#graph-question')).not.toBeNull();
		expect(screen.getByRole('button', { name: 'Ask' })).toBeTruthy();
		expect(container.querySelector('svg')).toBeNull();
	});

	it('C2: and the region that is waiting says so in words, not only in colour', async () => {
		const { container } = render(GraphPage, { data: view({ model: pending() }) });
		const arriving = container.querySelector('[data-testid="region-arriving"]');

		expect(arriving).not.toBeNull();
		// The sentence, with the decorative glyph stripped — what reaches the accessibility tree.
		expect(sentenceOf(arriving)).toBe('Loading graph…');
	});

	it('one read, one marker: the bound line and Why these wait with the canvas', async () => {
		// They are three views of a single read. Three arriving markers would tell the reader those
		// regions could disagree about whether it answered, and they cannot.
		const { container } = render(GraphPage, { data: view({ model: pending() }) });

		expect(container.querySelectorAll('[data-testid="region-arriving"]')).toHaveLength(1);
		expect(container.querySelector('[data-testid="bound-line"]')).toBeNull();
		expect(container.querySelector('.why')).toBeNull();
	});

	it('C3: a model that will not read says so, and does NOT read as still arriving', async () => {
		const { container } = render(GraphPage, { data: view({ model: broken() }) });

		await vi.waitFor(() => {
			expect(container.querySelector('[data-testid="region-failed"]')).not.toBeNull();
		});
		// The perpetual-skeleton bug, at page scale: a read that will not resolve must stop
		// presenting as one that has not resolved YET.
		expect(container.querySelector('[data-testid="region-arriving"]')).toBeNull();
		// And the reader can still ask something else — the frame is not taken down with the read.
		expect(container.querySelector('#graph-question')).not.toBeNull();
	});

	it('C4: the two do not present alike, and not on one channel', async () => {
		const arriving = render(GraphPage, { data: view({ model: pending() }) });
		const arrivingWords = sentenceOf(
			arriving.container.querySelector('[data-testid="region-arriving"]'),
		);
		arriving.unmount();

		const failed = render(GraphPage, { data: view({ model: broken() }) });
		await vi.waitFor(() => {
			expect(failed.container.querySelector('[data-testid="region-failed"]')).not.toBeNull();
		});
		const failedWords = sentenceOf(failed.container.querySelector('[data-testid="region-failed"]'));

		// Differential, per spec §3.3, and on the SENTENCE with the glyph stripped — so neither a
		// redesign of either state nor a one-channel difference satisfies it.
		expect(arrivingWords).not.toBe('');
		expect(failedWords).not.toBe('');
		expect(arrivingWords).not.toBe(failedWords);
	});

	it('a refusal is the answer, so it renders before the read rather than behind it', async () => {
		// RULING C's load-bearing half, at the render. The two addressed refusals are decided above
		// every read, and a refusal arriving behind a loading marker would be a delay dressed as an
		// answer.
		render(GraphPage, { data: view({ refusal: { kind: 'nothing-to-ask' }, model: pending() }) });

		expect(screen.getByText(/There is nothing here yet/)).toBeTruthy();
		expect(screen.queryByTestId('region-arriving')).toBeNull();
	});
});
/**
 * The line between the two navigations that look alike, and the defect that came of missing it.
 *
 * **Opening a panel changes `sel` and nothing else.** `withGraphSelection` rewrites that one param
 * in place, so the read the load starts asks the same question of the same places from the same
 * seeds — the model coming back is the one already drawn. Taking the marks down for it is not
 * caution about staleness; it is discarding an answer the page is already holding, and the reader
 * watches fifty marks become "Loading graph…" and then the same fifty marks.
 *
 * **Changing `q`, `in` or `from` is the opposite claim.** That read is a DIFFERENT answer, and marks
 * left standing under a question the reader has replaced are a false one. Both cases are witnessed
 * here, because a fix that simply keeps marks across everything passes the first and fails the
 * reader on the second.
 *
 * Each case drives BOTH halves of the navigation in one batch — the address the click built, then
 * the load's answer to it — because that is how SvelteKit lands them, and a page that read them a
 * tick apart would be reading a key that belongs to neither.
 */
describe('a read that cannot change the marks does not take them down', () => {
	/** Never settles — where a fresh navigation's model sits the moment `data` arrives. */
	const pending = () => new Promise<GraphModel>(() => {});

	/** Draw the marks and hand back how many landed, so "still there" is a number and not a guess. */
	const drawn = async (container: HTMLElement): Promise<number> => {
		await vi.waitFor(() => {
			expect(container.querySelectorAll('.node-chip').length).toBeGreaterThan(0);
		});
		return container.querySelectorAll('.node-chip').length;
	};

	it('keeps every mark across a `?sel=` navigation, and still says a read is in flight', async () => {
		const node = selected();
		const { container, rerender } = render(GraphPage, { data: view() });
		const before = await drawn(container);

		setPage(`/graph/@me?q=what+keeps+a+surface+honest&sel=${node.id}`, { owner: '@me' });
		await rerender({ data: view({ model: pending(), selected: node.id }) });

		expect(container.querySelectorAll('.node-chip')).toHaveLength(before);
		// C1 still holds: keeping the marks is not licence to hide the read. The region says in
		// words that something is happening, or the reader is left to wonder whether the click did.
		expect(sentenceOf(container.querySelector('[data-testid="region-arriving"]'))).toBe(
			'Loading graph…',
		);
	});

	it('gives them up when the QUESTION changed, because that answer is a different one', async () => {
		const { container, rerender } = render(GraphPage, { data: view() });
		await drawn(container);

		setPage('/graph/@me?q=what+else+is+in+here', { owner: '@me' });
		await rerender({ data: view({ question: 'what else is in here', model: pending() }) });

		// These marks answer a question the reader replaced. Holding them would not be
		// responsiveness — it would be the surface claiming they are an answer to what was asked.
		expect(container.querySelectorAll('.node-chip')).toHaveLength(0);
		expect(sentenceOf(container.querySelector('[data-testid="region-arriving"]'))).toBe(
			'Loading graph…',
		);
	});

	it('hands the held marks over when the new read lands, and stops saying it is updating', async () => {
		const node = selected();
		const { container, rerender } = render(GraphPage, { data: view() });
		await drawn(container);

		// The same answer re-read, drawn from fewer marks — so "the new one arrived" is witnessable
		// rather than indistinguishable from "the old one was never replaced".
		const fewer: GraphModel = { ...flagship, nodes: flagship.nodes.slice(0, 3), edges: [] };
		setPage(`/graph/@me?q=what+keeps+a+surface+honest&sel=${node.id}`, { owner: '@me' });
		await rerender({ data: view({ model: fewer, selected: node.id }) });

		await vi.waitFor(() => {
			expect(container.querySelectorAll('.node-chip')).toHaveLength(3);
		});
		expect(container.querySelector('[data-testid="region-arriving"]')).toBeNull();
	});
});

describe('the unconnected field is declared rather than scattered', () => {
	it('the fixture has an unconnected population — but NOT the real one, and says which', async () => {
		// **This file cannot witness the ratio the ruling was made on.** The capture's trim rule
		// keeps every survey hit a via entry references, which keeps the CONNECTED hits by
		// construction and only 4 arbitrary unconnected ones per stage. So this reads 10 of 52
		// (19%) where the live response reads 80 of 155 (51%) — recorded in the fixture's own
		// `_trimmed.degree_zero_NOT_witnessable`. Same species as the 101-edge collapse that block
		// already names: a trim preserving one property destroyed another.
		//
		// What IS witnessable here is that the field exists, is populated, and is captioned truly.
		// The population itself is asserted against the wire, not against this file.
		const model = flagship;
		const zero = model.nodes.filter((n) => n.degree === 0);

		expect(zero.length).toBeGreaterThan(0);
		expect(zero.length).toBeLessThan(model.nodes.length);
	});

	it('says how many are unconnected, in the reader’s own words', async () => {
		await painted(view());
		const model = flagship;
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

	it('still draws every one of them — the field is a place, not a bound', async () => {
		const { container } = await painted(view());
		const model = flagship;

		expect(container.querySelectorAll('.node-chip')).toHaveLength(model.nodes.length);
	});

	it('adds no mark class — the field costs nothing from a vocabulary of two', async () => {
		// The guard above already asserts this for the page as a whole; this one names WHY it is
		// re-asserted here, so a future field that reaches for a third mark fails on purpose.
		const { container } = await painted(view());
		const markClasses = new Set(
			[...container.querySelectorAll('svg g[class]')]
				.map((g) => g.getAttribute('class')?.split(' ')[0])
				.filter((c): c is string => !!c && c !== 'labels'),
		);

		expect([...markClasses].sort()).toEqual(['edge', 'node-chip']);
	});

	it('a fully connected answer gets no caption and no field at all', async () => {
		await painted(
			view({ model: { ...flagship, nodes: flagship.nodes.filter((n) => n.degree > 0) } }),
		);
		expect(screen.queryByTestId('unconnected-caption')).toBeNull();
	});
});

describe('the receiver is reachable from the readout, not merely built', () => {
	it('every place the answer drew on links to its own measurements', async () => {
		// `displaced-structure-remains-reachable` says displaced structure *remains available*, and
		// available means the reader can get there without being told a URL. A receiver nothing
		// links to would satisfy the clause's letter and none of its point.
		const { container } = await painted(view());
		const links = [...container.querySelectorAll('[data-testid="measured-links"] a')];

		expect(links).toHaveLength(2);
		expect(links[0].getAttribute('href')).toBe('/graph/@me/analysis?in=ctx%3A%40me%2Ftemper');
		expect(links[1].getAttribute('href')).toBe(
			'/graph/@me/analysis?in=map%3A019f2391-e001-7933-b88a-28fb92e56ac1',
		);
	});

	it('an answer drawn from no place offers no link rather than an empty row', async () => {
		await painted(view({ placesAsked: [] }));
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

	const entryView = () => view({ model: entryModel, question: '', readout: null });

	it('the caption says what the band IS connected to, not that it is connected to nothing', async () => {
		await painted(entryView());

		const caption = screen.getByTestId('unconnected-caption').textContent ?? '';
		expect(caption).toBe(
			'1 of these 3 is not connected to anything else drawn here — but it connects to 87 things elsewhere in your corpus.',
		);
		// no-internal-vocabulary-is-load-bearing still governs the chrome.
		for (const word of ['degree', 'orphan', 'node']) {
			expect(caption.toLowerCase()).not.toContain(word);
		}
	});

	it('the accessibility list stops asserting 0 links about a hub', async () => {
		await painted(entryView());

		const row = screen.getByRole('link', { name: /Maintenance/ }).textContent ?? '';
		expect(row).toContain('0 drawn here · 87 in your corpus');
		expect(row).not.toContain('0 links');
	});

	it('a mark with strokes on screen still reads as links — §5.3 stands where it always did', async () => {
		await painted(entryView());

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
	it('asserts NOTHING about a question, anywhere on the unaddressed entry', async () => {
		const { container } = await painted(entryView());

		const rendered = (container.textContent ?? '').toLowerCase();
		expect(rendered).not.toContain('you asked');
		expect(rendered).not.toContain('asked about');
	});

	it("and the words it does use are the ones this read declared, not another read's", async () => {
		const { container } = await painted(entryView());

		const headings = armHeadings(container);
		expect(headings).toEqual(
			entryModel.arms
				.filter((a) => entryModel.nodes.some((n) => n.arm === a.key))
				.map((a) => `${a.label} · ${entryModel.nodes.filter((n) => n.arm === a.key).length}`),
		);
		expect(headings).toEqual(['What your work is built around · 3']);
	});

	it('draws NO ring, because one arm across every mark distinguishes nothing', async () => {
		const { container } = await painted(entryView());

		expect(new Set(entryModel.nodes.map((n) => n.arm)).size).toBe(1);
		expect(container.querySelectorAll('.arm-ring')).toHaveLength(0);
		// The marks themselves are untouched — what was withdrawn is an encoding, not a node.
		expect(container.querySelectorAll('.node-chip')).toHaveLength(entryModel.nodes.length);
	});

	it('and the field is still a place, not a bound — every mark is drawn', async () => {
		const { container } = await painted(entryView());
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

	const pairedView = () => view({ model: paired, question: '', readout: null, selected: 'a' });

	it('opens the rail instead of throwing each_key_duplicate', async () => {
		await painted(pairedView());

		expect(screen.getByTestId('node-rail')).toBeTruthy();
	});

	it('and lists BOTH — they read alike and are two different edges', async () => {
		await painted(pairedView());

		const rail = screen.getByTestId('node-rail').textContent ?? '';
		expect(rail).toContain('NEIGHBORS · 2');
		// Not deduped away: all 275 production edges are distinct under the four-field identity,
		// so collapsing them would drop a real relationship rather than a duplicate.
		expect(rail.match(/supports/g)).toHaveLength(2);
	});
});

describe('the ring is withdrawn only where it distinguishes nothing', () => {
	it('an answer with more than one arm still rings — this is not a blanket removal', async () => {
		const model = flagship;
		// Guards the test above from passing vacuously: if the flagship ever collapsed to one arm,
		// "no rings on the entry read" would stop being evidence of anything.
		expect(new Set(model.nodes.map((n) => n.arm)).size).toBeGreaterThan(1);

		const { container } = await painted(view());
		expect(container.querySelectorAll('.arm-ring').length).toBeGreaterThan(0);
	});

	it('and rings exactly the marks that are not a walk — the encoding is unchanged', async () => {
		// Counted WITHOUT consulting `model.arms`, deliberately. The canvas now decides the ring
		// from the read's declaration, so a count taken from that same declaration agrees with the
		// canvas whatever the declaration says — it would pass on a screen ringing everything.
		// This is the figure #741 shipped, stated independently, so a wrong declaration moves it.
		const model = flagship;
		const notWalk = model.nodes.filter((n) => n.arm !== 'walk').length;

		const { container } = await painted(view());
		expect(container.querySelectorAll('.arm-ring')).toHaveLength(notWalk);
	});

	it('and the READ declares which arm was reached — the canvas no longer assumes it', async () => {
		// The other half, and the half the render assertion above cannot reach. `coreOf` and the
		// ring used to hard-code `!== 'walk'`: a global check about a global enum, which is what no
		// per-view vocabulary could satisfy. The check moved into the read; this pins what it says.
		expect(COMPOSITION_ARMS.map((a) => [a.key, a.reached])).toEqual([
			['seed', false],
			['survey', false],
			['walk', true],
		]);
	});

	it('the composition still names its own three arms — D1 changed no word of it', async () => {
		const { container } = await painted(view());
		const model = flagship;

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

/**
 * D2 — the traversed view, rendered.
 *
 * **The trap this block is written against.** A traversal declares its own arms *and* its own bound
 * line, so it can validate itself: an expected ring count taken from `model.arms`, or an expected
 * sentence taken from the `BoundDeclaration`, agrees with the canvas whatever either says. The
 * ground used here is the **seed id** — an argument the caller took from the URL, which nothing in
 * the response carries — and hand-written strings.
 */
describe('a traversed view', () => {
	const HOPPED_FROM = 'n-hopped-from';

	const subgraph: AtlasSubgraph = {
		nodes: ['n-a', 'n-b', HOPPED_FROM, 'n-c'].map((id, i) => ({
			id,
			title: id.toUpperCase(),
			doc_type: 'task',
			home: 'context' as const,
			degree: 10 + i,
			salience: null,
			excerpt: null,
			stage: null,
			home_id: 'ctx-1',
			updated: '2026-08-21T10:00:00Z',
		})),
		edges: [
			[HOPPED_FROM, 'n-a'],
			[HOPPED_FROM, 'n-b'],
			['n-a', 'n-c'],
		].map(([source, target]) => ({
			id: `${source}-${target}`,
			source,
			target,
			// `edge_kind` is the stored enum (`express` | `contains` | `leads_to` | `near`); the
			// reader-facing word is `label`. Typing this fixture as `AtlasSubgraph` is what caught
			// the two being conflated here.
			edge_kind: 'leads_to' as const,
			polarity: 'forward' as const,
			label: 'supports',
			weight: 2,
		})),
	};

	const traversed = (over: ViewOverrides = {}): GraphViewData => {
		const model = buildTraversal(subgraph, [HOPPED_FROM], new Map([['ctx-1', '@me/temper']]));
		return view({
			model,
			// The read ran no composition, so there is nothing to explain the marks with.
			readout: null,
			bound: declareTraversalBounds({
				drawn: model.nodes.length,
				from: model.nodes.filter((n) => n.id === HOPPED_FROM).length,
				depth: 1,
			}),
			...over,
		});
	};

	beforeEach(() =>
		setPage(`/graph/@me?q=what+keeps+a+surface+honest&from=${HOPPED_FROM}&depth=1`, {
			owner: '@me',
		}),
	);

	it('rings the mark the reader hopped from, and only that one', async () => {
		// Counted off the DOM; the expected value is the seed id passed to `buildTraversal`, which
		// is the one fact the response does not carry. A canvas that ringed everything, or nothing,
		// moves this number — a count taken from `model.arms` would not.
		const { container } = await painted(traversed());
		const rings = container.querySelectorAll('.arm-ring');

		expect(rings).toHaveLength(1);
		expect(subgraph.nodes).toHaveLength(4);
	});

	it('draws no ring at all when the mark hopped from is not visible to this reader', async () => {
		// The service returns a seed "that reached nothing", so an absent seed was not readable.
		// Every mark is then something the walk reached, and a ring would encode a constant —
		// exactly the defect #741 removed from the entry read.
		const model = buildTraversal(subgraph, ['a-seed-i-cannot-read'], new Map());
		const { container } = await painted(
			traversed({
				model,
				bound: declareTraversalBounds({ drawn: model.nodes.length, from: 0, depth: 1 }),
			}),
		);

		expect(container.querySelectorAll('.arm-ring')).toHaveLength(0);
	});

	it('replaces "Why these" rather than letting the panel disappear', async () => {
		// §7.2 named "disappear" the second-best of three, because it "loses the reader's route back
		// to how they got here." Until D2 the panel rendered only `{#if data.readout}`.
		const { container } = await painted(traversed());
		const panel = container.querySelector('.why');

		expect(panel).not.toBeNull();
		expect(panel?.textContent).not.toContain('Why these');
	});

	it('says the reader STARTED from the question, never that it is still narrowing', async () => {
		// §4: the walk runs over the reader's whole visible corpus, so `q` is provenance and not a
		// filter still in force. "You asked:" in the present tense is the sentence that would imply
		// otherwise, and it is what a composition screen says.
		const panel = (await painted(traversed())).container.querySelector('.why');

		expect(panel?.textContent).toContain('You started from this question');
		expect(panel?.textContent).not.toContain('You asked:');
		// The composition's sentence about the places being the whole answer — false on a hop, and
		// contradicted by the bound line beside it. Observed on production 2026-08-21.
		expect(panel?.textContent).not.toContain('everything in the places you named is the answer');
	});

	it('offers the route back, with the walk taken off the address', async () => {
		const back = (await painted(traversed())).container
			.querySelector('.why a[href*="/graph/"]')
			?.getAttribute('href');

		expect(back).toBe('/graph/@me?q=what+keeps+a+surface+honest');
	});

	it('keeps the per-place measurement links, under a sentence that says what they describe', async () => {
		// `displaced-structure-remains-reachable`: the analysis door has to stay reachable without
		// the reader being told a URL. These describe the GROUNDING, not this screen, so they
		// survive with their sentence changed rather than being dropped with the readout.
		const measured = (await painted(traversed())).container.querySelector(
			'[data-testid="measured-links"]',
		);

		expect(measured).not.toBeNull();
		expect(measured?.textContent).toContain('How your starting places were measured');
		expect(measured?.querySelectorAll('a').length).toBeGreaterThan(0);
	});

	it('reports no stage accounting, because no pipeline ran', async () => {
		const panel = (await painted(traversed())).container.querySelector('.why');

		expect(panel?.textContent).not.toContain('What each step was handed');
	});

	it('draws a bound line describing THIS screen, with nothing borrowed from the composition', async () => {
		// §7.1: the line "must not keep displaying the grounding query's counts — on hop three those
		// describe a screen the reader is no longer looking at." Written as a whole string, so any
		// composition axis leaking back in fails it.
		const { container } = await painted(traversed());

		expect(container.querySelector('.bound')?.textContent?.trim()).toBe(
			'Showing 4 marks · 1 you hopped from · complete within 1 hop · deeper not reported',
		);
	});
});
