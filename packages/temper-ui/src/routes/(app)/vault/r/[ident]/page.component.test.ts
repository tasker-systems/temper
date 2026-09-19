import { render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import { describeFailure, GaveUp, GIVE_UP_AFTER_MS } from '$lib/server/bounded';
import type { ElementEvent, EventTrail } from '$lib/types/generated/element_trail';
import type { GraphEdgeRow, ResourceConnections } from '$lib/types/generated/graph';
import { makeRow } from '../../../../../test/fixtures';
import { sentenceOf } from '../../../../../test/sentence';
import Page from './+page.svelte';
import type { PageData } from './$types';

/**
 * The route with the highest defect density and, until this file, no component test at all.
 *
 * Two of the four defects this branch fixes were live HERE — a failed trail read degrading to
 * `null` and a failed edges read degrading to `[]`, each of which rendered exactly like a resource
 * that genuinely had no history and no connections. That pair is spec §5.1's *failed vs empty*,
 * and it is what the C4 block below exists for.
 *
 * `page.server.test.ts` beside this file witnesses the other half — that the load hands the
 * template promises rather than values. Neither can see what the other sees.
 */

/** Never settles, so anything that waits for it never paints. That is C1, stated structurally. */
const pending = <T>(): Promise<T> => new Promise<T>(() => {});

/**
 * Rejects, with spec §5.3's **other** catch attached — the one that keeps an unhandled rejection
 * from taking the process down, which is a different mechanism from the `{:catch}` that renders
 * the failure. Having one does not give you the other.
 */
const broken = <T>(): Promise<T> => {
	const p = Promise.reject<T>(new Error('503'));
	p.catch(() => {});
	return p;
};

/**
 * A read the system stopped waiting for, in the shape one really arrives in.
 *
 * Minted by the production hook rather than hand-written, and **never** as a `GaveUp` instance:
 * `$lib/region`'s own comment says why — a prototype does not survive SvelteKit serialising a
 * rejected streamed promise, so `error instanceof GaveUp` in a client `{:catch}` is unreachable by
 * construction. A test that rejected with the instance would exercise a path production never
 * takes, and `regionStateFor` would answer `failed` for it.
 */
const gaveUpOn = <T>(label: string): Promise<T> => {
	const p = Promise.reject<T>(
		describeFailure(new GaveUp(label, GIVE_UP_AFTER_MS), 'Internal Error'),
	);
	p.catch(() => {});
	return p;
};

const event = (n: number): ElementEvent => ({
	event_id: `ev-${n}`,
	kind: 'resource.updated',
	actor_entity_id: 'ent-1',
	actor_name: 'Pete',
	occurred_at: '2026-08-20T10:00:00Z',
	confidence: null,
	correlation_id: null,
	payload: {},
});

const trailOf = (count: number): EventTrail => ({
	element_kind: 'node',
	element_id: 'res-1',
	events: Array.from({ length: count }, (_, i) => event(i)),
});

const edge = (n: number): GraphEdgeRow => ({
	edge_id: `edge-${n}`,
	peer_table: 'kb_resources',
	peer_id: `peer-${n}`,
	peer_title: `A peer ${n}`,
	peer_slug: `peer-${n}`,
	edge_kind: 'near',
	polarity: 'forward',
	label: 'relates to',
	direction: 'outgoing',
	weight: 0.5,
	created: '2026-08-20T10:00:00Z',
});

/**
 * The envelope the bounded connections read returns. `truncated` derives from the page
 * exactly as the server's constructor derives it — `returned < total` — so a fixture that
 * hides rows reads truncated for the same reason the wire one does.
 */
const connectionsOf = (
	rows: GraphEdgeRow[],
	total: number = rows.length,
	limit = 50,
): ResourceConnections =>
	// The fixture models the WIRE — JSON, so numbers at runtime — while ts-rs types the i64
	// columns `bigint` at the type level. The double cast is the honest spelling of that
	// known mismatch; the vault-list.test.ts idiom sidesteps it by minting BigInt fixtures,
	// which would make these witnesses lie about what actually crosses the fetch boundary.
	({
		rows,
		total,
		limit,
		returned: rows.length,
		truncated: rows.length < total,
	}) as unknown as ResourceConnections;

/**
 * The four fields this page reads, cast to `PageData` at the one place it is handed over.
 *
 * `PageData` also carries the `(app)` layout's `user`, `profile`, `entitlements` and nav rows,
 * none of which this component touches. Spelling those out here would pin this file to the
 * layout's shape rather than to the page's — and `Pick` keeps the four that matter typed by the
 * real load rather than by a hand-written double.
 */
type Fill = Partial<
	Pick<
		PageData,
		'content' | 'trail' | 'connections' | 'artifacts' | 'mayChange' | 'stateVocabulary'
	>
>;

const RESOURCE = makeRow({
	title: 'The rendering approach',
	doc_type_name: 'design',
	context_name: 'Temper',
	managed_meta: {
		'temper-stage': 'design',
		'temper-mode': null,
		'temper-effort': null,
		'temper-status': null,
		'temper-seq': null,
		'temper-branch': null,
		'temper-pr': null,
		'temper-llm-model': null,
		'temper-llm-run': null,
		'temper-provenance': null,
	},
	open_meta: { owner: 'Pete', priority: 'high' },
});

/** `doc_type` + `temper-stage` + `owner` + `priority` — see `mergeProperties`. */
const PROPERTY_ROWS = 4;

const data = (fill: Fill = {}): PageData =>
	({
		resource: RESOURCE,
		content: fill.content ?? Promise.resolve('# A body\n\nWith a paragraph in it.'),
		trail: fill.trail ?? Promise.resolve(trailOf(2)),
		connections: fill.connections ?? Promise.resolve(connectionsOf([edge(1)])),
		// The default is the shape the overwhelming majority of resources arrive in — owning no
		// artifacts — because every pre-existing assertion in this file is about that page, and
		// the artifacts region's contract is that such a page renders exactly as it always did.
		artifacts: fill.artifacts ?? Promise.resolve([]),
		// The default is the shape a reader WITHOUT change authority gets: nothing offered,
		// and no vocabulary asked for. Every pre-existing assertion in this file is about that
		// reader, so the write arm must leave all of them standing.
		mayChange: fill.mayChange ?? false,
		stateVocabulary: fill.stateVocabulary ?? null,
	}) as PageData;

/**
 * The three regions, each scoped to the element that holds it.
 *
 * Scoped rather than page-wide because every assertion below is about ONE region: a page-wide
 * `querySelector('[data-testid="region-failed"]')` cannot tell the reader's history from their
 * connections, and C3's whole content is that a failed region names itself.
 *
 * The rail's two are addressed by position because the page renders them in order and neither
 * carries a test id — the toggle bar (D-F6) is the rail's first child, then history, then
 * connections, in all four states.
 */
const documentRegion = (c: HTMLElement): Element | null => c.querySelector('.body');
const historyRegion = (c: HTMLElement): Element | null =>
	c.querySelector('.rail')?.children[1] ?? null;
const connectionsRegion = (c: HTMLElement): Element | null =>
	c.querySelector('.rail')?.children[2] ?? null;

type Scope = (c: HTMLElement) => Element | null;

const REGIONS: [name: string, scope: Scope, key: keyof Fill][] = [
	['document', documentRegion, 'content'],
	['history', historyRegion, 'trail'],
	['connections', connectionsRegion, 'connections'],
];

/**
 * The sentence one region is saying once it reaches `testid`, with the decorative glyph stripped.
 *
 * Two things it is careful about, and both come from spec §3.3. It reads the **RegionState's own
 * element** rather than the region around it: `EventHistory` and `EdgeList` put a count in their
 * heading, which differs between empty and failed on its own and would keep a differential green
 * on identical wording. And it compares the **sentence** rather than `textContent`, because the
 * marker glyph is one of the four channels the states differ on — a raw comparison is satisfied by
 * `⊘` vs `⚠` alone, which is the defect the probe already caught once.
 *
 * It unmounts before returning, so two states are never on screen at the same time.
 */
const wordsOf = async (fill: Fill, scope: Scope, testid: string): Promise<string> => {
	const { container, unmount } = render(Page, { data: data(fill), form: null });
	const marker = () => scope(container)?.querySelector(`[data-testid="${testid}"]`) ?? null;
	await vi.waitFor(() => {
		expect(marker()).not.toBeNull();
	});
	const words = sentenceOf(marker());
	unmount();
	return words;
};

describe('C1: the scaffold does not depend on the fill', () => {
	it('paints the masthead, the doc type, the title and the home chip with all three reads in flight', () => {
		const { container } = render(Page, {
			data: data({ content: pending(), trail: pending(), connections: pending() }),
			form: null,
		});

		expect(container.querySelector('.masthead')).not.toBeNull();
		expect(container.querySelector('.eyebrow')?.textContent).toBe('design');
		expect(container.querySelector('.title')?.textContent).toBe('The rendering approach');
		expect(container.querySelector('.chip')?.textContent).toContain('Temper');
	});

	it('paints every property row with all three reads in flight', () => {
		const { container } = render(Page, {
			data: data({ content: pending(), trail: pending(), connections: pending() }),
			form: null,
		});
		const keys = [...container.querySelectorAll('.props dt')].map((dt) => dt.textContent);

		expect(container.querySelectorAll('.props .row')).toHaveLength(PROPERTY_ROWS);
		expect(keys).toEqual(['doc_type', 'temper-stage', 'owner', 'priority']);
	});

	it('shows an arriving marker in each of the three regions, and nowhere else', () => {
		const { container } = render(Page, {
			data: data({ content: pending(), trail: pending(), connections: pending() }),
			form: null,
		});

		for (const [name, scope] of REGIONS) {
			expect(
				scope(container)?.querySelector('[data-testid="region-arriving"]'),
				`${name} is not declaring itself`,
			).not.toBeNull();
		}
		expect(container.querySelectorAll('[data-testid="region-arriving"]')).toHaveLength(
			REGIONS.length,
		);
	});
});

describe('C2: an arriving region declares itself in words', () => {
	it.each([
		['document', documentRegion, 'Loading document…'],
		['history', historyRegion, 'Loading history…'],
		['connections', connectionsRegion, 'Loading connections…'],
	] as [
		string,
		Scope,
		string,
	][])('the arriving %s region carries a sentence, not a bare shimmer', (_name, scope, sentence) => {
		const { container } = render(Page, {
			data: data({ content: pending(), trail: pending(), connections: pending() }),
			form: null,
		});

		// The words that reach the accessibility tree, with the decorative marker removed. An
		// animation with no text is silent to everything except an eye on the pixels.
		expect(sentenceOf(scope(container)?.querySelector('[data-testid="region-arriving"]'))).toBe(
			sentence,
		);
	});
});

describe('C3: a failure is a third state, not a stuck second one', () => {
	it.each(REGIONS)('a failed %s read names that region', async (name, scope, key) => {
		const words = await wordsOf({ [key]: broken() }, scope, 'region-failed');

		// A failed region says WHAT failed. "Something went wrong" leaves the rest of the page
		// untrustworthy because the reader cannot tell which part of it is still true.
		expect(words.toLowerCase()).toContain(name);
		expect(words.toLowerCase()).not.toContain('something went wrong');
	});

	it.each(
		REGIONS,
	)('a failed %s read drops the arriving marker — the perpetual skeleton', async (_name, scope, key) => {
		const { container } = render(Page, { data: data({ [key]: broken() }), form: null });
		await vi.waitFor(() => {
			expect(scope(container)?.querySelector('[data-testid="region-failed"]')).not.toBeNull();
		});

		// A read that will not resolve must stop presenting as one that has not resolved YET.
		expect(scope(container)?.querySelector('[data-testid="region-arriving"]')).toBeNull();
	});

	it('a failed history read leaves the document and the connections arriving on their own', async () => {
		const { container } = render(Page, {
			data: data({ content: pending(), trail: broken(), connections: pending() }),
			form: null,
		});
		await vi.waitFor(() => {
			expect(
				historyRegion(container)?.querySelector('[data-testid="region-failed"]'),
			).not.toBeNull();
		});

		// Three reads, three regions: one failing says nothing about the other two, and the
		// scaffold is not taken down with it.
		expect(
			documentRegion(container)?.querySelector('[data-testid="region-arriving"]'),
		).not.toBeNull();
		expect(
			connectionsRegion(container)?.querySelector('[data-testid="region-arriving"]'),
		).not.toBeNull();
		expect(container.querySelector('.title')?.textContent).toBe('The rendering approach');
	});
});

/**
 * C4 — the pair that was live on this route, in two of its three regions.
 *
 * `empty` asserts *there is nothing here*, which is a claim about the reader's own material.
 * A failed read has verified no such thing. Rendering them alike is the register's negative face
 * failing, and it is what the load's `null` / `[]` degradations used to do.
 */
describe('C4: an empty region does not present like a failed one', () => {
	it.each(
		REGIONS,
	)('%s: what came back empty says something different from what did not come back', async (_name, scope, key) => {
		const emptyValue: Fill = {
			content: Promise.resolve(''),
			trail: Promise.resolve(trailOf(0)),
			connections: Promise.resolve(connectionsOf([])),
		};

		const empty = await wordsOf({ [key]: emptyValue[key] }, scope, 'region-empty');
		const failed = await wordsOf({ [key]: broken() }, scope, 'region-failed');

		expect(empty).not.toBe('');
		expect(failed).not.toBe('');
		expect(empty).not.toBe(failed);
	});
});

/**
 * The refusal (spec §5.4) reaching a reader on this route.
 *
 * `bounded` wraps all three of this load's reads, so a give-up is what any of them presents as
 * after 8 seconds — and it is not a fault. It must not borrow the failure's words.
 */
describe('a read the system stopped waiting for is not a read that failed', () => {
	it('the give-up names the region it stopped waiting for', async () => {
		const stopped = await wordsOf({ trail: gaveUpOn('history') }, historyRegion, 'region-gave-up');

		expect(stopped.toLowerCase()).toContain('history');
		expect(stopped.toLowerCase()).toContain('stopped waiting');
	});

	it('the give-up does not present like a plain failure', async () => {
		const stopped = await wordsOf({ trail: gaveUpOn('history') }, historyRegion, 'region-gave-up');
		const failed = await wordsOf({ trail: broken() }, historyRegion, 'region-failed');

		expect(stopped).not.toBe('');
		expect(stopped).not.toBe(failed);
	});
});

/**
 * The two rail components' own emptiness verdicts.
 *
 * Neither `{:then}` branch in the page tests for emptiness — each component owns that verdict —
 * so these witness the components through the page rather than in isolation, which is the wiring
 * this file is for.
 */
describe('the rail states its emptiness rather than rendering nothing', () => {
	it('EdgeList: a resource with no connections says so', async () => {
		const { container } = render(Page, {
			data: data({ connections: Promise.resolve(connectionsOf([])) }),
			form: null,
		});
		const region = () => connectionsRegion(container);
		await vi.waitFor(() => {
			expect(region()?.querySelector('[data-testid="region-empty"]')).not.toBeNull();
		});

		// It used to render NOTHING here — an absence that says nothing, and one a failed read
		// degrading to `[]` produced identically.
		expect(region()?.querySelector('.edge')).toBeNull();
		expect(sentenceOf(region()?.querySelector('[data-testid="region-empty"]'))).toBe(
			'No connections.',
		);
		// Chrome at zero too: `0 of 0` is the same bound line, stating completeness rather
		// than leaving it to be inferred from the absence of a truncation marker.
		expect(region()?.querySelector('.label')?.textContent).toBe('Connections · 0 of 0');
	});

	it('EventHistory: a resource with no history says so, in the shared vocabulary', async () => {
		const { container } = render(Page, {
			data: data({ trail: Promise.resolve(trailOf(0)) }),
			form: null,
		});
		const region = () => historyRegion(container);
		await vi.waitFor(() => {
			expect(region()?.querySelector('[data-testid="region-empty"]')).not.toBeNull();
		});

		// The verdict is `EventHistory`'s, not the page's: the heading carrying the count is the
		// component's own, so the emptiness marker beside it came from `trailModel(trail).length`
		// rather than from a second predicate in the template.
		//
		// **What this cannot witness**, recorded rather than banked: `trailModel` currently filters
		// nothing — it reverses and maps 1:1 — so a trail whose events all filter out is not a
		// reachable state today. If it ever gains a filter, this is the assertion that already
		// covers it, because the count and the verdict come from the same call.
		expect(region()?.querySelector('.label')?.textContent).toBe('History · 0');
		expect(sentenceOf(region()?.querySelector('[data-testid="region-empty"]'))).toBe('No history.');
	});

	it('EventHistory: a trail that carries events renders them and no emptiness verdict', async () => {
		const { container } = render(Page, {
			data: data({ trail: Promise.resolve(trailOf(2)) }),
			form: null,
		});
		const region = () => historyRegion(container);
		await vi.waitFor(() => {
			expect(region()?.querySelector('.event')).not.toBeNull();
		});

		expect(region()?.querySelectorAll('.event')).toHaveLength(2);
		expect(region()?.querySelector('[data-testid="region-empty"]')).toBeNull();
		expect(region()?.querySelector('.label')?.textContent).toBe('History · 2');
	});
});

/**
 * D-F3 — the row states the relationship in the reader's terms, or states nothing.
 *
 * `label` is the reader's own word for their relationship; `edge_kind` is the system's structural
 * name for it. An edge whose label is empty (the type is non-null and the wire `COALESCE`s the
 * column to `''`) used to fall back to `edge_kind`, so a row rendered the system's vocabulary —
 * 'near', 'contains' — as if the reader had written it. Weight and polarity are ledger metadata:
 * they name how the edge was asserted, not what the reader is looking at, and they rendered on
 * every row regardless.
 */
describe('a connection row reads in the reader’s terms', () => {
	/** The unlabeled edge the system actually ships: `label` arrives `''`, never null. */
	const unlabeled = (n: number): GraphEdgeRow => ({ ...edge(n), label: '', polarity: 'inverse' });

	const rowWords = async (fill: Fill): Promise<string> => {
		const { container, unmount } = render(Page, { data: data(fill), form: null });
		await vi.waitFor(() => {
			expect(connectionsRegion(container)?.querySelector('.edge')).not.toBeNull();
		});
		const words = connectionsRegion(container)?.textContent ?? '';
		unmount();
		return words;
	};

	it('an unlabeled edge states no relationship text — not the system’s edge kind', async () => {
		const words = await rowWords({ connections: Promise.resolve(connectionsOf([unlabeled(1)])) });

		// 'near' is the fixture's `edge_kind`. Before D-F3 the fallback rendered it; a version
		// that renders it again — or any placeholder prose in its place — fails here.
		expect(words).not.toContain('near');
	});

	it('weight and polarity appear on no row', async () => {
		const words = await rowWords({ connections: Promise.resolve(connectionsOf([unlabeled(1)])) });

		// The fixture carries `weight: 0.5` and `polarity: 'inverse'`; the current template
		// renders both, so this bites only because the fixture polarity is non-forward.
		expect(words).not.toContain('0.5');
		expect(words).not.toContain('inverse');
	});

	it('a labeled edge renders its label verbatim — no translation table', async () => {
		const words = await rowWords({ connections: Promise.resolve(connectionsOf([edge(1)])) });

		expect(words).toContain('relates to');
	});

	// An unlabeled row renders no relationship text at all, arrows included — the whole
	// arrow-label-arrow span sits inside the label guard — so both arrows here come from the
	// labeled rows. The assertion keeps the arrow arm alive without reading the unlabeled
	// row's silence as the arrows being gone.
	it('direction arrows stay on labeled rows — incoming and outgoing', async () => {
		const incoming = (n: number): GraphEdgeRow => ({ ...edge(n), direction: 'incoming' });
		const words = await rowWords({
			connections: Promise.resolve(connectionsOf([edge(1), unlabeled(2), incoming(3)])),
		});

		expect(words).toContain('→');
		expect(words).toContain('←');
	});

	it('a blob peer states its peer and never links into /vault/r', async () => {
		const blobPeer = (n: number): GraphEdgeRow => ({
			...edge(n),
			peer_table: 'kb_blobs',
			peer_id: '01jabcdefghij00000000000k'.padEnd(26, '0'),
		});
		const { container, unmount } = render(Page, {
			data: data({ connections: Promise.resolve(connectionsOf([blobPeer(1)])) }),
			form: null,
		});
		await vi.waitFor(() => {
			expect(connectionsRegion(container)?.querySelector('.edge')).not.toBeNull();
		});
		const peer = connectionsRegion(container)?.querySelector('.edge .peer');

		expect(peer?.tagName).toBe('SPAN');
		expect(peer?.textContent).toContain('blob · 01jabcde');
		unmount();
	});
});

/**
 * D-F5 — the render bound is stated chrome, not a silent slice.
 *
 * The region renders the most recent 50 events of whatever the trail read returned; the read is
 * whole, so the true full count is known and stated in the heading. The bound between "read" and
 * "rendered" used to sit nowhere on screen: a resource with 200 events said "History · 200" and
 * then stopped without explanation, which reads as a rendering defect rather than as a stated
 * bound. The statement is chrome — present whether or not the slice actually bit — because a
 * warning that appears only when the bound binds is a second predicate that can drift away from
 * the slice it describes.
 */
describe('the history region states its render bound', () => {
	const historyWords = async (fill: Fill): Promise<string> => {
		const { container, unmount } = render(Page, { data: data(fill), form: null });
		await vi.waitFor(() => {
			expect(historyRegion(container)?.querySelector('.event')).not.toBeNull();
		});
		const words = historyRegion(container)?.textContent ?? '';
		unmount();
		return words;
	};

	it('a trail beyond the bound states the most recent 50 of the true full count', async () => {
		const words = await historyWords({ trail: Promise.resolve(trailOf(60)) });

		expect(words).toContain('most recent 50 of 60');
	});

	it('the bound statement is present at or under the bound — chrome, not a warning', async () => {
		const words = await historyWords({ trail: Promise.resolve(trailOf(2)) });

		expect(words).toContain('most recent 2 of 2');
	});

	it('the slice still renders at most 50 events, and the heading keeps the true full count', async () => {
		const { container, unmount } = render(Page, {
			data: data({ trail: Promise.resolve(trailOf(60)) }),
			form: null,
		});
		const region = () => historyRegion(container);
		await vi.waitFor(() => {
			expect(region()?.querySelector('.event')).not.toBeNull();
		});

		expect(region()?.querySelectorAll('.event')).toHaveLength(50);
		expect(region()?.querySelector('.label')?.textContent).toBe('History · 60');
		unmount();
	});
});

/**
 * D-F6 — the rail closes, and the closed state says what it withholds.
 *
 * The rail carried History and Connections unconditionally: there was no way to put them away,
 * and the only way to stop seeing them was to stop rendering them, which would be the exact
 * failed-vs-empty absence this page spent a whole block curing. So closed is a STATE — the rail
 * stays on screen, names the two regions it is withholding, and offers them back — rather than a
 * rail-shaped hole. The main column is untouched by it: nothing the reader was reading reflows
 * because they put the rail away.
 *
 * This is presentation state in the page (an in-page `$state` toggle, the `openEvent` precedent
 * in `EventHistory`), not address state: no URL carries it, and a fresh page arrives open.
 */
describe('the rail closes and says what it withholds', () => {
	const toggle = (c: HTMLElement) => c.querySelector<HTMLButtonElement>('.rail-toggle');

	const openRail = async () => {
		const { container, unmount } = render(Page, { data: data(), form: null });
		await vi.waitFor(() => {
			expect(historyRegion(container)?.querySelector('.event')).not.toBeNull();
		});
		return { container, unmount };
	};

	/** Closes the rail and proves the close happened, so no assertion below reads the open rail. */
	const closedRail = async () => {
		const { container, unmount } = await openRail();
		toggle(container)?.click();
		// Svelte 5 flushes the handler's DOM update on a microtask, so the close is awaited
		// rather than assumed — the same reason `wordsOf` waits on its marker.
		await vi.waitFor(() => {
			expect(container.querySelector('.event'), 'the rail never closed').toBeNull();
		});
		return { container, unmount };
	};

	it('a control in the rail closes it', async () => {
		const { container, unmount } = await openRail();

		expect(toggle(container), 'the rail offers no control to close itself').not.toBeNull();
		toggle(container)?.click();

		await vi.waitFor(() => {
			expect(container.querySelector('.rail')).not.toBeNull();
			expect(container.querySelector('.event')).toBeNull();
			expect(container.querySelector('.edge')).toBeNull();
		});
		unmount();
	});

	it('the closed state names the two regions it withholds', async () => {
		const { container, unmount } = await closedRail();

		const words = container.querySelector('.rail')?.textContent ?? '';
		expect(words).toContain('History');
		expect(words).toContain('Connections');
		unmount();
	});

	it('closed is a state, not an absence — the rail reopens', async () => {
		const { container, unmount } = await closedRail();

		toggle(container)?.click();
		await vi.waitFor(() => {
			expect(historyRegion(container)?.querySelector('.event')).not.toBeNull();
		});
		expect(container.querySelector('.rail')?.querySelector('.event')).not.toBeNull();
		unmount();
	});

	it('closing the rail leaves the main column untouched', async () => {
		const { container, unmount } = await closedRail();

		expect(container.querySelector('.title')?.textContent).toBe('The rendering approach');
		expect(container.querySelectorAll('.props .row')).toHaveLength(PROPERTY_ROWS);
		expect(container.querySelector('.body')).not.toBeNull();
		unmount();
	});
});

/**
 * D-F1 — the walk affordance is a deep link into the traversal door, not a read of it.
 *
 * The rail offered no walk control before this: the question "what is near this, walked out
 * from here?" needed the reader to open the graph surface and re-describe the thing they were
 * looking at. The control composes the traversal address with the viewed resource as the `from`
 * seed, through the one grammar author (`graphHref`) — no address is formatted on this page.
 *
 * `no-affordance-overstates-what-it-does` bites at the words/address join: the rendered words
 * and the target address must agree, so both are asserted EXACTLY — the words carry only the
 * walk (what the graph screen draws there belongs to the graph register), and the address
 * carries only the seed (depth is grammar-only, ruled 2026-08-21 — its default lives in the
 * read, and a `depth` here would claim a read the address did not ask for). A control that
 * navigates anywhere but `/graph/@me?from=<id>` fails here.
 */
describe('the walk control deep-links the traversal door with the thing in front of the reader', () => {
	/** A second, kind-distinct resource uuid — full, never a prefix (a prefix resolves to nothing). */
	const OTHER_ROW_ID = '019f420c-cf01-7bc1-87c9-09684b0fa69f';

	const walkControl = (c: HTMLElement): HTMLAnchorElement | null =>
		c.querySelector<HTMLAnchorElement>('.rail-bar .walk');

	it('the rail offers a walk control, and its words claim exactly the walk its address builds', async () => {
		// The pre-change shape had no walk control at all — every assertion in this block fails
		// against it by construction.
		const { container, unmount } = render(Page, { data: data(), form: null });
		const walk = walkControl(container);

		expect(walk, 'the rail offers no walk control').not.toBeNull();
		// Exact words: any copy that promises what the graph will draw, how far it reaches, or
		// what kind of question it answers overstates what this control does.
		expect(walk?.textContent).toBe('Walk out from here');
		// Exact address: the traversal door, seeded with the viewed resource, and nothing else.
		expect(walk?.getAttribute('href')).toBe(`/graph/@me?from=${RESOURCE.id}`);
		unmount();
	});

	it('the address carries the viewed resource as its only seed — no depth, nothing else', async () => {
		const { container, unmount } = render(Page, { data: data(), form: null });
		const href = walkControl(container)?.getAttribute('href') ?? '';
		const url = new URL(href, 'http://ui.test');

		// The seed is the thing in front of the reader — the viewed resource's own uuid, so the
		// reader describes nothing.
		expect(url.pathname).toBe('/graph/@me');
		expect(url.searchParams.getAll('from')).toEqual([RESOURCE.id]);
		// Depth is grammar-only (2026-08-21): the default lives in the read, so the emission
		// must not carry it.
		expect(url.searchParams.get('depth')).toBeNull();
		// And no second parameter rides along: no `in`, `q` or `sel` — the walk asks only
		// "out from here".
		expect([...url.searchParams.keys()]).toEqual(['from']);
		unmount();
	});

	it('the control is offered with the rail open and with it closed', async () => {
		// It is an affordance, not a region: closing withholds History and Connections, and the
		// closed state's copy stays exactly true because the walk is neither.
		const { container, unmount } = render(Page, { data: data(), form: null });
		await vi.waitFor(() => {
			expect(historyRegion(container)?.querySelector('.event')).not.toBeNull();
		});
		expect(walkControl(container)).not.toBeNull();

		container.querySelector<HTMLButtonElement>('.rail-toggle')?.click();
		await vi.waitFor(() => {
			expect(container.querySelector('.event')).toBeNull();
		});

		const walk = walkControl(container);
		expect(walk, 'the walk control vanished with the regions it never belonged to').not.toBeNull();
		expect(walk?.getAttribute('href')).toBe(`/graph/@me?from=${RESOURCE.id}`);
		expect(container.querySelector('.rail-closed')?.textContent).toContain(
			'History and Connections are withheld',
		);
		unmount();
	});

	it('the seed is whatever resource the view is serving — the control does not branch on kind', async () => {
		// The resource view only ever serves `kb_resources` rows, and seeds ARE `kb_resources`
		// rows (the grammar's durable kind), so there is no kind the control must exclude: one
		// unconditional control is the whole answer. This render is a task-typed row; the
		// default fixture above is a design.
		const other = makeRow({ id: OTHER_ROW_ID, title: 'A task, not a design' });
		const { container, unmount } = render(Page, {
			data: { ...data(), resource: other } as PageData,
			form: null,
		});

		expect(walkControl(container)?.getAttribute('href')).toBe(`/graph/@me?from=${OTHER_ROW_ID}`);
		unmount();
	});
});

/**
 * `NodeRail.svelte:84-86` states this as a rule rather than a layout preference: *"The label sits
 * OUTSIDE the await."* Two sibling surfaces, one stated rule, and until now it was applied on one
 * of them — this rail dropped its heading for exactly the two states that most need naming.
 *
 * A region that vanishes while it is arriving cannot say that it is arriving.
 */
describe('a rail region keeps its heading in every state', () => {
	/** Each state, with the selector that says the region has reached it. */
	const stateOf = (
		key: 'trail' | 'connections',
	): [state: string, fill: Fill, settledOn: string][] => {
		const present: Fill =
			key === 'trail'
				? { trail: Promise.resolve(trailOf(2)) }
				: { connections: Promise.resolve(connectionsOf([edge(1)])) };
		const empty: Fill =
			key === 'trail'
				? { trail: Promise.resolve(trailOf(0)) }
				: { connections: Promise.resolve(connectionsOf([])) };
		return [
			['arriving', { [key]: pending() }, '[data-testid="region-arriving"]'],
			['present', present, key === 'trail' ? '.event' : '.edge'],
			['empty', empty, '[data-testid="region-empty"]'],
			['failed', { [key]: broken() }, '[data-testid="region-failed"]'],
		];
	};

	it.each([
		['history', historyRegion, 'trail', 'History'],
		['connections', connectionsRegion, 'connections', 'Connections'],
	] as [
		string,
		Scope,
		'trail' | 'connections',
		string,
	][])('the %s heading is present while arriving, present, empty and failed', async (_name, scope, key, heading) => {
		for (const [state, fill, settledOn] of stateOf(key)) {
			const { container, unmount } = render(Page, { data: data(fill), form: null });
			await vi.waitFor(() => {
				expect(scope(container)?.querySelector(settledOn), `${state} never settled`).not.toBeNull();
			});

			// `?? '(no heading at all)'` so the red reads as the defect rather than as a type
			// complaint about `undefined` — the absence IS what this test is about.
			expect(
				scope(container)?.querySelector('.label')?.textContent ?? '(no heading at all)',
				`the ${state} region must still name itself`,
			).toContain(heading);
			unmount();
		}
	});
});

/**
 * The property table, offering a change.
 *
 * The load decides *whether* and *what* — `page.server.test.ts` witnesses that. What is here is
 * what the reader can actually reach: the control's shape, and the acts that do and do not
 * write.
 */
describe('a state the system defines is changed where it is read', () => {
	const TASK_STATES = { 'temper-stage': ['backlog', 'design', 'done'] };

	const stageCell = (container: HTMLElement) =>
		[...container.querySelectorAll('.props .row')].find(
			(row) => row.querySelector('dt')?.textContent === 'temper-stage',
		);

	it('offers no control at all to a reader who may not change this', async () => {
		// The default fixture is that reader. The read view must be exactly what it was before
		// this arm existed — no controls, no placeholder rows for states not held, and none of
		// the arm's explanatory notes.
		//
		// The resource carries a STRUCTURED description on purpose. Without one, the assertion
		// that no "not editable here" note appears cannot fail: `hasStructured` would be false
		// for want of a structured row rather than for want of write authority, and the probe
		// would pass against a version that leaked the note to every reader.
		const readOnlyReader = makeRow({
			title: 'The rendering approach',
			doc_type_name: 'design',
			open_meta: { owner: 'Pete', tags: ['a', 'b'] },
		});
		const { container } = render(Page, {
			data: {
				...data({ content: pending(), trail: pending(), connections: pending() }),
				resource: readOnlyReader,
			} as PageData,
			form: null,
		});
		expect(container.querySelector('.props select')).toBeNull();
		expect(container.querySelector('.props form')).toBeNull();
		expect([...container.querySelectorAll('.props dt')].map((dt) => dt.textContent)).toContain(
			'tags',
		);
		expect(container.querySelector('.props .unread')).toBeNull();
	});

	it('renders a refusal that belongs to no row against the table itself', async () => {
		// `field` is a three-way address: a key names its row, `''` names the attach form, and
		// `null` names the whole table (a submission carrying nothing to change). Only the first
		// two were rendered by any test, so the third branch shipped unexercised.
		const { container } = render(Page, {
			data: data({
				content: pending(),
				trail: pending(),
				connections: pending(),
				mayChange: true,
				stateVocabulary: {},
			}),
			form: { field: null, message: 'Nothing to change.' },
		});
		expect(container.querySelector('.props .row .err')).toBeNull();
		expect(sentenceOf(container.querySelector('.props > .unread[role="alert"]'))).toBe(
			'Nothing to change.',
		);
	});

	it('offers exactly the states the work carries, and reaches them by POST', async () => {
		const { container } = render(Page, {
			data: data({
				content: pending(),
				trail: pending(),
				connections: pending(),
				mayChange: true,
				stateVocabulary: TASK_STATES,
			}),
			form: null,
		});

		const cell = stageCell(container);
		const select = cell?.querySelector('select') as HTMLSelectElement;
		expect([...select.options].map((o) => o.value)).toEqual(['backlog', 'design', 'done']);
		expect(select.value).toBe('design');

		// `no-reading-act-becomes-a-changing-one` starts here: the change is a POST to a named
		// action. Nothing about it is reachable by following a link or loading a URL.
		const form = cell?.querySelector('form') as HTMLFormElement;
		expect(form.getAttribute('method')?.toLowerCase()).toBe('post');
		expect(form.getAttribute('action')).toBe('?/changeState');
		expect(form.querySelector('input[name="field"]')?.getAttribute('value')).toBe('temper-stage');
	});

	it('does not write when the reader merely moves through the options', async () => {
		// THE BITE for `no-reading-act-becomes-a-changing-one`, and it is not hypothetical: a
		// `<select>` wired to submit on `change` fires per option as a keyboard reader arrows
		// past, so looking at what the states ARE would write two of them into the ledger.
		const submitted = vi.fn();
		const original = HTMLFormElement.prototype.requestSubmit;
		HTMLFormElement.prototype.requestSubmit = submitted;
		try {
			const { container } = render(Page, {
				data: data({
					content: pending(),
					trail: pending(),
					connections: pending(),
					mayChange: true,
					stateVocabulary: TASK_STATES,
				}),
				form: null,
			});
			const select = stageCell(container)?.querySelector('select') as HTMLSelectElement;
			select.value = 'done';
			select.dispatchEvent(new Event('change', { bubbles: true }));
			select.dispatchEvent(new Event('input', { bubbles: true }));
			expect(submitted).not.toHaveBeenCalled();
		} finally {
			HTMLFormElement.prototype.requestSubmit = original;
		}
	});

	it('will not spend a write restating the value already stored', async () => {
		const { container } = render(Page, {
			data: data({
				content: pending(),
				trail: pending(),
				connections: pending(),
				mayChange: true,
				stateVocabulary: TASK_STATES,
			}),
			form: null,
		});
		const cell = stageCell(container);
		expect((cell?.querySelector('button') as HTMLButtonElement).disabled).toBe(true);
	});

	it('offers a state the work carries that this resource has not got', async () => {
		const { container } = render(Page, {
			data: data({
				content: pending(),
				trail: pending(),
				connections: pending(),
				mayChange: true,
				stateVocabulary: { ...TASK_STATES, 'temper-mode': ['plan', 'build'] },
			}),
			form: null,
		});
		const keys = [...container.querySelectorAll('.props dt')].map((dt) => dt.textContent);
		expect(keys).toContain('temper-mode');
		// Unset, and it cannot be set back to nothing: no door retracts a managed property, so
		// the placeholder is not a selectable value.
		const mode = [...container.querySelectorAll('.props .row')].find(
			(row) => row.querySelector('dt')?.textContent === 'temper-mode',
		);
		const placeholder = mode?.querySelector('option[value=""]') as HTMLOptionElement;
		expect(placeholder.disabled).toBe(true);
	});

	it('says it could not check, rather than quietly offering nothing', async () => {
		// `no-partial-view-reads-as-complete`. A reader who may change this resource and is
		// shown no controls must be able to tell "this kind carries no states" from "nobody
		// asked" — the two are the same silence otherwise.
		const { container } = render(Page, {
			data: data({
				content: pending(),
				trail: pending(),
				connections: pending(),
				mayChange: true,
				stateVocabulary: null,
			}),
			form: null,
		});
		expect(container.querySelector('.props select')).toBeNull();
		expect(sentenceOf(container.querySelector('.props .unread'))).toBe(
			'Could not read which states this kind of work carries, so none are offered.',
		);
	});

	it('shows a refusal beside the state it was refused for', async () => {
		const { container } = render(Page, {
			data: data({
				content: pending(),
				trail: pending(),
				connections: pending(),
				mayChange: true,
				stateVocabulary: TASK_STATES,
			}),
			form: { field: 'temper-stage', message: 'You may not change this resource.' },
		});
		expect(sentenceOf(stageCell(container)?.querySelector('.err'))).toBe(
			'You may not change this resource.',
		);
	});
});

/**
 * The description arm, as the reader reaches it.
 *
 * The fixture's open tier is `{ owner: 'Pete', priority: 'high' }` — both single values, so
 * both are offered. The structured case gets its own render.
 */
describe('a reader attaches and revises their own descriptions where they read them', () => {
	const offering = {
		content: pending<string>(),
		trail: pending<never>(),
		connections: pending<never>(),
		mayChange: true,
		stateVocabulary: {},
	};

	const cellFor = (container: HTMLElement, key: string) =>
		[...container.querySelectorAll('.props .row')].find(
			(row) => row.querySelector('dt')?.textContent === key,
		);

	it('offers no description control to a reader who may not change this', () => {
		const { container } = render(Page, {
			data: data({ content: pending(), trail: pending(), connections: pending() }),
			form: null,
		});
		expect(container.querySelector('.props input[type="text"]')).toBeNull();
		expect(container.querySelector('.attach')).toBeNull();
	});

	it('offers revision on a description, by POST to its own action', () => {
		const { container } = render(Page, { data: data(offering), form: null });
		const cell = cellFor(container, 'owner');
		const input = cell?.querySelector('input[name="value"]') as HTMLInputElement;
		expect(input.value).toBe('Pete');
		const form = cell?.querySelector('form') as HTMLFormElement;
		// A DIFFERENT action from the state arm. They share a storage layer and nothing else.
		expect(form.getAttribute('action')).toBe('?/changeDescription');
		expect(form.getAttribute('method')?.toLowerCase()).toBe('post');
		expect((cell?.querySelector('button') as HTMLButtonElement).disabled).toBe(true);
	});

	it('offers attaching a description the system has no field for', () => {
		const { container } = render(Page, { data: data(offering), form: null });
		const attach = container.querySelector('.attach') as HTMLFormElement;
		expect(attach.getAttribute('action')).toBe('?/attachDescription');
		expect(attach.querySelector('input[name="name"]')).not.toBeNull();
		expect(attach.querySelector('input[name="value"]')).not.toBeNull();
	});

	it('offers nothing on a structured description, and says so', () => {
		// The register's exclusion, made visible. `tags` is the commonest open key there is; a
		// reader who finds no control on it and no explanation reads it as a bug.
		const withList = makeRow({
			title: 'The rendering approach',
			doc_type_name: 'design',
			open_meta: { owner: 'Pete', tags: ['a', 'b'] },
		});
		const { container } = render(Page, {
			data: { ...data(offering), resource: withList } as PageData,
			form: null,
		});
		expect(cellFor(container, 'tags')?.querySelector('input')).toBeNull();
		expect(cellFor(container, 'owner')?.querySelector('input')).not.toBeNull();
		expect([...container.querySelectorAll('.props .unread')].map((p) => sentenceOf(p))).toContain(
			'Descriptions holding lists or nested values are not editable here.',
		);
	});

	it('never offers a free-text control on a state the system defines', () => {
		// The rejected equivalence, at the surface: the state arm validates against a closed
		// vocabulary and this one cannot, so free text must never reach a managed key.
		//
		// `temper-branch` is here on purpose. It is a managed key with NO enum — most of them
		// are — so a version of this that only checked the validated key would pass while free
		// text reached every unvalidated one. The reason free text is wrong here is that the key
		// belongs to the system, not that this particular one happens to be checked.
		const withUnvalidatedState = makeRow({
			title: 'The rendering approach',
			doc_type_name: 'task',
			managed_meta: {
				'temper-stage': 'design',
				'temper-branch': 'jct/x',
				'temper-mode': null,
				'temper-effort': null,
				'temper-status': null,
				'temper-seq': null,
				'temper-pr': null,
				'temper-llm-model': null,
				'temper-llm-run': null,
				'temper-provenance': null,
			},
			open_meta: { owner: 'Pete' },
		});
		const { container } = render(Page, {
			data: {
				...data({ ...offering, stateVocabulary: { 'temper-stage': ['backlog', 'design'] } }),
				resource: withUnvalidatedState,
			} as PageData,
			form: null,
		});
		const stage = cellFor(container, 'temper-stage');
		expect(stage?.querySelector('select')).not.toBeNull();
		expect(stage?.querySelector('input[type="text"]')).toBeNull();

		// And not only the state that HAS a vocabulary. A managed key with no enum — most of
		// them — must get no free-text box either: the reason free text is wrong here is that
		// the key belongs to the system, not that this particular one happens to be validated.
		const managedKeys = [...container.querySelectorAll('.props .row.is-managed')].map(
			(row) => row.querySelector('dt')?.textContent,
		);
		expect(managedKeys).toContain('temper-branch');
		for (const row of container.querySelectorAll('.props .row.is-managed')) {
			expect(
				row.querySelector('input[type="text"]'),
				`${row.querySelector('dt')?.textContent} is a state, not a description`,
			).toBeNull();
		}
		// The description beside them still gets one, so this is not passing by offering nothing.
		expect(cellFor(container, 'owner')?.querySelector('input[type="text"]')).not.toBeNull();
	});

	it('shows an attach refusal at the attach form, not at a row', () => {
		const { container } = render(Page, {
			data: data(offering),
			form: {
				field: '',
				message: '"temper-stage" is a name the system owns.',
			},
		});
		expect(cellFor(container, 'owner')?.querySelector('.err')).toBeNull();
		expect(sentenceOf(container.querySelector('.props > .err'))).toBe(
			'"temper-stage" is a name the system owns.',
		);
	});
});

describe('the data artifacts region', () => {
	// Scoped to the main column: the artifacts region renders below the body, never in the
	// rail, and the rail scopes above must be able to tell the two apart.
	const mainRegion = (c: HTMLElement): Element | null => c.querySelector('.main');

	const artifact = {
		artifact_id: '01a00000000000000000000000000001',
		resource_id: 'res-1',
		kind_owner_table: 'kb_profiles',
		kind_owner_id: 'p-1',
		artifact_kind: 'measurement',
		intent: 'member',
		precedence: 0,
		content_hash: 'a'.repeat(64),
		content_bytes: 1536n,
		shape_state: 'never_declared',
		is_folded: false,
		created: '2026-08-20T10:00:00Z',
		content: { run: 2 },
	};

	it('renders a resource that owns no artifacts exactly as before — no section, no placeholder', async () => {
		const { container, unmount } = render(Page, { data: data(), form: null });
		await vi.waitFor(() => expect(container.querySelector('.props')).not.toBeNull());
		expect(container.querySelector('.main')?.textContent).not.toContain('Data artifacts');
		unmount();
	});

	it('keeps rendering nothing while the artifacts read is still in flight', () => {
		const { container } = render(Page, {
			data: data({ artifacts: pending<never[]>() }),
			form: null,
		});
		expect(container.querySelector('.main')?.textContent).not.toContain('Data artifacts');
	});

	it('renders the section whole when the resource owns artifacts', async () => {
		const { container, unmount } = render(Page, {
			data: data({ artifacts: Promise.resolve([artifact]) }),
			form: null,
		});
		await vi.waitFor(() => {
			expect(container.querySelector('.main')?.textContent).toContain('Data artifacts · 1');
		});
		expect(container.querySelector('.main')?.textContent).toContain('measurement');
		expect(container.querySelector('.main')?.textContent).toContain('1.5 KB');
		unmount();
	});

	it('never renders a read that gave up as absence — the stopping names itself', async () => {
		const words = await wordsOf(
			{ artifacts: gaveUpOn('data artifacts') },
			mainRegion,
			'region-gave-up',
		);
		expect(words.toLowerCase()).toContain('data artifacts');
	});

	it('never renders a failed read as absence — the failure names itself', async () => {
		const words = await wordsOf({ artifacts: broken() }, mainRegion, 'region-failed');
		expect(words.toLowerCase()).toContain('data artifacts');
	});
});

/**
 * D-F4 — the connections bound line is chrome, not a warning.
 *
 * The heading reads `shown of total` off the one envelope the read returned: present when the
 * read was clipped (stating what was withheld) and present when it was not (so complete is
 * something the reader is TOLD, never something they infer from silence — the `bound.ts`
 * SeedAxis form). A version that renders the bound only when it binds, or a count-only
 * heading, fails here: either one makes a clipped view distinguishable from a complete one
 * only by silence, which is the silent-slice defect one level down.
 */
describe('the connections bound line is chrome, not a warning', () => {
	it('a clipped read states how much was withheld — the heading carries both halves', async () => {
		const { container, unmount } = render(Page, {
			data: data({
				connections: Promise.resolve(connectionsOf([edge(1), edge(2), edge(3)], 12)),
			}),
			form: null,
		});
		await vi.waitFor(() => {
			expect(connectionsRegion(container)?.querySelector('.edge')).not.toBeNull();
		});

		expect(connectionsRegion(container)?.querySelector('.label')?.textContent).toBe(
			'Connections · 3 of 12',
		);
		unmount();
	});

	it('an unclipped read still states the bound — complete is told, not inferred', async () => {
		const { container, unmount } = render(Page, {
			data: data({ connections: Promise.resolve(connectionsOf([edge(1), edge(2)], 2)) }),
			form: null,
		});
		await vi.waitFor(() => {
			expect(connectionsRegion(container)?.querySelector('.edge')).not.toBeNull();
		});

		const heading = connectionsRegion(container)?.querySelector('.label')?.textContent;
		expect(heading).toBe('Connections · 2 of 2');
		// The `of total` half is the point — it is what makes completeness a stated fact
		// rather than an absence.
		expect(heading).toContain(' of 2');
		unmount();
	});
});
