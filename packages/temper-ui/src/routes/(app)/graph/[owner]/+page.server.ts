import { error } from '@sveltejs/kit';
import { declareBounds, declareEntryBounds, declareTraversalBounds } from '$lib/graph/bound';
import { buildGraphPlan } from '$lib/graph/composition';
import { describeAnchor, questionFor, readableAnchors, resolveAnchors } from '$lib/graph/entry';
import {
	buildEntryGraph,
	buildGraph,
	buildTraversal,
	excerptOf,
	type GraphModel,
} from '$lib/graph/model';
import { buildReadout, disclosedRegionIds } from '$lib/graph/readout';
import type { GraphRefusal, GraphViewData } from '$lib/graph/view';
import { ApiError } from '$lib/server/api';
import { bounded, derive } from '$lib/server/bounded';
import {
	readAnchorRegions,
	readAnchorSources,
	readEntry,
	readResourceBody,
	readSeedResources,
	readTraversal,
	runComposition,
} from '$lib/server/graph-query';
import { readTrail } from '$lib/server/graph-reads';
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
 * A `sel` that names something this answer does not contain — the one branch on which a rail read is
 * neither started nor answerable.
 *
 * **Unreachable by any consumer, and a rejection rather than a value on purpose.** `selected`
 * resolves to `null` in exactly this case and the rail is gated on it, so nothing ever awaits the
 * two promises this lands in. What it must not do is stand in for the read with a *value*: `null` on
 * the excerpt says *this resource has no body*, and an empty trail says *it has no history*. Both
 * are claims about the reader's material that no read verified, which is the conflation spec §5.2
 * rules out. Nothing was read, and this says only that.
 */
class NotInThisAnswer extends Error {
	constructor(label: string) {
		super(`no ${label} read ran: this selection is not in the answer`);
		this.name = 'NotInThisAnswer';
	}
}

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
 * **Everything here is STREAMED, including the selection itself.** `[2026-08-21]` The selection used
 * to be settled, and could be while the model was — this function took a resolved `GraphModel`. Once
 * the model streams, the only way to keep `selected` a value is to `await` the model here, which
 * would silently restore the blocking the whole page was changed to stop, and every test would still
 * pass. So the resolution moved into a `.then()`. **What it resolves against did not change**: the
 * model this read actually produced, not the corpus.
 *
 * Everything the rail shows apart from the excerpt and the history comes from `model.nodes`, so once
 * the model lands the rail frame paints fully populated with two regions declaring themselves as
 * arriving.
 *
 * The degradation policy is unchanged and still right: *a failed side-read must never take down a
 * screen whose marks are all drawn.* What changed is what it degrades **to**.
 * `[amended — 2026-08-21, spec §5.2]` Both reads used to degrade to `null` — the trail through a
 * `.catch()` here, the body inside {@link readResourceBody} — and `null` asserts *there is nothing
 * here*, which is a claim about the reader's material that a failed read has verified nothing
 * about. A rejection now travels to the template, where `{:catch}` renders it as a **named
 * failure**: *history unavailable*, distinct from *no history*.
 *
 * {@link bounded} supplies both the give-up (spec §5.4 — a read that never answers presents as
 * arriving forever, which is the exact failure of `working-and-stopped-are-distinguishable`) and
 * the `.catch()` attached at creation that keeps an unawaited rejection from crashing the server.
 * That catch is a *different mechanism* from the template's `{:catch}`; spec §5.3 says so, and
 * having one does not give you the other.
 */
