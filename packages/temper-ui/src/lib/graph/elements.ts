import type { GraphEdge, GraphNode } from '../types/generated/graph';
import { deriveDisplay } from './derive';
import { nodeColor, nodeFontSize, nodeHeightPx, nodeWidthPx } from './styling';

/**
 * Cytoscape element definitions — the wire format that `cytoscape({ elements })`
 * consumes. Matches the `ElementDefinition` type from `@types/cytoscape`.
 *
 * We mirror the prototype's shape (`ui_kits/app/KnowledgeGraph.jsx:35-75`)
 * including the precomputed `fontSize` / `widthPx` / `heightPx` / `fill` /
 * `label` / `fullTitle` / `dateStrip` / `sessions` data fields, and the
 * `type-<doctype>` + `aggregator|participant` classes that the stylesheet
 * selectors hook into.
 */
export interface CytoscapeNodeElement {
	group: 'nodes';
	data: {
		id: string;
		type: string;
		aggregator: boolean;
		label: string;
		fullTitle: string;
		dateStrip: string | null;
		edges: number;
		sessions: number;
		fontSize: number;
		widthPx: number;
		heightPx: number;
		fill: string;
	};
	classes: string;
}

export interface CytoscapeEdgeElement {
	group: 'edges';
	data: {
		id: string;
		source: string;
		target: string;
		type: string;
		sourceFill: string;
	};
	classes: string;
}

export type CytoscapeElement = CytoscapeNodeElement | CytoscapeEdgeElement;

/**
 * Transform a server subgraph response into a Cytoscape element array.
 *
 * Pure: same input → same output; no DOM / Cytoscape API access.
 */
export function toCytoscapeElements(
	nodes: GraphNode[],
	edges: GraphEdge[]
): CytoscapeElement[] {
	const nodeById = new Map(nodes.map((n) => [n.id, n] as const));

	const nodeElements: CytoscapeNodeElement[] = nodes.map((n) => {
		const { label, fullTitle, dateStrip } = deriveDisplay(n);
		return {
			group: 'nodes',
			data: {
				id: n.id,
				type: n.doc_type,
				aggregator: n.aggregator,
				label,
				fullTitle,
				dateStrip,
				edges: n.edge_count,
				sessions: n.session_count,
				fontSize: nodeFontSize(n),
				widthPx: nodeWidthPx(n, label.length),
				heightPx: nodeHeightPx(n),
				fill: nodeColor(n.doc_type)
			},
			classes: [`type-${n.doc_type}`, n.aggregator ? 'aggregator' : 'participant'].join(' ')
		};
	});

	const edgeElements: CytoscapeEdgeElement[] = edges.map((e, i) => {
		const sourceNode = nodeById.get(e.source);
		return {
			group: 'edges',
			data: {
				id: `e${i}`,
				source: e.source,
				target: e.target,
				type: e.edge_type,
				sourceFill: sourceNode ? nodeColor(sourceNode.doc_type) : nodeColor('unknown')
			},
			classes: `etype-${e.edge_type}`
		};
	});

	return [...nodeElements, ...edgeElements];
}
