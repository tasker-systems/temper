import type { EdgeKind, Polarity } from '$lib/types/generated/graph';
import type { AtlasEntry } from '$lib/types/generated/graph_atlas';
import type { QueryResponse, ResourceHit, StageResult } from '$lib/types/generated/query';
import type { ResourceView } from '$lib/types/generated/resource_view';
import { isCogmapHomed } from '$lib/vault-url';
import type { GraphPlan } from './composition';
import { whereOf } from './presentation';

/**
 * The whole mark vocabulary: **node** and **edge**, and nothing else.
 *
 * §3 — *"Nodes are `ResourceHit.resource`… Edges are `ViaEntry` — real `kb_edges` rows, as
 * stored."* That is how `navigation-never-silently-changes-kind` is satisfied: **not by careful
 * labelling but because there is no second kind to change into.** A region cannot reach the canvas
 * because {@link hitsOf} discriminates on `StageOutput.produced` and drops the `regions` variant —
 * a structural guarantee rather than a rule someone remembers.
 *
 * @see internal/superpowers/specs/2026-08-20-graph-successor-surface-design.md §3
 */

/**
 * One arm of the read that produced this model, and the words that read supplies for it.
 *
 * `[ruled — 2026-08-21, Pete]` **A read declares the arms it produced and what to call them. No
 * read may name another read's arms, and nothing outside a model may translate one.**
 *
 * What this replaces, and why the replacement is structural rather than a better wording: there
 * used to be a `NodeArm` union of `'seed' | 'survey' | 'walk'` naming **which stage of a
 * composition produced a row**, and a `describeArm` switch in `presentation.ts` translating it into
 * **a claim about what the reader did** — *"In the places you asked about"*. Those coincided
 * exactly as long as the composition was the only read on the surface, and have not since the entry
 * read landed. Three live instances followed, each one a new view meeting an old label:
 *
 * 1. the unaddressed entry, where {@link buildEntryGraph} marked all 130 marks `'seed'` and every
 *    hover card and the accessibility heading asserted a question nobody had asked;
 * 2. the ring, which fired on all 130 of them because they shared an arm;
 * 3. *"Walk from here →"*, which drops `q` and re-runs the composition with the hopped-from node as
 *    a seed, so its card said *"in the places you asked about"* about a node the reader hopped from.
 *
 * A fourth view would have produced a fourth. **The switch was the defect** — a claim made in one
 * place about screens built somewhere else — so the words moved to the read that knows what it did.
 * That is why reader-facing strings live in this file rather than in `presentation.ts`: they are
 * not a shared vocabulary any more, they are part of what a read returns.
 *
 * @see internal/superpowers/specs/2026-08-21-the-handoff-and-the-arm-vocabulary-design.md §1, §2
 */
export interface GraphArm {
	/**
	 * Identifies this arm **within this model** — the value {@link GraphNode.arm} carries.
	 *
	 * Deliberately not a union. A global enum is a vocabulary every read shares, which is exactly
	 * what let one read's words reach another's screen; a key is meaningless except against the
	 * `arms` of the model it came from.
	 */
	key: string;
	/** What this read calls the arm, to a reader. The only place these words come from. */
	label: string;
	/**
	 * Whether this arm holds what following edges **reached**, as against where the read stood.
	 *
	 * A declared property rather than the string `'walk'`, because `coreOf` and the ring both used
	 * to hard-code `!== 'walk'` — a global check no per-view vocabulary can satisfy. The ring
	 * encodes the view's standing point: ringed = what this view was built from, bare = what
	 * following edges reached from it.
	 */
	reached: boolean;
}

/**
 * One resource, drawn.
 *
 * It carries the whole {@link ResourceView} rather than a copied subset: the node IS the payload,
 * and every panel that opens off a node (the rail, the hover card) reads its fields from the row
 * the read already returned. The flattened `title`/`doc_type`/`home`/`degree` beside it are what
 * the layout and the marks consume, and they are named exactly as `AtlasNode` names them so the
 * surviving marks need no adapter.
 */
