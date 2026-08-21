<script lang="ts">
	/**
	 * The canvas. **Its entire mark vocabulary is node and edge.**
	 *
	 * There is no second kind to change into, which is how `navigation-never-silently-changes-kind`
	 * is satisfied structurally rather than by careful labelling — adding a third mark would be a
	 * visible, reviewable act, and `model.ts`'s own test fails the moment the model grows a third
	 * collection to draw from.
	 *
	 * The arm a node arrived by is carried by a **ring** around the mark, not by a different mark:
	 * what the reader named in the places they asked about is ringed, what a walk reached is bare.
	 * Shape still carries home and hue still carries doc-type, exactly as they do on every other
	 * screen in the app — so the arm is a fourth channel on one mark, never a second mark.
	 *
	 * `[ruled — 2026-08-20, Pete]` **The unconnected field.** Measured post-Beat-0.5, 80 of the
	 * flagship answer's 155 nodes have degree zero. They used to settle through the same force pass
	 * as everything else and read as a scatter of identical discs. They are now drawn in a declared
	 * band beneath the connected core, captioned in the reader's own words. Every one of them is
	 * still drawn, still hoverable, still the same `.node-chip` mark — the field is a *place on the
	 * canvas*, not a new kind of thing, and `presentation.ts` carries why.
	 *
	 * @see internal/superpowers/specs/2026-08-20-graph-successor-surface-design.md §3, §7
	 */
	import { onDestroy, onMount } from 'svelte';
	import { type Camera, attachCamera } from '$lib/graph/camera';
	import { type LabelCandidate, placeLabels } from '$lib/graph/labels';
	import { forceNeighborhood } from '$lib/graph/layout/forceNeighborhood';
	import { CANVAS_BG, paletteStyleVars } from '$lib/graph/palette';
	import type { GraphModel } from '$lib/graph/model';
	import {
		armsDistinguish,
		describeUnconnected,
		nodeMeta,
		nodeRadius,
		packField,
		partitionByConnection,
	} from '$lib/graph/presentation';
	import Edge from '$lib/components/graph/marks/Edge.svelte';
	import NodeChip from '$lib/components/graph/marks/NodeChip.svelte';

	interface Props {
		model: GraphModel;
		selected: string | null;
		onSelect: (id: string) => void;
		/** Nothing came back. The reason belongs to the caller, which knows which door was used. */
		emptyMessage?: string;
	}
	let { model, selected, onSelect, emptyMessage = 'Nothing to draw here yet.' }: Props = $props();

	const W = 1040;
	const H = 620;
	const MIN_ZOOM = 0.3;
	const MAX_ZOOM = 4;
	/** Height the field claims when there is anything to put in it, caption included. */
	const FIELD_H = 132;
	const FIELD_PAD = 24;
	const CAPTION_H = 30;

	const parts = $derived(partitionByConnection(model.nodes));
	// The core keeps the whole canvas when nothing is unconnected, so a fully-connected answer is
	// laid out exactly as it was before the field existed.
	const coreH = $derived(parts.unconnected.length > 0 ? H - FIELD_H : H);

	// The reader's own material holds the core; what a walk reached rings it. Keyed on the ARM
	// rather than on `home`, because on this surface both homes are ordinary — a reader whose
	// corpus is entirely context-homed would otherwise find all of it flung to the outer ring.
	const armById = $derived(new Map(model.nodes.map((n) => [n.id, n.arm])));
	const graph = $derived(
		forceNeighborhood({ nodes: parts.connected, edges: model.edges }, [], {
			width: W,
			height: coreH,
			coreOf: (n) => armById.get(n.id) !== 'walk',
		}),
	);

	const field = $derived(
		packField(
			// The order the answer returned them in. Placing these is a legibility act and must not
			// become a ranking — §2.3 ruled unranked-everything is the design.
			parts.unconnected.map((n) => n.id),
			{
				x: FIELD_PAD,
				y: coreH + CAPTION_H,
				width: W - 2 * FIELD_PAD,
				height: FIELD_H - CAPTION_H - 8,
			},
		),
	);

	// The band's corpus figures are handed over rather than reached for: the caption must be TOLD
	// what this read measured. `buildGraph` reports `null` for all of them and gets the
	// answer-scoped sentence; `buildEntryGraph` reports real ones and gets the sentence that says
	// what its marks ARE connected to. Neither borrows the other's claim.
	const caption = $derived(
		describeUnconnected(
			parts.unconnected.length,
			model.nodes.length,
			field.undrawn,
			parts.unconnected.map((n) => n.corpusDegree),
		),
	);

	// The ring encodes a contrast between arms. Where every mark shares one — which is every entry
	// read, since `buildEntryGraph` has no arms to tell apart — there is no contrast to encode and
	// the channel is not spent.
	const rings = $derived(armsDistinguish(model.nodes));

	const nodeById = $derived(new Map(model.nodes.map((n) => [n.id, n])));

	/**
	 * Every mark on the canvas, from both placements, in one list.
	 *
	 * One list rather than two loops is the point: the field is a set of coordinates, not a second
	 * kind of mark, and everything downstream — captions, the selected-node fallback, the DOM class
	 * the vocabulary test counts — cannot tell the two apart because there is nothing to tell.
	 */
	const marks = $derived([
		...graph.nodes.map((n) => ({
			id: n.id,
			x: n.x,
			y: n.y,
			degree: n.degree,
			title: n.title,
			docType: n.docType,
			home: n.home,
		})),
		...field.placed.map((p) => {
			const n = nodeById.get(p.id)!;
			return {
				id: p.id,
				x: p.x,
				y: p.y,
				degree: 0,
				title: n.title,
				docType: n.doc_type,
				home: n.home,
			};
		}),
	]);

	// G2 — captions are placed so none lands on another caption or on another node's mark.
	// Every node is still drawn and still hoverable; what is bounded is the always-on label.
	const labels = $derived(
		placeLabels(
			marks.map(
				(n): LabelCandidate => ({
					id: n.id,
					x: n.x,
					y: n.y,
					r: nodeRadius(n.degree),
					title: n.title,
					degree: n.degree,
				}),
			),
		),
	);
	const labelled = $derived(new Set(labels.map((l) => l.id)));

	const ariaLabel = $derived(
		caption
			? `Your resources and the edges between them. ${caption}`
			: 'Your resources and the edges between them',
	);

	let hoveredEdge = $state<number | null>(null);
	let svgEl: SVGSVGElement | undefined = $state();
	let viewportEl: SVGGElement | undefined = $state();
	let camera: Camera | undefined;

	onMount(() => {
		if (svgEl && viewportEl) {
			camera = attachCamera(svgEl, viewportEl, { min: MIN_ZOOM, max: MAX_ZOOM });
		}
	});
	onDestroy(() => camera?.destroy());
