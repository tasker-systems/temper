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
	forceSimulation,
	type SimulationNodeDatum
} from 'd3-force';
import type { AtlasEdge, AtlasNode, AtlasSubgraph } from '$lib/types/generated/graph_atlas';

export interface ForceNode extends SimulationNodeDatum {
	id: string;
	title: string;
	docType: string | null;
	home: AtlasNode['home'];
	degree: number;
	isSeed: boolean;
	/** Server-derived first-paragraph preview (see `AtlasNode.excerpt`); null when absent. */
	excerpt: string | null;
	x: number;
	y: number;
}

export interface ForceEdge {
	edge: AtlasEdge;
	source: ForceNode;
	target: ForceNode;
}

export interface ForceGraph {
	nodes: ForceNode[];
	edges: ForceEdge[];
}

const TICKS = 300;

export function forceNeighborhood(
	subgraph: AtlasSubgraph,
	seeds: string[],
	size: { width: number; height: number }
): ForceGraph {
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
		y: size.height / 2 + Math.sin((i / Math.max(1, nodeCount)) * 2 * Math.PI) * 120
	}));
	const byId = new Map(nodes.map((n) => [n.id, n]));

	const links = subgraph.edges
		.map((edge) => {
			const source = byId.get(edge.source);
			const target = byId.get(edge.target);
			return source && target ? { edge, source, target } : null;
		})
		.filter((l): l is ForceEdge => l !== null);

	const sim = forceSimulation(nodes)
		.force(
			'link',
			forceLink(links.map((l) => ({ source: l.source, target: l.target }))).distance(90).strength(0.6)
		)
		.force('charge', forceManyBody().strength(-260))
		.force('center', forceCenter(size.width / 2, size.height / 2))
		.force('collide', forceCollide<ForceNode>().radius((n) => 12 + Math.min(10, n.degree)))
		.stop();

	for (let i = 0; i < TICKS; i++) sim.tick();

	return { nodes, edges: links };
}