function resolveSelection(
	token: string,
	model: Promise<GraphModel>,
	selection: string | null,
): Pick<GraphViewData, 'selected' | 'selectedExcerpt' | 'selectedTrail'> {
	// **A reader who opened no rail pays nothing, and knows it before any read.** That `?sel=` is
	// absent is knowable from the ADDRESS, so this case is decided here rather than chained off the
	// model: no read is started, and the outer null on the two read fields keeps saying what it has
	// always said — *nothing is selected*, which is a different fact from a read that answered with
	// nothing. `selected` is the one that changes spelling, because it is a promise on every path.
	if (!selection) {
		return { selected: Promise.resolve(null), selectedExcerpt: null, selectedTrail: null };
	}

	const node = derive(model, (m) => m.nodes.find((n) => n.id === selection) ?? null);

	/** Start a rail read **only if the selection was drawn**, and bound it from that moment. */
	const railRead = <T>(start: (id: string) => Promise<T>, label: string): Promise<T> =>
		derive(node, (n) => {
			if (!n) throw new NotInThisAnswer(label);
			// `bounded` inside rather than around, so the give-up measures the READ rather than the
			// read plus however long the model took to arrive.
			return bounded(start(n.id), label);
		});

	return {
		selected: derive(node, (n) => n?.id ?? null),
		// 600 rather than the 280 the canvas uses: the rail has the room, and this is the one
		// targeted body read on the screen. `excerptOf` takes the markdown directly — it never
		// needed a row, and asking for one is what used to force a branch that handed the WHOLE
		// document to this slot whenever a node had no row behind it.
		selectedExcerpt: railRead(
			(id) => readResourceBody(token, id).then((md) => excerptOf(md, 600)),
			'excerpt',
		),
		selectedTrail: railRead((id) => readTrail(token, 'node', id), 'history'),
	};
}

/** Which of the reader's own rows a `from` seed no longer resolving refers to. */
const isNotFound = (e: unknown): boolean => e instanceof ApiError && e.status === 404;

/**
 * Anchor id → how the reader names it.
 *
 * The server returns an id deliberately: rendering `@owner/slug` in SQL would duplicate
 * `graph_home_contexts`' owner_ref expression, and this page has already read every anchor it can
 * see. Shared by the two reads whose marks carry a `home_id` — the entry read and the traversal —
 * rather than rebuilt in each, which is how the second one would come to name places differently
 * from the first.
 */
const homesOf = (
	contexts: { id: string; owner_ref: string; slug: string }[],
	cogmaps: { id: string; name: string }[],
): Map<string, string> =>
	new Map<string, string>([
		...contexts.map((c): [string, string] => [c.id, `${c.owner_ref}/${c.slug}`]),
		...cogmaps.map((m): [string, string] => [m.id, m.name]),
	]);

/**
 * The whole load: **read, then assemble.** Six lines, and the shape is the point.
 *
 * Every branch of the answer lives in {@link readFor}, whose return type is {@link GraphRead} —
 * so a branch that tries to decide `selected` is an excess property and **fails to compile**.
 * That is the difference between a defect that is fixed and one that cannot recur: the entry read
 * shipped hard-coding `selected: null` and nothing objected, for months.
 *
 * The selection is then resolved once, against whatever that read actually drew.
 *
 * **The `await` on the left is the scaffold's, never the answer's.** {@link readFor} awaits exactly
 * two things and returns the rest as promises (spec §2, §3.1): the anchor sources, because the plan
 * they feed decides *which read runs at all*, and the `from` seeds, because a seed that no longer
 * resolves is a real 404 about the reader's own material rather than a page frame around a failed
 * region. Everything downstream of the plan streams.
 */
export const load: PageServerLoad = async (event): Promise<GraphViewData> => {
	const token = event.locals.accessToken!;
	const address = parseGraphAddress(event.url);
	const read = await readFor(token, address);
	return {
		owner: event.params.owner,
		...read,
		...resolveSelection(token, read.model, address.selection),
	};
};

/**
 * Which answer this address gets, and everything about it **except the selection**.
 *
 * Three exits, and the return type is what keeps them honest — see {@link GraphRead}.
 */
