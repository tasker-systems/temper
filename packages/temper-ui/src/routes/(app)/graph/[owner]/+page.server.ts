import { error } from '@sveltejs/kit';
import { declareBounds, declareEntryBounds } from '$lib/graph/bound';
import { buildGraphPlan } from '$lib/graph/composition';
import { describeAnchor, questionFor, readableAnchors, resolveAnchors } from '$lib/graph/entry';
import { buildEntryGraph, buildGraph, excerptOf, type GraphModel } from '$lib/graph/model';
import { buildReadout, disclosedRegionIds } from '$lib/graph/readout';
import type { GraphRefusal, GraphViewData } from '$lib/graph/view';
import { ApiError } from '$lib/server/api';
import {
	readAnchorRegions,
	readAnchorSources,
	readEntry,
	readResourceBody,
	readSeedResources,
	runComposition,
} from '$lib/server/graph-query';
import { readTrail } from '$lib/server/graph-reads';
import type { EventTrail } from '$lib/types/generated/element_trail';
import type { ResourceView } from '$lib/types/generated/resource_view';
import { type GraphAddress, parseGraphAddress } from '$lib/vault-url';
import type { PageServerLoad } from './$types';

/**
 * The graph surface — one route, four params, three entries.
 *
 * **The whole addressable state is `q` / `in` / `from` / `sel`**, and the invariant that buys is
 * that the URL is a projection of the composition: no state on screen the URL does not describe,
 * no param that does not name a part of the plan. There are no tiers to ascend and no scope to
 * hold; this load reads the address, resolves it against what the reader can read, builds one
 * composition, and folds the answer into two marks.
 *
 * **No backend change in this arc.** Everything here is `/api/query` plus reads that already
 * existed, through the wholesale `/api` proxy.
 *
 * @see internal/superpowers/specs/2026-08-20-graph-successor-surface-design.md §1, §2, §3
 */

/**
 * What a READ returns: everything about the answer, and **nothing about the selection**.
 *
 * Derived from {@link GraphViewData} by subtraction rather than written out, so the two cannot
 * drift and a field added to the view is a field a read must supply — except the four named here,
 * which a read may not decide.
 *
 * `[found on production — 2026-08-21]` This type exists because of a defect it makes
 * unrepresentable. Every branch of this load used to assemble the whole `GraphViewData` itself, so
 * resolving `?sel=` was something a branch had to *remember*. The composition branch remembered;
 * the entry read — added later, by chunk A — did not, and hard-coded `selected: null`. The result
 * was that **clicking any mark on the screen every reader meets first wrote `?sel=` into the URL
 * and opened nothing**, for every node, for as long as that read has existed. No test could fail:
 * the rail's tests all run on the composition fixture.
 *
 * A third read is already designed (chunk D2's traversal). Adding the three lines to the entry
 * branch would have closed the instance and left the next branch free to forget it again.
 */
type GraphRead = Omit<GraphViewData, 'owner' | 'selected' | 'selectedExcerpt' | 'selectedTrail'>;

/**
 * The rail's contents for `?sel=`, resolved against whatever the read actually drew.
 *
 * **Resolved against the model, not looked up in the corpus**: a `sel` naming a resource this
 * answer does not contain opens nothing, rather than a rail describing something that is not on
 * screen. That rule predates this function — it is why the selection is resolved here at all — and
 * it now holds for every read instead of one.
 *
 * Both reads take a **bare id**, which is what makes this work on an entry mark: those marks are an
 * `AtlasNode` projection with `resource: null`, and neither `/api/resources/{id}/content` nor the
 * element trail needs a row. Nothing is synthesised to stand in for the missing one.
 *
 * A failed trail read degrades to `null` — the rail simply carries no history — while a failed body
 * read is already `null` inside {@link readResourceBody}. Neither is allowed to take down a screen
 * whose marks are all drawn and correct.
 */
