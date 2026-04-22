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
 *
 * `labelWithDate` is the label concatenated with `\n` + `dateStrip` when
 * a date exists — the `node.tier-detail[dateStrip]` rule swaps the visible
 * label to this multiline form at the detail zoom tier.
 *
 * `labelWithDateAndStage` stacks a task's stage below its label/date at the
 * detail zoom tier. Populated only when `stage` is set (tasks only, see
 * `GraphNode.stage` in temper-core) and keyed by the `node.tier-detail[stage]`
 * selector in styling.ts. When both stage and a date strip are present the
 * rendered label reads `{label}\n{dateStrip}\n{STAGE}`; stage-only tasks
 * collapse to `{label}\n{STAGE}`.
 */
export interface CytoscapeNodeElement {
	group: 'nodes';
	data: {
		id: string;
		type: string;
		aggregator: boolean;
		label: string;
		labelWithDate: string | null;
		labelWithDateAndStage: string | null;
		stage: string | null;
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
		const labelWithDate = dateStrip ? `${label}\n${dateStrip}` : null;
		// Stage is task-only server-side; keep it as a plain string for the
		// stylesheet selector to see and as an uppercased segment in the
		// composed label for readability at the detail zoom tier.
		const stage = n.stage ?? null;
		const stageSegment = stage ? stage.toUpperCase() : null;
		const labelWithDateAndStage = stageSegment
			? dateStrip
				? `${label}\n${dateStrip}\n${stageSegment}`
				: `${label}\n${stageSegment}`
			: null;
		return {
			group: 'nodes',
			data: {
				id: n.id,
				type: n.doc_type,
				aggregator: n.aggregator,
				label,
				labelWithDate,
				labelWithDateAndStage,
				stage,
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