export interface GraphNode {
	id: string;
	title: string;
	doc_type: string | null;
	home: 'context' | 'cogmap';
	/** Degree over the DEDUPED edge set — see {@link buildGraph}. */
	degree: number;
	/**
	 * How connected this resource is in the reader's whole corpus — **or `null` when the read that
	 * produced this node did not report one.**
	 *
	 * The second of the two degrees §5.3 found, kept under a second name because that is the whole
	 * of what §5.3 was protecting: *"a node can carry `degree: 12` and show zero edges, and a
	 * reader has no way to reconcile that but to doubt themselves."* One name for two quantities is
	 * the hazard; two names are not.
	 *
	 * `[narrowed — 2026-08-21, Pete]` §5.3 ruled the corpus figure must never reach the screen. It
	 * now may, **only inside a sentence that states its relationship to the drawn one** — a bare
	 * second number beside a mark stays forbidden. What forced it: measured on production, every
	 * node in the entry read's band carries corpus degree ≥ the cut (min 11, max 87 at K=130), so
	 * `0 links` is false for all 26 and nothing but this figure can say why.
	 *
	 * **`null` means NOT REPORTED, never zero.** `ResourceView` carries no degree, so
	 * {@link buildGraph} genuinely cannot supply one — and a screen built from it must not borrow
	 * the entry read's sentence for a fact its own read never measured.
	 *
	 * @see internal/superpowers/specs/2026-08-21-hub-stranding-is-a-telling-failure-design.md §4, §5.1
	 */
	corpusDegree: number | null;
	excerpt: string | null;
	/**
	 * Which of **this model's** {@link GraphModel.arms} put this node on the canvas.
	 *
	 * A key, resolved against the legend the read declared. Nothing may translate it except the
	 * model it belongs to.
	 */
	arm: string;
	/**
	 * Where this resource lives, already in the reader's terms (`@owner/slug`, or a map's name).
	 *
	 * Carried on the node rather than read off `resource`, because the entry read has no
	 * `ResourceView` behind its marks and every panel must work the same on both paths. `null` means
	 * the read did not report a home — which the panels state, rather than rendering an empty cell.
	 */
	homeRef: string | null;
	/** When it last moved. Same reason as {@link homeId}. */
	updated: string | null;
	/** Workflow stage, where the read sourced one. */
	stage: string | null;
	/**
	 * The whole row, when the read that produced this node returned one.
	 *
	 * **`null` on the entry read**, whose marks come from `AtlasNode` — a projection, not a row.
	 * Nothing that renders a node may require this: the fields a panel needs are hoisted above, and
	 * a synthesised `ResourceView` would put fabricated values on a hover card, which is the exact
	 * defect class this surface is being repaired for.
	 */
	resource: ResourceView | null;
}

/**
 * One `kb_edges` row, drawn once.
 *
 * **`ViaEntry` is one entry per (seed, edge) PAIR**, not per edge — *"one entry per edge it was
 * reached by"*, and a walk takes a seed SET, so one edge repeats once per seed that reached it.
 * Measured on the real 50-node walk `[2026-08-20]`: **1,973 entries collapse to 102 distinct
 * edges**, a 19.3× inflation, with a maximum degree of **25**. Drawing `via` undeduped would put
 * 1,973 edge marks where 102 belong.
 *
 * **There is no edge id in this vocabulary, and that is a property of `ViaEntry` rather than an
 * omission here** — it carries `seed_id`, `source_id`, `target_id`, `edge_kind`, `label` and
 * `polarity`, and no `EdgeId`. So an edge has no durable address, cannot be named by `?sel=`, and
 * cannot be handed to `/api/graph/elements/edge/{id}/trail`. The successor's selection grammar is
 * therefore nodes only. Synthesising a composite id would manufacture an address the substrate
 * never issued and that nothing could resolve.
 */
export interface GraphEdge {
	source: string;
	target: string;
	edge_kind: EdgeKind;
	label: string | null;
	polarity: Polarity;
	/**
	 * **Always `null`.** `AtlasEdge` carries a stored `weight` that drives stroke thickness;
	 * `ViaEntry` carries none, so this states the absence rather than substituting a 1 that would
	 * read as a real, uniformly-light weight. {@link edgeStyle} renders a default stroke for it.
	 */
	weight: number | null;
	/**
	 * The distinct seeds this edge was reached from — the union of the collapsed entries'
	 * `seed_id`, which is *"the reason `via` exists at all"*: the score is the best path from ANY
	 * seed, so without this a multi-seed walk cannot say which.
	 */
	seedIds: string[];
}