async function resolveSelection(
	token: string,
	model: GraphModel,
	selection: string | null,
): Promise<Pick<GraphViewData, 'selected' | 'selectedExcerpt' | 'selectedTrail'>> {
	const node = selection ? (model.nodes.find((n) => n.id === selection) ?? null) : null;
	if (!node) return { selected: null, selectedExcerpt: null, selectedTrail: null };

	const [selectedExcerpt, selectedTrail] = await Promise.all([
		// 600 rather than the 280 the canvas uses: the rail has the room, and this is the one
		// targeted body read on the screen. `excerptOf` takes the markdown directly — it never
		// needed a row, and asking for one is what used to force a branch that handed the WHOLE
		// document to this slot whenever a node had no row behind it.
		readResourceBody(token, node.id).then((md) => excerptOf(md, 600)),
		readTrail(token, 'node', node.id).catch((): EventTrail | null => null),
	]);
	return { selected: node.id, selectedExcerpt, selectedTrail };
}

/** Which of the reader's own rows a `from` seed no longer resolving refers to. */
const isNotFound = (e: unknown): boolean => e instanceof ApiError && e.status === 404;

/**
 * The whole load: **read, then assemble.** Six lines, and the shape is the point.
 *
 * Every branch of the answer lives in {@link readFor}, whose return type is {@link GraphRead} —
 * so a branch that tries to decide `selected` is an excess property and **fails to compile**.
 * That is the difference between a defect that is fixed and one that cannot recur: the entry read
 * shipped hard-coding `selected: null` and nothing objected, for months.
 *
 * The selection is then resolved once, against whatever that read actually drew.
 */
export const load: PageServerLoad = async (event): Promise<GraphViewData> => {
	const token = event.locals.accessToken!;
	const address = parseGraphAddress(event.url);
	const read = await readFor(token, address);
	return {
		owner: event.params.owner,
		...read,
		...(await resolveSelection(token, read.model, address.selection)),
	};
};

/**
 * Which answer this address gets, and everything about it **except the selection**.
 *
 * Three exits, and the return type is what keeps them honest — see {@link GraphRead}.
 */
