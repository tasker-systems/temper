import type { GraphNode } from '../types/generated/graph';

/** Zoom level above which non-concept labels become readable. */
const LABEL_ZOOM_THRESHOLD = 1.5;

/** Truncate a label to `max` chars, appending "…" if cut. */
export function truncateLabel(title: string, max: number): string {
	if (title.length <= max) return title;
	// Reserve one char for the ellipsis.
	return title.slice(0, max - 1) + '…';
}

/** Whether a node's label should render at the given zoom level. */
export function shouldShowLabel(node: GraphNode, zoomLevel: number): boolean {
	if (node.doc_type === 'concept') return true;
	return zoomLevel >= LABEL_ZOOM_THRESHOLD;
}