export interface GraphModel {
	nodes: GraphNode[];
	edges: GraphEdge[];
	/**
	 * The arms this read produced, in the order a reader should meet them, each carrying its own
	 * words — the legend for {@link GraphNode.arm}.
	 *
	 * **Declaring an arm does not light a channel.** {@link armsDistinguish} still derives the ring's
	 * contrast from the nodes actually drawn, and the accessibility list still drops a group with no
	 * members. An arm a read declares but returns nothing for must not draw ink, and a count taken
	 * from this list rather than from the marks would do exactly that.
	 */
	arms: GraphArm[];
	/**
	 * How many raw `ViaEntry` rows collapsed into {@link edges}.
	 *
	 * Carried so the collapse is **assertable** rather than merely believed: a test can pin
	 * 1,973 → 102 on a captured response, and a regression that stopped deduping would move this
	 * number rather than quietly multiplying the marks. It is not rendered — the reader is owed
	 * the edges, not the arithmetic that produced them.
	 */
	viaEntries: number;
}

/**
 * The resource hits of one stage, or none.
 *
 * **The `regions` variant is dropped here, and that is the structural half of
 * `no-derived-thing-poses-as-authored`.** A stage that produced regions contributes no node,
 * because the discriminant is checked rather than assumed — so a future act returning regions
 * into a stage this surface draws cannot leak a `RegionHit` onto the canvas by accident.
 */
const hitsOf = (stage: StageResult | undefined): ResourceHit[] =>
	stage?.produced?.produced === 'resources' ? stage.produced.hits : [];

/** `kb_resource_homes.anchor_table`, read through the one function that gets the `!=` right. */
const homeOf = (row: ResourceView): 'context' | 'cogmap' =>
	isCogmapHomed(row) ? 'cogmap' : 'context';

/**
 * The first paragraph of a body, when a body was requested.
 *
 * `ResourceView.content` is *"absent means **not requested**, never 'empty body'"*, so this
 * returns `null` for the canvas reads (which request no body) and a real preview for the rail's
 * single targeted read. The client-side twin of the server's `compute_excerpt`, at the same
 * 280-char word-boundary bound the `AtlasNode.excerpt` doc names, so the two do not render
 * differently for the same resource.
 *
 * **Takes the body, not the row.** It only ever read one field, and asking for a whole
 * `ResourceView` to reach it had a cost that was not obvious: the graph load holds markdown from
 * `/api/resources/{id}/content` and a node that may have no row behind it (the entry read's marks
 * are an `AtlasNode` projection), so it had to write
 * `md === null || node.resource === null ? md : excerptOf({ ...node.resource, content: md })` —
 * a branch that hands the **whole document** to a slot sized for a paragraph. That branch was
 * unreachable only because one of the two reads happened to always carry a row. Narrowing the
 * parameter to what the function actually uses deletes the branch and the leak with it.
 */
export function excerptOf(body: string | null | undefined, max = 280): string | null {
	const trimmed = body?.trim();
	if (!trimmed) return null;
	const para = trimmed
		.split(/\n\s*\n/, 1)[0]
		.replace(/\s+/g, ' ')
		.trim();
	if (!para) return null;
	if (para.length <= max) return para;
	const cut = para.slice(0, max);
	const space = cut.lastIndexOf(' ');
	return `${(space > max * 0.6 ? cut.slice(0, space) : cut).trimEnd()}…`;
}

/**
 * `(source, target, kind, label)` — the four fields the spec names, and no more. **An edge's
 * identity on this surface**, and the only one: anything that has to tell two edges apart uses
 * this rather than assembling its own.
 *
 * JSON-encoded rather than interpolated so a **null label and an empty one stay distinct**.
 * `kb_edges.label` is `TEXT` with no `NOT NULL`, so the `None` is real and unobserved rather than
 * impossible; joining with an empty-string fallback would fold the two together and draw one mark
 * where the substrate holds two. A separator can be escaped into a label; a JSON array cannot be
 * forged.
 *
 * `[exported — 2026-08-21]` The rail's neighbour list keyed its render on `other.id + label + dir`,
 * which is **not** this and cannot substitute for it: it drops the kind, so two edges between the
 * same pair with the same effective label collided and Svelte threw `each_key_duplicate`, blanking
 * the whole page. Measured on the production entry read: **43 of 275 edges share a pair, and 5 of
 * 130 nodes crashed the rail.** All 275 are distinct under THIS key — which is also why deduping
 * them would have been wrong, and why the repair is one identity rather than a better ad-hoc one.
 */
export const edgeIdentity = (
	source: string,
	target: string,
	edge_kind: string,
	label: string | null,
): string => JSON.stringify([source, target, edge_kind, label]);

