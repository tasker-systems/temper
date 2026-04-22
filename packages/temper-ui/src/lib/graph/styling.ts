import type { EdgeType, GraphNode } from '../types/generated/graph';

/**
 * Knowledge-graph doctype palette.
 *
 * Mirrors `--graph-*` in `app.css` and `--color-graph-*` in the Tailwind
 * `@theme` block. If you change a color here, change both CSS references —
 * the hex is duplicated across the three because Cytoscape inline styles,
 * CSS vars, and Tailwind utilities each need the raw value.
 *
 * Palette lifted from the kg-handoff legend table:
 *   research — `#8cc5e2` (cool blue)
 *   task     — `#f0a870` (warm orange)
 *   goal     — `#f5d277` (amber, aggregator)
 *   concept  — `#d89ccb` (warm pink, aggregator)
 *   decision — `#c9923c` (aggregator, aligned with goal)
 *
 * `session` has no visible color because sessions are annotations, not
 * nodes — they render as a small green `⌊N⌋` glyph beside other nodes.
 * The `sessionGlyph` color slot is the only place the session hue lives.
 *
 * `memory` is reserved for an upcoming doctype covering builder-and-agent
 * tooling conventions. The slot is wired up now so the renderer is ready
 * when the Rust `DocType::Memory` variant lands.
 */
const NODE_COLORS: Record<string, string> = {
	research: '#8cc5e2',
	task: '#f0a870',
	session: '#9ed3af',
	concept: '#d89ccb',
	goal: '#f5d277',
	decision: '#c9923c',
	memory: '#8e9fc7'
};

const FALLBACK_COLOR = '#9ca3af';

/** Color for a node based on its doc type. Used by both the graph and peek. */
export function nodeColor(docType: string): string {
	return NODE_COLORS[docType] ?? FALLBACK_COLOR;
}

/** The session-annotation glyph color (`⌊N⌋`). */
export const SESSION_GLYPH_COLOR = NODE_COLORS.session;

// ─── Typographic sizing ─────────────────────────────────────────────────────

/** Font size in px for a node label. */
export function nodeFontSize(node: GraphNode): number {
	if (node.aggregator) return 19;
	return node.edge_count >= 10 ? 14 : 13;
}

/**
 * Rendered node width — sized from the label so the word fills its cell.
 *
 * Aggregators are ellipses with a 180px floor (so the italic label has
 * breathing room inside its gravity-well wash); participants tight-fit.
 */
export function nodeWidthPx(node: GraphNode, labelLength: number): number {
	if (node.aggregator) {
		return Math.max(180, labelLength * 12) + 40;
	}
	return Math.max(60, labelLength * 8);
}

/** Rendered node height — fixed per participant/aggregator class. */
export function nodeHeightPx(node: GraphNode): number {
	return node.aggregator ? 70 : 22;
}

// ─── Cytoscape stylesheet ───────────────────────────────────────────────────

/**
 * Base Cytoscape stylesheet — one rule per selector. Keys match the
 * prototype's `cy.style([...])` block 1:1 so R11's visual verdict carries
 * over verbatim.
 *
 * Per-node data used by these rules (set by `toCytoscapeElements`):
 *   - `label`    : display label (truncated)
 *   - `fill`     : doctype hue
 *   - `fontSize` : 13 | 14 | 19
 *   - `widthPx`  : computed from label length
 *   - `heightPx` : 22 | 70
 *
 * Hover-emphasis classes (toggled by KnowledgeGraph's mouseover handler):
 *   - `node.hovered` — the pointer is directly over this node
 *   - `node.dim`     — some other node is hovered and this one isn't its neighbor
 *   - `edge.incident` — edge touches the hovered node; lifted in its source hue
 *   - `edge.quiet`    — no relation to the hovered node; faded to 3% alpha
 *
 * All hover transitions animate over `EMPHASIS_TRANSITION_MS` so mouseleave
 * returns to steady state within the kg-handoff.md 180ms budget.
 *
 * Cytoscape doesn't support per-edge linear gradients natively. The
 * `edge.incident` rule approximates the source→target gradient with a solid
 * `data(sourceFill)` stroke at full saturation — a proper gradient would
 * require a custom renderer layer and is deferred.
 */
export type CytoscapeStyle = Array<{ selector: string; style: Record<string, unknown> }>;

/** Emphasis transition duration in ms — matches the kg-handoff.md spec. */
export const EMPHASIS_TRANSITION_MS = 180;