</script>

<div class="graph-canvas" style={paletteStyleVars()}>
	<svg bind:this={svgEl} viewBox={`0 0 ${W} ${H}`} role="img" aria-label={ariaLabel}>
		<rect x="0" y="0" width={W} height={H} fill={CANVAS_BG} />
		<defs>
			<marker id="arrow-end" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
				<path d="M0,0 L10,5 L0,10 z" fill="context-stroke" />
			</marker>
			<marker id="arrow-start" viewBox="0 0 10 10" refX="1" refY="5" markerWidth="7" markerHeight="7" orient="auto">
				<path d="M10,0 L0,5 L10,10 z" fill="context-stroke" />
			</marker>
		</defs>
		<g bind:this={viewportEl}>
			{#if model.nodes.length === 0}
				<text x={W / 2} y={H / 2} text-anchor="middle" fill="#7d8496" font-size="14">{emptyMessage}</text>
			{:else}
				<!-- Edges first, so a mark is never hidden under a stroke. Keyed on the four fields
				     that identify the row, which is also the key the model deduped on. -->
				{#each graph.edges as e, i (`${e.edge.source}|${e.edge.target}|${e.edge.edge_kind}|${e.edge.label}`)}
					<g
						role="presentation"
						onmouseenter={() => (hoveredEdge = i)}
						onmouseleave={() => (hoveredEdge = null)}
					>
						<Edge
							x1={e.source.x}
							y1={e.source.y}
							x2={e.target.x}
							y2={e.target.y}
							edge={e.edge}
							label={hoveredEdge === i}
						/>
					</g>
				{/each}

				{#if caption}
					<!-- Chrome, not a mark: a rule and a sentence. Deliberately NOT wrapped in a
					     classed <g> — the canvas's mark vocabulary is two, and saying what a
					     region of the canvas is must not spend a third entry in it. -->
					<line
						x1={FIELD_PAD}
						y1={coreH + 10}
						x2={W - FIELD_PAD}
						y2={coreH + 10}
						stroke="#333a49"
						stroke-width="1"
					/>
					<text
						data-testid="unconnected-caption"
						x={FIELD_PAD}
						y={coreH + 26}
						font-size="11"
						fill="#8b94a5">{caption}</text
					>
				{/if}

				{#each marks as n (n.id)}
					{@const node = nodeById.get(n.id)}
					{#if node}
						<NodeChip
							x={n.x}
							y={n.y}
							r={nodeRadius(n.degree)}
							title={n.title}
							docType={n.docType}
							home={n.home}
							ringed={rings && node.arm !== 'walk'}
							anchored={false}
							edges={n.degree}
							excerpt={node.excerpt}
							meta={nodeMeta(node)}
							onEnter={() => onSelect(n.id)}
						/>
					{/if}
				{/each}

				<!-- Captions last: a label drawn between the node passes would be covered by a
				     later mark, which is the collision G2 exists to prevent, reintroduced by
				     draw order. -->
				<g class="labels" aria-hidden="true">
					{#each labels as l (l.id)}
						<text
							x={l.x}
							y={l.y}
							text-anchor="middle"
							font-size="10"
							fill={l.id === selected ? '#e6edf5' : '#aab4c0'}>{l.text}</text
						>
					{/each}
				</g>

				{#if selected && labelled.has(selected) === false}
					<!-- A selected node the collision pass could not caption still gets one: the
					     reader has explicitly asked which this is. This reaches into the field too,
					     which is exactly why the two placements share one list. -->
					{@const s = marks.find((n) => n.id === selected)}
					{#if s}
						<text
							x={s.x}
							y={s.y + nodeRadius(s.degree) + 12}
							text-anchor="middle"
							font-size="10"
							fill="#e6edf5">{s.title.length > 28 ? `${s.title.slice(0, 27)}…` : s.title}</text
						>
					{/if}
				{/if}
			{/if}
		</g>
	</svg>
</div>

<style>
	.graph-canvas {
		display: flex;
		width: 100%;
		height: 100%;
		min-height: 0;
	}
	.graph-canvas svg {
		display: block;
		width: 100%;
		flex: 1 1 auto;
		min-height: 0;
	}
	.labels text {
		paint-order: stroke;
		stroke: rgba(27, 30, 38, 0.85);
		stroke-width: 3px;
		stroke-linejoin: round;
	}
</style>
