import type { SimulationLinkDatum, SimulationNodeDatum } from 'd3-force';
import type { EdgeType, GraphEdge, GraphNode } from '../types/generated/graph';
import type { Position } from './positions';

/** A node enriched with d3-force's required fields. */
export interface SimulationNode extends GraphNode, SimulationNodeDatum {}

/** A link indexed by node id — d3-force resolves refs via `.id(d => d.id)`. */
export interface SimulationLink extends SimulationLinkDatum<SimulationNode> {
	source: string;
	target: string;
	edge_type: EdgeType;
}

/** Merge seeded positions into the d3-force node shape. */
export function toSimulationNodes(
	nodes: GraphNode[],
	positions: Map<string, Position>
): SimulationNode[] {
	return nodes.map((node) => {
		const pos = positions.get(node.id);
		return {
			...node,
			x: pos?.x ?? 0,
			y: pos?.y ?? 0
		};
	});
}

/** Convert domain edges to id-referenced simulation links. */
export function toSimulationLinks(edges: GraphEdge[]): SimulationLink[] {
	return edges.map((edge) => ({
		source: edge.source,
		target: edge.target,
		edge_type: edge.edge_type
	}));
}
