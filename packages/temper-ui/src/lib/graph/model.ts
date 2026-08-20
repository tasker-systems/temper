import type { EdgeKind, Polarity } from '$lib/types/generated/graph';
import type { QueryResponse, ResourceHit, StageResult } from '$lib/types/generated/query';
import type { ResourceView } from '$lib/types/generated/resource_view';
import { isCogmapHomed } from '$lib/vault-url';
import type { GraphPlan } from './composition';

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
 * Which arm of the answer put this node on the canvas.
 *
 * The three are exactly the three arms the bound line declares, so a reader who asks *"which of
 * these did I ask for and which did the machine reach"* is answered by the same partition in both
 * places. `seed` and `survey` are both **the reader's own material in the places they named**;
 * `walk` is what was reached by following an edge out of it.
 */
export type NodeArm = 'seed' | 'survey' | 'walk';

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
	excerpt: string | null;
	arm: NodeArm;
	resource: ResourceView;
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
 */
export function excerptOf(row: ResourceView, max = 280): string | null {
	const body = row.content?.trim();
	if (!body) return null;
	const para = body
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
 * `(source, target, kind, label)` — the four fields the spec names, and no more.
 *
 * JSON-encoded rather than interpolated so a **null label and an empty one stay distinct**.
 * `kb_edges.label` is `TEXT` with no `NOT NULL`, so the `None` is real and unobserved rather than
 * impossible; joining with an empty-string fallback would fold the two together and draw one mark where the
 * substrate holds two. A separator can be escaped into a label; a JSON array cannot be forged.
 */
const edgeKey = (e: {
	source_id: string;
	target_id: string;
	edge_kind: string;
	label: string | null;
}): string => JSON.stringify([e.source_id, e.target_id, e.edge_kind, e.label]);

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
export function buildGraph({ response, plan, seeds }: GraphInput): GraphModel {
	const nodes = new Map<string, GraphNode>();

	const add = (row: ResourceView, arm: NodeArm): void => {
		if (nodes.has(row.id)) return;
		nodes.set(row.id, {
			id: row.id,
			title: row.title,
			doc_type: row.doc_type_name,
			home: homeOf(row),
			degree: 0,
			excerpt: excerptOf(row),
			arm,
			resource: row,
		});
	};

	for (const row of seeds) add(row, 'seed');
	for (const stage of plan.surveyStages) {
		for (const hit of hitsOf(response.returned?.[stage])) add(hit.resource, 'survey');
	}
	const walk = hitsOf(response.returned?.[plan.walkStage]);
	for (const hit of walk) add(hit.resource, 'walk');

	// The collapse. Keyed on the four fields the spec names; `polarity` rides along rather than
	// keying, because it is a property of the identified row rather than part of its identity.
	const edges = new Map<string, GraphEdge>();
	let viaEntries = 0;
	for (const hit of walk) {
		for (const v of hit.via ?? []) {
			viaEntries++;
			if (!nodes.has(v.source_id) || !nodes.has(v.target_id)) continue;
			const key = edgeKey(v);
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

	return { nodes: [...nodes.values()], edges: [...edges.values()], viaEntries };
}
