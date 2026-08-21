/**
 * Tier-2 neighborhood layout (spec C2-D6): the ONLY place d3-force runs. Builds
 * a force graph from an R4 AtlasSubgraph, runs the simulation synchronously to a
 * settled state, and returns final node/edge positions. Pure w.r.t. inputs; the
 * simulation is deterministic (deterministic ring init, no Math.random).
 */
import {
	forceCenter,
	forceCollide,
	forceLink,
	forceManyBody,
	forceRadial,
	forceSimulation,
	type SimulationNodeDatum,
} from 'd3-force';
import type { NodeHome } from '$lib/types/generated/graph_atlas';

/**
 * What the simulation reads off a node — nothing about where it came from.
 *
 * `[widened — 2026-08-20]` from `AtlasNode` so the successor surface's nodes (which are
 * `ResourceHit.resource` projections) settle through the same physics. §4.2 calls this module
 * *"exactly what the successor draws"*; this is what makes that literally true rather than
 * aspirational. `AtlasNode` satisfies it unchanged.
 */
export interface LayoutNode {
	id: string;
	title: string;
	doc_type: string | null;
	home: NodeHome;
	degree: number;
	excerpt: string | null;
}

/** What the simulation reads off an edge: which two nodes it joins. Everything else is the mark's. */
export interface LayoutEdge {
	source: string;
	target: string;
}

export interface LayoutGraph<E extends LayoutEdge = LayoutEdge> {
	nodes: LayoutNode[];
	edges: E[];
}

export interface ForceNode extends SimulationNodeDatum {
	id: string;
	title: string;
	docType: string | null;
	home: NodeHome;
	degree: number;
	isSeed: boolean;
	/** Server-derived first-paragraph preview (see `AtlasNode.excerpt`); null when absent. */
	excerpt: string | null;
	x: number;
	y: number;
}

export interface ForceEdge<E extends LayoutEdge = LayoutEdge> {
	edge: E;
	source: ForceNode;
	target: ForceNode;
}

export interface ForceGraph<E extends LayoutEdge = LayoutEdge> {
	nodes: ForceNode[];
	edges: ForceEdge<E>[];
}

const TICKS = 300;

export interface ForceOptions {
	width: number;
	height: number;
	/**
	 * Which home is the SUBJECT of this view: its nodes hold the core, the other home rings
	 * them. Beat D's region drill distils ideas FROM sources, so cogmap facets are the core
	 * (the default). Beat E's context view inverts it: the work is the subject.
	 *
	 * This is the composition, not the visual language. Mark SHAPE stays keyed on `home`
	 * (`marks.ts`) so a circle is always a cogmap node and a rounded-square always a context
	 * resource, in every view.
	 */
	coreHome?: NodeHome;
	/**
	 * Which nodes hold the radial core, when `home` is not the axis that matters.
	 *
	 * `[added — 2026-08-20]` for the successor surface, where **both** homes are ordinary and the
	 * meaningful axis is the ARM: the reader's own material in the places they named settles at
	 * the core, and what a walk reached settles around it. Keying that on `home` would scatter a
	 * reader's own context rows to the outer ring on the very screen built to show them.
	 *
	 * Omitted, this falls back to `home === coreHome`, so the Atlas's two views are unchanged.
	 */
	coreOf?: (node: ForceNode) => boolean;
}

export function forceNeighborhood<E extends LayoutEdge>(
	subgraph: LayoutGraph<E>,
	seeds: string[],
	size: ForceOptions,
): ForceGraph<E> {
	const seedSet = new Set(seeds);
	const nodeCount = subgraph.nodes.length;
	const nodes: ForceNode[] = subgraph.nodes.map((n, i) => ({
		id: n.id,
		title: n.title,
		docType: n.doc_type,
		home: n.home,
		degree: n.degree,
		isSeed: seedSet.has(n.id),
		excerpt: n.excerpt,
		x: size.width / 2 + Math.cos((i / Math.max(1, nodeCount)) * 2 * Math.PI) * 120,
		y: size.height / 2 + Math.sin((i / Math.max(1, nodeCount)) * 2 * Math.PI) * 120,
	}));
	const byId = new Map(nodes.map((n) => [n.id, n]));

	const links = subgraph.edges
		.map((edge) => {
			const source = byId.get(edge.source);
			const target = byId.get(edge.target);
			return source && target ? { edge, source, target } : null;
		})
		.filter((l): l is ForceEdge<E> => l !== null);

	// Beat D: spatial reinforcement of the two axes — cogmap facets (ideas) settle
	// toward the center, context-resources (the builder axis / documents) drift to
	// an outer ring, so shape (NodeChip) and position agree.
	const minDim = Math.min(size.width, size.height);
	const rInner = minDim * 0.06;
	const rOuter = minDim * 0.44;
	// Which nodes hold the core. Default `home === 'cogmap'` preserves Beat D's region-drill layout.
	const coreHome = size.coreHome ?? 'cogmap';
	const isCore = size.coreOf ?? ((n: ForceNode) => n.home === coreHome);

	const sim = forceSimulation(nodes)
		.force(
			'link',
			// Cross-home links (facet→document) run looser + weaker so the radial can
			// pull documents outward; same-home links keep their structure.
			forceLink(links.map((l) => ({ source: l.source, target: l.target })))
				.distance((_l, i) => (links[i].source.home !== links[i].target.home ? 150 : 80))
				.strength((_l, i) => (links[i].source.home !== links[i].target.home ? 0.15 : 0.6)),
		)
		.force('charge', forceManyBody().strength(-260))
		.force('center', forceCenter(size.width / 2, size.height / 2))
		.force(
			'radial',
			forceRadial<ForceNode>(
				(n) => (isCore(n) ? rInner : rOuter),
				size.width / 2,
				size.height / 2,
			).strength(0.6),
		)
		.force(
			'collide',
			forceCollide<ForceNode>().radius((n) => 12 + Math.min(10, n.degree)),
		)
		.stop();

	for (let i = 0; i < TICKS; i++) sim.tick();

	return { nodes, edges: links };
}
