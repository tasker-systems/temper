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
	 * @see internal/superpowers/specs/2026-08-20-graph-successor-surface-design.md §3
	 */
	import { onDestroy, onMount } from 'svelte';
	import { type Camera, attachCamera } from '$lib/graph/atlas/camera';
	import { type LabelCandidate, placeLabels } from '$lib/graph/atlas/labels';
	import { forceNeighborhood } from '$lib/graph/atlas/layout/forceNeighborhood';
	import { CANVAS_BG, paletteStyleVars } from '$lib/graph/atlas/palette';
	import type { GraphModel } from '$lib/graph/model';
	import { nodeMeta, nodeRadius } from '$lib/graph/presentation';
	import Edge from '$lib/components/graph/atlas/marks/Edge.svelte';
	import NodeChip from '$lib/components/graph/atlas/marks/NodeChip.svelte';

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

	// The reader's own material holds the core; what a walk reached rings it. Keyed on the ARM
	// rather than on `home`, because on this surface both homes are ordinary — a reader whose
	// corpus is entirely context-homed would otherwise find all of it flung to the outer ring.
	const armById = $derived(new Map(model.nodes.map((n) => [n.id, n.arm])));
	const graph = $derived(
		forceNeighborhood(model, [], {
			width: W,
			height: H,
			coreOf: (n) => armById.get(n.id) !== 'walk',
		}),
	);

	const nodeById = $derived(new Map(model.nodes.map((n) => [n.id, n])));

	// G2 — captions are placed so none lands on another caption or on another node's mark.
	// Every node is still drawn and still hoverable; what is bounded is the always-on label.
	const labels = $derived(
		placeLabels(
			graph.nodes.map(
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
	<svg bind:this={svgEl} viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Your resources and the edges between them">
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

				{#each graph.nodes as n (n.id)}
					{@const node = nodeById.get(n.id)}
					{#if node}
						<NodeChip
							x={n.x}
							y={n.y}
							r={nodeRadius(n.degree)}
							title={n.title}
							docType={n.docType}
							home={n.home}
							seed={node.arm !== 'walk'}
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
					     reader has explicitly asked which this is. -->
					{@const s = graph.nodes.find((n) => n.id === selected)}
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
