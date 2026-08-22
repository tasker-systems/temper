// GraphCanvas.component.test.ts — paint order, which is the only stacking SVG has.
//
// `[observed on production — 2026-08-22, Pete]` "the hover-over effects have the z-index washed out
// by the dots so we can't see them much of the time."
//
// The name is the reason it survived. **SVG does not honour `z-index`** — stacking is document
// order — so every attempt to fix this by setting one changed nothing, and that reads as the fix
// not taking rather than the diagnosis being wrong. The card was rendered inside its own
// `NodeChip`'s `<g>`, inside the node loop, so every node drawn after it painted over it: hover a
// node early in the list and most of the graph covers it; hover the last one and it looks fine.
//
// `GraphCanvas.svelte` already states the rule one block below, for captions — *"a label drawn
// between the node passes would be covered by a later mark … reintroduced by draw order."* The card
// broke the rule the labels already follow.
import { render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { GraphModel } from '$lib/graph/model';
import { resetAppContext, setPage } from '../../../test/app-context';
import GraphCanvas from './GraphCanvas.svelte';

vi.mock('$app/stores', () => import('../../../test/app-context'));
vi.mock('$app/navigation', () => import('../../../test/app-context'));

/**
 * Three connected marks. Three is the smallest number that can witness the defect: the card has to
 * be hovered on a node that is **not** the last one drawn, or a card rendered in the wrong place
 * still happens to paint on top and the test passes for the wrong reason.
 */
const model = (): GraphModel =>
	({
		nodes: [
			{ id: 'a', title: 'Alpha', doc_type: 'task', home: 'context', homeRef: '@me/x', degree: 2 },
			{ id: 'b', title: 'Bravo', doc_type: 'task', home: 'context', homeRef: '@me/x', degree: 2 },
			{ id: 'c', title: 'Charlie', doc_type: 'task', home: 'context', homeRef: '@me/x', degree: 2 },
		],
		edges: [
			{
				source: 'a',
				target: 'b',
				edge_kind: 'relates_to',
				label: 'relates',
				seedIds: [],
				weight: 1,
			},
			{
				source: 'b',
				target: 'c',
				edge_kind: 'relates_to',
				label: 'relates',
				seedIds: [],
				weight: 1,
			},
		],
		arms: [],
		viaEntries: 0,
	}) as unknown as GraphModel;

beforeEach(() => {
	resetAppContext();
	setPage('/graph/@me', { owner: '@me' });
});

/** Index of an element among every node it shares the SVG with, in document order. */
const paintIndex = (root: Element, el: Element): number =>
	[...root.querySelectorAll('*')].indexOf(el);

describe('the hover card is painted after every mark', () => {
	it('a card opened on the FIRST mark still paints above the last one', async () => {
		const { container } = render(GraphCanvas, {
			model: model(),
			selected: null,
			onSelect: () => {},
			emptyMessage: 'nothing',
		});

		const chips = [...container.querySelectorAll('.node-chip')];
		expect(chips.length).toBe(3);

		// The worst case, and the one a casual check misses: hover the mark drawn FIRST.
		chips[0].dispatchEvent(new MouseEvent('mouseenter', { bubbles: false }));
		await vi.waitFor(() => {
			expect(container.querySelector('foreignObject')).not.toBeNull();
		});

		const svg = container.querySelector('svg') as Element;
		const card = container.querySelector('foreignObject') as Element;
		const lastChip = chips[chips.length - 1];

		// Document order IS paint order in SVG. The card must come after the last mark, or that
		// mark is drawn over it — which is the whole defect.
		expect(paintIndex(svg, card)).toBeGreaterThan(paintIndex(svg, lastChip));
	});

	it('and the card is not nested inside the mark that opened it', async () => {
		const { container } = render(GraphCanvas, {
			model: model(),
			selected: null,
			onSelect: () => {},
			emptyMessage: 'nothing',
		});

		const chips = [...container.querySelectorAll('.node-chip')];
		chips[0].dispatchEvent(new MouseEvent('mouseenter', { bubbles: false }));

		// Awaited, and asserted non-null before it is used. Written first as a bare
		// `if (card) expect(...)`, which passed against the DEFECT — the card had not rendered yet,
		// so the guard skipped the assertion and the test reported green while proving nothing.
		let card: Element | null = null;
		await vi.waitFor(() => {
			card = container.querySelector('foreignObject');
			expect(card).not.toBeNull();
		});

		// Stated separately from the ordering assertion because it is the STRUCTURAL cause rather
		// than the symptom: while the card lives inside the chip's own `<g>`, no amount of
		// reordering the loop can lift it above the marks that follow.
		expect(chips[0].contains(card)).toBe(false);
	});
});