async function readFor(token: string, address: GraphAddress): Promise<GraphRead> {
	// **A refusal is the answer, not a delay.** Nothing here is read and nothing here waits: the
	// refusal is settled, so it renders with the page chrome, and the four streamed fields are
	// already-resolved promises rather than nulls. An outer null on any of them would be a fourth
	// state on a field that already carries three.
	const empty = (refusal: GraphRefusal): GraphRead => ({
		question: address.question,
		borrowedFrom: null,
		refusal,
		// No read ran, so no read declared an arm. Empty rather than borrowed from a builder.
		model: Promise.resolve({ nodes: [], edges: [], arms: [], viaEntries: 0 }),
		bound: Promise.resolve(null),
		readout: Promise.resolve(null),
		tooLittleStructure: Promise.resolve(null),
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
	//
	// **This read stays awaited, and that is the reason.** Streaming it would turn an honest 404
	// into a page frame around a failed region — the same call the resource page makes about its own
	// scaffold read. A 404 must be a 404.
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

	// ── The handoff: `from` present means NAVIGATE, and the question stops deciding ─────────────
	//
	// §10.3, ruled: "asking a question and our query composition frame helps set the space, but then
	// you traverse the graph as normal WITHOUT A QUESTION LOCKING YOU IN." So this branch runs no
	// composition — `q` is still in the address, and it is provenance rather than a filter.
	//
	// It is checked BEFORE `isEntry` and before the composition, which is the whole of the routing
	// change: `?q=X&from=Y` used to run the composition with Y as an explicit seed, which is why a
	// hop re-ran the question and drew the answer under a legend that said "In the places you asked
	// about" over a mark the reader had hopped to.
	//
	// **The walk is not confined to the grounding's result set**, and that is the ruling rather than
	// a gap: `traversal_slice` calls `graph_induced_edges` over the reader's whole visible corpus.
	// Nothing this branch returns may imply the question is still narrowing.
	if (address.seeds.length > 0) {
		// Resolved HERE rather than left to the service's `unwrap_or(1)`, because the bound line has
		// to report the depth that actually ran. A client that omitted the param would be reporting
		// a number it did not choose and cannot see — two copies of one default, on either side of
		// the wire, with nothing linking them.
		const depth = address.depth ?? 1;
		// **One read, three fields.** The canvas, the bound line and the panel beside them are three
		// views of this single walk, so all three are derived from it rather than from three reads
		// of their own — they cannot disagree about whether it answered, because there is nothing
		// for them to disagree about.
		const walked = bounded(readTraversal(token, address.seeds, depth), 'graph');
		const model = derive(walked, (w) =>
			buildTraversal(w, address.seeds, homesOf(contexts, cogmaps)),
		);

		return {
			// The reader's own `q`, never `questionFor`'s borrowed one: a charter question stands in
			// for a question the reader did not ask, and this panel's whole job is to say where THEY
			// started. Borrowing here would attribute a question to them that they never typed.
			question: address.question,
			borrowedFrom: null,
			refusal: null,
			model,
			bound: derive(model, (m) =>
				declareTraversalBounds({
					drawn: m.nodes.length,
					// Counted over the marks actually DRAWN, not over what was asked for. A seed the
					// reader cannot see is not returned, and claiming to have hopped from it would put a
					// number on screen with no mark under it.
					from: m.nodes.filter((n) => address.seeds.includes(n.id)).length,
					depth,
				}),
			),
			// No composition ran, so there is no reasoning to report. The provenance panel is built
			// from `question` and `placesAsked` instead — see `GraphPage`. Derived from the walk
			// rather than resolved outright, so this null is only ever asserted about an answer that
			// actually arrived.
			readout: derive(model, () => null),
			// Rung 2 is the entry read's verdict about its own answer, and this read has no such
			// axis: `traversal_slice` ranks and cuts nothing, so no walk can reach that conclusion.
			tooLittleStructure: Promise.resolve(null),
			// The places the GROUNDING named, carried so its measurements stay reachable. They
			// describe where the reader started, not this screen, and the panel says so.
			placesAsked: plan.anchorsAsked.map((a) => describeAnchor(a, cogmaps)),
		};
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
		// One read, and everything below is a view of it — see the traversal branch's note.
		const entry = bounded(
			readEntry(
				token,
				// Empty ranks across everything visible; named places confine it. `addressed` is exactly
				// the distinction — a reader who named a place is answered WITHIN it.
				addressed ? resolution.anchors.map((a) => a.id) : [],
			),
			'graph',
		);

		const bounds = derive(entry, (e) => ({
			drawn: e.bounds.drawn,
			eligible: e.bounds.eligible,
			inScope: e.bounds.in_scope,
			truncated: e.bounds.truncated,
		}));

		return {
			question: null,
			borrowedFrom: null,
			// Nothing about the ADDRESS refuses this reader, and that is all this field may report.
			refusal: null,
			model: derive(entry, (e) => buildEntryGraph(e, homesOf(contexts, cogmaps))),
			bound: derive(bounds, (b) =>
				declareEntryBounds(b, {
					asked: plan.anchorsAsked.length,
					available: plan.anchorsAvailable,
				}),
			),
			// No question was asked, so there is no reasoning to report. Absent rather than an empty
			// readout: inventing one would fabricate an explanation for a screen nobody questioned.
			readout: derive(entry, () => null),
			// Rung 2 (spec §6): too little structure to BE a graph. Declared, never drawn as an
			// empty canvas — "dots the reader cannot use are not more honest than a sentence saying
			// the graph is the wrong instrument here", and the sentence is the one that respects
			// `the-unstructured-reader-is-never-worse-off`. The rung must be VISIBLE, because a
			// reader on it is looking at a different claim than one on rung 1.
			//
			// **It streams, and `refusal` above does not.** `eligible` is a number this read
			// reports, so the verdict cannot precede the read — it arrives in place of the canvas
			// it replaces, which is where a reader is already looking. Putting it on `refusal`
			// would have meant awaiting this read to decide the page frame, and that is the
			// blocking this whole route was changed to stop.
			tooLittleStructure: derive(bounds, (b) =>
				b.eligible === 0 ? { kind: 'too-little-structure' as const, inScope: b.inScope } : null,
			),
			placesAsked: plan.anchorsAsked.map((a) => describeAnchor(a, cogmaps)),
		};
	}

	// Past this point a question (or an explicit `from`) is in hand, so the composition IS the
	// answer and there is no second, unranked copy of the reader's places to fetch.
	//
	// One read, three fields, again — and the second read here is INSIDE the first because it is
	// downstream of it: whether any grouping was disclosed is something only the response can say.
	// The chained promise needs no `.catch()` of its own; `bounded`'s `Promise.race` subscribes to
	// what it is given, and its guard is on what it hands back.
	const composed = bounded(
		runComposition(token, plan.composition).then(async (response) => ({
			response,
			// Naming a grouping needs the anchors' shapes, and only when something was disclosed — a
			// no-question entry runs no funnel and discloses nothing, so it pays nothing.
			regions:
				disclosedRegionIds(response).length > 0
					? await readAnchorRegions(token, plan.anchorsAsked)
					: { rows: [], complete: true },
		})),
		'graph',
	);

	return {
		question: question.text,
		borrowedFrom: question.borrowedFrom,
		refusal: null,
		model: derive(composed, ({ response }) => buildGraph({ response, plan, seeds: namedSeeds })),
		bound: derive(composed, ({ response }) => declareBounds(response, plan)),
		readout: derive(composed, ({ response, regions }) => buildReadout(response, regions)),
		// A composition has no connection floor and no `eligible` axis, so it can never reach rung
		// 2's verdict — see the entry branch, which is the only read that can.
		tooLittleStructure: Promise.resolve(null),
		placesAsked: plan.anchorsAsked.map((a) => describeAnchor(a, cogmaps)),
	};
}
