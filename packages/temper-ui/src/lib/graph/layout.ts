/**
 * Cytoscape fcose layout configuration — lifted verbatim from the prototype
 * (ui_kits/app/KnowledgeGraph.jsx) so the production graph reads identically.
 *
 * fcose's tuning vocabulary:
 *   - `idealEdgeLength`   : target edge length in px (higher = looser)
 *   - `nodeRepulsion`     : repulsion between every pair of nodes
 *   - `edgeElasticity`    : how hard edges pull connected nodes together
 *   - `gravity`           : pull toward the centroid (keeps clusters on-canvas)
 *   - `nodeSeparation`    : minimum breathing room between nodes
 *   - `packComponents`    : lay out disconnected components in a tiled grid
 *   - `quality: 'proof'`  : more iterations, deterministic final placement
 */
export interface FcoseConfig {
	name: 'fcose';
	animate: boolean;
	fit: boolean;
	padding: number;
	randomize: boolean;
	idealEdgeLength: number;
	nodeRepulsion: number;
	edgeElasticity: number;
	gravity: number;
	gravityRange: number;
	gravityCompound: number;
	numIter: number;
	tile: boolean;
	nodeSeparation: number;
	packComponents: boolean;
	quality: 'proof' | 'default' | 'draft';
	aspectRatio: number;
}

/** Default (prototype-matching) fcose config. */
export function defaultFcoseConfig(): FcoseConfig {
	return {
		name: 'fcose',
		animate: false,
		fit: true,
		padding: 100,
		randomize: true,
		idealEdgeLength: 180,
		nodeRepulsion: 25_000,
		edgeElasticity: 0.35,
		gravity: 0.15,
		gravityRange: 5.0,
		gravityCompound: 1.2,
		numIter: 3500,
		tile: false,
		nodeSeparation: 180,
		packComponents: true,
		quality: 'proof',
		aspectRatio: 1.8
	};
}