async function readFor(token: string, address: GraphAddress): Promise<GraphRead> {
	const empty = (refusal: GraphRefusal): GraphRead => ({
		question: address.question,
		borrowedFrom: null,
		refusal,
		// No read ran, so no read declared an arm. Empty rather than borrowed from a builder.
		model: { nodes: [], edges: [], arms: [], viaEntries: 0 },
		bound: null,
		readout: null,
		placesAsked: [],
	});

	const [contexts, cogmaps] = await readAnchorSources(token);
	const resolution = resolveAnchors(readableAnchors({ contexts, cogmaps }), address.anchors);

	// A named place that no longer resolves is a REFUSAL, not a widening. An empty anchor list with
	// a question in hand makes the builder emit `find-about-anywhere` — search everything I can see
	// — so a link naming one deleted context would answer across the whole corpus while the bound
	// line truthfully reported `12 of 12 places`. Plausible, well-formed and wrong.
	if (resolution.entry === 'none-resolved') {
		return empty({ kind: 'no-place-resolved', named: resolution.named });
	}

	const addressed = resolution.entry === 'named';
	const question = questionFor(address.question, resolution.anchors, cogmaps);

	const outcome = buildGraphPlan({
		anchors: resolution.anchors,
		question: question.text,
		seeds: address.seeds,
		available: resolution.available,
	});
	if (!outcome.ok) return empty({ kind: 'nothing-to-ask' });
	const plan = outcome.plan;

	// The seeds a walk grows from but never returns. Explicit `from` ids replace the upstream stage
	// as what the walk grows from, so they are what gets drawn beside it; otherwise a no-question
	// entry's own rows come from the list read, which is also the only source on this screen with a
	// true denominator.
	//
	// A `from` id that no longer resolves is an honest 404 about the reader's own material —
	// declined rather than silently answered as a smaller graph. `/api/resources/{id}` answers 404
	// for a resource that is gone or was never readable, and the two are deliberately
	// indistinguishable.
	let namedSeeds: ResourceView[] = [];
	if (address.seeds.length > 0) {
		try {
			namedSeeds = await readSeedResources(token, address.seeds);
		} catch (e) {
			if (isNotFound(e)) {
				error(404, 'That resource is no longer here, so there is nothing to walk from.');
			}
			throw e;
		}
	}

	// ── The grounding/navigation split, at the one place it is observable ───────────────────────
	//
	// A reader who has asked nothing gets the ENTRY READ, and runs no composition at all.
	//
	// This is what replaces the recency page. That page drew 200 rows ordered `updated DESC` while
	// the walk seeded from every visible resource — two sets chosen by unrelated criteria, so 244 of
	// 250 marks arrived with their edges dropped for having an endpoint off-canvas. The entry read
	// ranks by degree and returns the induced subgraph over that ranking, so one criterion decides
	// both and every edge has both endpoints drawn. Measured on production, the unconnected band
	// falls from 97.6% to 20%.
	//
	// It also stops paying for a composition nobody asked for, which is most of the latency the
	// reader ranked second-most-jarring.
	const isEntry = !question.text && address.seeds.length === 0;
	if (isEntry) {
		const entry = await readEntry(
			token,
			// Empty ranks across everything visible; named places confine it. `addressed` is exactly
			// the distinction — a reader who named a place is answered WITHIN it.
			addressed ? resolution.anchors.map((a) => a.id) : [],
		);

		// Anchor id → how the reader names it. The server returns an id deliberately: rendering
		// `@owner/slug` in SQL would duplicate an expression that already exists elsewhere, and this
		// page has already read every anchor it can see.
		const homes = new Map<string, string>([
			...contexts.map((c): [string, string] => [c.id, `${c.owner_ref}/${c.slug}`]),
			...cogmaps.map((m): [string, string] => [m.id, m.name]),
		]);

		const bounds = {
			drawn: entry.bounds.drawn,
			eligible: entry.bounds.eligible,
			inScope: entry.bounds.in_scope,
			truncated: entry.bounds.truncated,
		};

		return {
			question: null,
			borrowedFrom: null,
			// Rung 2 (spec §6): too little structure to BE a graph. Declared, never drawn as an
			// empty canvas — "dots the reader cannot use are not more honest than a sentence saying
			// the graph is the wrong instrument here", and the sentence is the one that respects
			// `the-unstructured-reader-is-never-worse-off`. The rung must be VISIBLE, because a
			// reader on it is looking at a different claim than one on rung 1.
			refusal:
				bounds.eligible === 0 ? { kind: 'too-little-structure', inScope: bounds.inScope } : null,
			model: buildEntryGraph(entry, homes),
			bound: declareEntryBounds(bounds, {
				asked: plan.anchorsAsked.length,
				available: plan.anchorsAvailable,
			}),
			// No question was asked, so there is no reasoning to report. Absent rather than an empty
			// readout: inventing one would fabricate an explanation for a screen nobody questioned.
			readout: null,
			placesAsked: plan.anchorsAsked.map((a) => describeAnchor(a, cogmaps)),
		};
	}

	// Past this point a question (or an explicit `from`) is in hand, so the composition IS the
	// answer and there is no second, unranked copy of the reader's places to fetch.
	const response = await runComposition(token, plan.composition);

	// Naming a grouping needs the anchors' shapes, and only when something was disclosed — a
	// no-question entry runs no funnel and discloses nothing, so it pays nothing.
	const regions =
		disclosedRegionIds(response).length > 0
			? await readAnchorRegions(token, plan.anchorsAsked)
			: { rows: [], complete: true };

	const model = buildGraph({
		response,
		plan,
		seeds: namedSeeds,
	});

	return {
		question: question.text,
		borrowedFrom: question.borrowedFrom,
		refusal: null,
		model,
		bound: declareBounds(response, plan),
		readout: buildReadout(response, regions),
		placesAsked: plan.anchorsAsked.map((a) => describeAnchor(a, cogmaps)),
	};
}
