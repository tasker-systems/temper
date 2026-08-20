import { error } from '@sveltejs/kit';
import { declareBounds } from '$lib/graph/bound';
import { buildGraphPlan } from '$lib/graph/composition';
import { questionFor, readableAnchors, resolveAnchors } from '$lib/graph/entry';
import { buildGraph, excerptOf } from '$lib/graph/model';
import { buildReadout, disclosedRegionIds } from '$lib/graph/readout';
import type { GraphRefusal, GraphViewData } from '$lib/graph/view';
import { ApiError } from '$lib/server/api';
import {
	readAnchorRegions,
	readAnchorSources,
	readResourceBody,
	readSeedResources,
	readSeedRows,
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
		model: { nodes: [], edges: [], viaEntries: 0 },
		bound: null,
		readout: null,
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

	// A question entry's own arm IS the survey, which is returned; only a no-question entry needs
	// the list read, and asking for it otherwise would put a second, unranked copy of the reader's
	// places on a screen that already has one.
	const wantsSeedRows = !question.text && address.seeds.length === 0;
	const [response, seedRows] = await Promise.all([
		runComposition(token, plan.composition),
		wantsSeedRows ? readSeedRows(token, plan.anchorsAsked, addressed) : Promise.resolve(null),
	]);

	// Naming a grouping needs the anchors' shapes, and only when something was disclosed — a
	// no-question entry runs no funnel and discloses nothing, so it pays nothing.
	const regions =
		disclosedRegionIds(response).length > 0
			? await readAnchorRegions(token, plan.anchorsAsked)
			: { rows: [], complete: true };

	const model = buildGraph({
		response,
		plan,
		seeds: [...namedSeeds, ...(seedRows?.rows ?? [])],
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
					md === null ? null : excerptOf({ ...selectedNode.resource, content: md }, 600),
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
		bound: declareBounds(response, plan, seedRows?.axis ?? null),
		readout: buildReadout(response, regions),
		selected: selectedNode?.id ?? null,
		selectedExcerpt,
		selectedTrail,
	};
};