export function buildStylesheet(): CytoscapeStyle {
	return [
		// ── Base node: the word IS the node ──────────────────────────────────
		{
			selector: 'node',
			style: {
				label: 'data(label)',
				color: 'data(fill)',
				'font-family': '"Source Serif 4", Georgia, serif',
				'font-size': 'data(fontSize)',
				'font-weight': 500,
				'text-halign': 'center',
				'text-valign': 'center',
				'text-wrap': 'none',
				'background-opacity': 0,
				'border-width': 0,
				width: 'data(widthPx)',
				height: 'data(heightPx)',
				'text-events': 'yes',
				'overlay-opacity': 0,
				opacity: 1,
				'text-opacity': 1,
				'transition-property':
					'opacity, text-opacity, background-opacity, text-background-opacity',
				'transition-duration': EMPHASIS_TRANSITION_MS
			}
		},
		// Aggregators: larger italic serif + soft radial-ish wash. Cytoscape
		// does not do gradients, so we approximate with a low-opacity fill on
		// an oversized ellipse that sits under the label.
		{
			selector: 'node.aggregator',
			style: {
				shape: 'ellipse',
				'background-color': 'data(fill)',
				'background-opacity': 0.05,
				width: 260,
				height: 140,
				'font-size': 19,
				'font-style': 'italic',
				'font-weight': 600
			}
		},
		// Hovered: subtle text-background wash in the node's own hue.
		{
			selector: 'node.hovered',
			style: {
				'text-background-color': 'data(fill)',
				'text-background-opacity': 0.12,
				'text-background-padding': 4
			}
		},
		// Dimmed: another node is hovered and this one isn't incident.
		{
			selector: 'node.dim',
			style: {
				opacity: 0.35
			}
		},
		// ── Edges ────────────────────────────────────────────────────────────
		{
			selector: 'edge',
			style: {
				width: 0.75,
				'line-color': 'rgba(255,255,255,0.10)',
				'curve-style': 'straight',
				'target-arrow-shape': 'none',
				opacity: 1,
				'transition-property': 'opacity, width, line-color',
				'transition-duration': EMPHASIS_TRANSITION_MS
			}
		},
		{ selector: 'edge.etype-depends_on', style: { 'line-style': 'solid' } },
		{ selector: 'edge.etype-extends', style: { 'line-style': 'solid' } },
		{ selector: 'edge.etype-parent_of', style: { 'line-style': 'solid' } },
		{ selector: 'edge.etype-derived_from', style: { 'line-style': 'solid' } },
		{
			selector: 'edge.etype-preceded_by',
			style: { 'line-style': 'dashed', 'line-dash-pattern': [6, 4] }
		},
		{
			selector: 'edge.etype-relates_to',
			style: { 'line-style': 'dashed', 'line-dash-pattern': [3, 3] }
		},
		{ selector: 'edge.etype-references', style: { 'line-style': 'dotted' } },
		// Incident edges (touch the hovered node): lifted to 1.1px in source
		// hue at full saturation — the gradient approximation.
		{
			selector: 'edge.incident',
			style: {
				width: 1.1,
				'line-color': 'data(sourceFill)',
				opacity: 1
			}
		},
		// Quiet edges (not incident on any hovered node): faded to 3% alpha.
		{
			selector: 'edge.quiet',
			style: {
				opacity: 0.03
			}
		},
		// ── Zoom tiers ──────────────────────────────────────────────────────
		//
		// kg-handoff.md § PR 5:
		//   < 0.5  → overview: only aggregators labeled; participants render
		//            as short colored tick marks.
		//     0.5 – 1.2 → mid: steady state.
		//   > 1.2  → detail: labels + date strips (stage tags deferred).
		//
		// Tier transitions are supposed to fade at 220ms. Cytoscape's
		// `transition-duration` is per-element, so we inherit the base
		// node's 180ms (see `EMPHASIS_TRANSITION_MS`) — a 40ms deviation
		// from the design budget in service of keeping one duration.
		// Mid tier has no style rule because it IS the steady state.
		{
			selector: 'node.tier-overview.participant',
			style: {
				'text-opacity': 0,
				'background-opacity': 1,
				'background-color': 'data(fill)',
				shape: 'rectangle',
				width: 12,
				height: 3
			}
		},
		// Detail tier: swap label to the multiline date variant for nodes
		// that have a date strip. Participants and aggregators both opt in.
		{
			selector: 'node.tier-detail[dateStrip]',
			style: {
				label: 'data(labelWithDate)',
				'text-wrap': 'wrap',
				'line-height': 1.2
			}
		},
		// Detail tier + task + stage: stack the stage tag under the label
		// (and date, when present). The combined `labelWithDateAndStage`
		// variant is precomputed in `toCytoscapeElements`, so this rule just
		// picks it up instead of relying on Cytoscape expression templates.
		// Only tasks carry a `stage` attribute — the server guards with a
		// doctype check — so the attribute-existence selector is enough to
		// gate the tag to tasks.
		{
			selector: 'node.tier-detail.type-task[stage]',
			style: {
				label: 'data(labelWithDateAndStage)',
				'text-wrap': 'wrap',
				'line-height': 1.2
			}
		}
	];
}

// ─── Edge-type dash patterns (exposed for the legend/peek to reuse) ────────

const EDGE_DASH: Record<EdgeType, string> = {
	depends_on: '',
	extends: '',
	parent_of: '',
	preceded_by: '6,4',
	relates_to: '3,3',
	derived_from: '',
	references: '2,3'
};

/** SVG-compatible `stroke-dasharray` for an edge type. Empty = solid. */
export function edgeStrokeDasharray(edgeType: EdgeType): string {
	return EDGE_DASH[edgeType] ?? '';
}