/**
 * Fold the entry read into the same two marks.
 *
 * **This is what replaces the recency page.** The old no-question entry drew 200 rows ordered
 * `updated DESC` while the walk seeded from every visible resource — two sets chosen by unrelated
 * criteria, so 244 of 250 marks arrived with their edges dropped for having an endpoint off-canvas.
 * Here one criterion decides both, and the server returns the induced subgraph, so **every edge has
 * both endpoints drawn by construction** and this function has no filtering left to do.
 *
 * Two differences from {@link buildGraph} worth naming rather than discovering:
 *
 * - **Degree is recomputed over the drawn edges, not taken from the wire.** `AtlasNode.degree` is
 *   the CORPUS degree — how connected this is in everything you can see — and the derived one is
 *   how connected it is in *what you are looking at*. Both are legitimate and they are different
 *   quantities; a node showing `degree: 12` beside three strokes gives a reader nothing to do but
 *   doubt themselves. Only the derived one reaches the screen.
 * - **Edges carry a real `weight`**, because `AtlasEdge` stores one. The composition's `ViaEntry`
 *   does not, which is why the field is nullable at all.
 *
 * `homes` maps an anchor id to how a reader names it. The server deliberately returns an id and not
 * a decorated ref: rendering `@owner/slug` in SQL would duplicate `graph_home_contexts`' owner_ref
 * expression, and the client already holds every anchor it can read.
 *
 * `salience` is deliberately dropped rather than carried: it is region-DERIVED, and a mark sized or
 * coloured by it would be `no-derived-thing-poses-as-authored` — the clause that got the tier model
 * deleted. `degree` may drive a channel because it counts the reader's own edges.
 *
 * @see internal/superpowers/specs/2026-08-20-grounding-and-navigation-split-design.md §5.1, §5.3, §8
 */
/**
 * The entry read's own arm — **one**, because it made one pass.
 *
 * It ranked the reader's whole visible corpus by how connected each thing is and drew the top of
 * it. That is the honest description, and it is a description of the READ: nothing here was asked
 * for, and nothing here was followed on to. `reached: false` follows from the same fact — there is
 * no second arm for these marks to stand apart from, so every one of them is core and
 * {@link armsDistinguish} withdraws the ring for want of a contrast.
 */
export const ENTRY_ARMS: GraphArm[] = [
	{ key: 'ranked', label: 'What your work is built around', reached: false },
];

export function buildEntryGraph(entry: AtlasEntry, homes: Map<string, string>): GraphModel {
	const nodes: GraphNode[] = entry.nodes.map((n) => ({
		id: n.id,
		title: n.title,
		doc_type: n.doc_type,
		home: n.home,
		// Recomputed below over the drawn edges. Starting from the wire's corpus degree and
		// incrementing would blend two different quantities into one number.
		degree: 0,
		// The wire figure, kept rather than dropped — `AtlasNode.degree` IS the corpus degree
		// (`graph_service.rs`: "§5.3's ruling ... is a claim about the screen, not the wire").
		// It is what lets the band say what its marks ARE connected to instead of asserting 0.
		corpusDegree: n.degree,
		excerpt: n.excerpt,
		homeRef: (n.home_id && homes.get(n.home_id)) ?? null,
		updated: n.updated,
		arm: ENTRY_ARMS[0].key,
		stage: n.stage,
		resource: null,
	}));
	const byId = new Map(nodes.map((n) => [n.id, n]));

	const edges: GraphEdge[] = entry.edges.map((e) => ({
		source: e.source,
		target: e.target,
		edge_kind: e.edge_kind,
		label: e.label,
		polarity: e.polarity,
		weight: e.weight,
		// No walk, so no seed reached this edge. Empty rather than fabricated.
		seedIds: [],
	}));

	for (const e of edges) {
		const s = byId.get(e.source);
		const t = byId.get(e.target);
		if (s) s.degree++;
		if (t) t.degree++;
	}

	// No `via` entries collapsed, because there were none to collapse — the server returned distinct
	// `kb_edges` rows. Zero is the honest count, not a missing measurement.
	return { nodes, edges, arms: ENTRY_ARMS, viaEntries: 0 };
}

export interface GraphInput {
	response: QueryResponse;
	plan: GraphPlan;
	/**
	 * The reader's own rows that the walk grew from but never returns — `follow-from` walks *"at
	 * least one hop"*, so a seed is not in its own answer. Both sources land here: the list read a
	 * no-question entry makes, and the explicit `from` seeds.
	 */
	seeds: ResourceView[];
}

