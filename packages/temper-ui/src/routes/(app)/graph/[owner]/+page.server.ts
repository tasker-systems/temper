import { error } from '@sveltejs/kit';
import { declareBounds, declareEntryBounds } from '$lib/graph/bound';
import { buildGraphPlan } from '$lib/graph/composition';
import { describeAnchor, questionFor, readableAnchors, resolveAnchors } from '$lib/graph/entry';
import { buildEntryGraph, buildGraph, excerptOf } from '$lib/graph/model';
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
import { parseGraphAddress } from '$lib/vault-url';
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

/** Which of the reader's own rows a `from` seed no longer resolving refers to. */
const isNotFound = (e: unknown): boolean => e instanceof ApiError && e.status === 404;

export const load: PageServerLoad = async ({ locals, params, url }): Promise<GraphViewData> => {
	const token = locals.accessToken!;
	const address = parseGraphAddress(url);

	const empty = (refusal: GraphRefusal): GraphViewData => ({
		owner: params.owner,
		question: address.question,
		borrowedFrom: null,
		refusal,
		// No read ran, so no read declared an arm. Empty rather than borrowed from a builder.
		model: { nodes: [], edges: [], arms: [], viaEntries: 0 },
		bound: null,
		readout: null,
		placesAsked: [],
		selected: null,
		selectedExcerpt: null,
		selectedTrail: null,
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
			owner: params.owner,
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
			selected: null,
			selectedExcerpt: null,
			selectedTrail: null,
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

	// The selection is ephemeral panel state and is resolved against what is actually drawn: a
	// `sel` naming a node this answer does not contain opens nothing rather than a rail describing
	// a resource that is not on screen.
	const selectedNode = address.selection
		? (model.nodes.find((n) => n.id === address.selection) ?? null)
		: null;
	const [selectedExcerpt, selectedTrail] = selectedNode
		? await Promise.all([
				readResourceBody(token, selectedNode.id).then((md) =>
					md === null || selectedNode.resource === null
						? md
						: excerptOf({ ...selectedNode.resource, content: md }, 600),
				),
				readTrail(token, 'node', selectedNode.id).catch((): EventTrail | null => null),
			])
		: [null, null];

	return {
		owner: params.owner,
		question: question.text,
		borrowedFrom: question.borrowedFrom,
		refusal: null,
		model,
		bound: declareBounds(response, plan),
		readout: buildReadout(response, regions),
		placesAsked: plan.anchorsAsked.map((a) => describeAnchor(a, cogmaps)),
		selected: selectedNode?.id ?? null,
		selectedExcerpt,
		selectedTrail,
	};
};