/**
 * Fold one response into the two marks.
 *
 * **Arm precedence is `seed` → `survey` → `walk`, first writer wins.** A resource can legitimately
 * be in more than one arm (a seed the walk also reaches, a survey row followed on to), and the
 * reader's own named material is the truer description of it — so the arm a node keeps is the
 * nearest one to the reader, never the last one written.
 *
 * **Edges are filtered to pairs whose BOTH endpoints are drawn.** An edge whose other end is not
 * on this screen would be a stroke to nowhere; dropping it is not an omission the bound line has
 * to declare, because the edge was never a row of any arm — it is provenance attached to a node
 * that IS declared.
 */
/**
 * The composition's three arms, with the three words they have always carried.
 *
 * Unchanged wording, moved: these sentences were `describeArm`'s three cases, and they were true
 * of this read the whole time — a composition IS built from the places the reader asked about. What
 * was false was that any other read could reach them. They are declared here, by the read they
 * describe, and that is the whole of the repair.
 *
 * `no-internal-vocabulary-is-load-bearing` reaches here too: the reader is told *followed on from
 * your work*, never *"reached by `follow-from`"*. The order is the order the bound line declares
 * them, so the same partition reads the same way in both places.
 */
export const COMPOSITION_ARMS: GraphArm[] = [
	{ key: 'seed', label: 'In the places you asked about', reached: false },
	{ key: 'survey', label: 'From your places', reached: false },
	{ key: 'walk', label: 'Followed on from your work', reached: true },
];

export function buildGraph({ response, plan, seeds }: GraphInput): GraphModel {
	const nodes = new Map<string, GraphNode>();

	const add = (row: ResourceView, arm: string): void => {
		if (nodes.has(row.id)) return;
		nodes.set(row.id, {
			id: row.id,
			title: row.title,
			doc_type: row.doc_type_name,
			home: homeOf(row),
			degree: 0,
			// **Absent, not zero.** `ResourceView` carries no degree, so this read has nothing to
			// say about the corpus — and a band on this screen may genuinely hold resources
			// connected to nothing at all, which the entry read's band never can.
			corpusDegree: null,
			excerpt: excerptOf(row.content),
			arm,
			// Hoisted from the row so a panel reads the same fields whichever read produced the
			// node. `whereOf` still prefers the row when one is present — this is the fallback the
			// entry read needs, not a replacement for it.
			homeRef: whereOf(row),
			updated: row.updated ?? null,
			stage: (row.managed_meta?.['temper-stage'] as string | undefined) ?? null,
			resource: row,
		});
	};

	const [SEED, SURVEY, WALK] = COMPOSITION_ARMS.map((a) => a.key);
	for (const row of seeds) add(row, SEED);
	for (const stage of plan.surveyStages) {
		for (const hit of hitsOf(response.returned?.[stage])) add(hit.resource, SURVEY);
	}
	const walk = hitsOf(response.returned?.[plan.walkStage]);
	for (const hit of walk) add(hit.resource, WALK);

	// The collapse. Keyed on the four fields the spec names; `polarity` rides along rather than
	// keying, because it is a property of the identified row rather than part of its identity.
	const edges = new Map<string, GraphEdge>();
	let viaEntries = 0;
	for (const hit of walk) {
		for (const v of hit.via ?? []) {
			viaEntries++;
			if (!nodes.has(v.source_id) || !nodes.has(v.target_id)) continue;
			const key = edgeIdentity(v.source_id, v.target_id, v.edge_kind, v.label);
			const seen = edges.get(key);
			if (seen) {
				if (!seen.seedIds.includes(v.seed_id)) seen.seedIds.push(v.seed_id);
				continue;
			}
			edges.set(key, {
				source: v.source_id,
				target: v.target_id,
				edge_kind: v.edge_kind,
				label: v.label,
				polarity: v.polarity,
				weight: null,
				seedIds: [v.seed_id],
			});
		}
	}

	for (const e of edges.values()) {
		const s = nodes.get(e.source);
		const t = nodes.get(e.target);
		if (s) s.degree++;
		if (t) t.degree++;
	}

	return {
		nodes: [...nodes.values()],
		edges: [...edges.values()],
		arms: COMPOSITION_ARMS,
		viaEntries,
	};
}
