<script lang="ts">
	/**
	 * The canvas. **Its entire mark vocabulary is node and edge.**
	 *
	 * There is no second kind to change into, which is how `navigation-never-silently-changes-kind`
	 * is satisfied structurally rather than by careful labelling — adding a third mark would be a
	 * visible, reviewable act, and `model.ts`'s own test fails the moment the model grows a third
	 * collection to draw from.
	 *
	 * The arm a node arrived by is carried by a **ring** around the mark, not by a different mark.
	 * `[ruled — 2026-08-21, Pete]` **The ring encodes the view's standing point**: ringed = what
	 * this view was built from, bare = what following edges reached from it. Which arms are which
	 * is read off `model.arms` — the read declares it — rather than hard-coded here against a
	 * global `'walk'`, which no per-view vocabulary could have satisfied. Shape still carries home
	 * and hue still carries doc-type, exactly as they do on every other screen in the app — so the
	 * arm is a fourth channel on one mark, never a second mark.
	 *
	 * `[ruled — 2026-08-22, Pete]` **Nothing unconnected is drawn here any more.** Post-Beat-0.5, 80
	 * of the flagship answer's 155 nodes have degree zero; they were first given a declared band of
	 * marks beneath the connected core. That band borrowed this canvas's grammar and dropped its
	 * semantics — position here means something, position in a row means nothing — so it is now a
	 * caption and a disclosure list in `UnconnectedBand.svelte`, beneath the drawing rather than in it.
	 *
	 * What this canvas draws is therefore exactly **what this answer connected**, and the core has the
	 * whole height back. A row of that list opens the same rail a mark's click opened.
	 *
	 * @see temper-artifacts:specs/2026-08-20-graph-successor-surface-design.md §3, §7
	 */
	import { onDestroy, onMount } from 'svelte';
	import { type Camera, attachCamera } from '$lib/graph/camera';
	import { type LabelCandidate, placeLabels } from '$lib/graph/labels';
	import { forceNeighborhood } from '$lib/graph/layout/forceNeighborhood';
	import { CANVAS_BG, paletteStyleVars } from '$lib/graph/palette';
	import type { GraphModel } from '$lib/graph/model';
	import {
		armsDistinguish,
		nodeMeta,
		nodeRadius,
		partitionByConnection,
	} from '$lib/graph/presentation';
	import Edge from '$lib/components/graph/marks/Edge.svelte';
	import NodeChip from '$lib/components/graph/marks/NodeChip.svelte';
	import NodeHoverCard from '$lib/components/graph/marks/NodeHoverCard.svelte';
	import UnconnectedBand from './UnconnectedBand.svelte';

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

	// What this answer connected is drawn here; what it did not is listed beneath. The core therefore
	// has the whole canvas on every answer, which is what it had before the band was ever drawn in it.
	const parts = $derived(partitionByConnection(model.nodes));

	// Where this read stood holds the core; what it reached from there rings it. Keyed on the ARM
	// rather than on `home`, because on this surface both homes are ordinary — a reader whose
	// corpus is entirely context-homed would otherwise find all of it flung to the outer ring.
	//
	// Resolved through the read's OWN legend. An arm this model does not declare is treated as
	// standing rather than reached: the two channels below then leave it alone, which is what an
	// unresolvable key should cost — nothing said about it, never a claim made up for it.
	const legend = $derived(new Map(model.arms.map((a) => [a.key, a])));
	const armOfNode = $derived(new Map(model.nodes.map((n) => [n.id, legend.get(n.arm)])));
	const reached = (id: string): boolean => armOfNode.get(id)?.reached === true;
	const graph = $derived(
		forceNeighborhood({ nodes: parts.connected, edges: model.edges }, [], {
			width: W,
			height: H,
			coreOf: (n) => !reached(n.id),
		}),
	);

	// The ring encodes a contrast between arms. Where every mark shares one — which is every entry
	// read, since `buildEntryGraph` has no arms to tell apart — there is no contrast to encode and
	// the channel is not spent.
	const rings = $derived(armsDistinguish(model.nodes));

	const nodeById = $derived(new Map(model.nodes.map((n) => [n.id, n])));

	/**
	 * Every mark on the canvas — which is now every node this answer **connected**, and only those.
	 *
	 * Kept as its own list rather than read off `graph.nodes` at each use because label placement, the
	 * selected-node fallback and the hover card all ask it the same question, and one shape for one
	 * question is what kept the band from ever becoming a second kind of mark while it existed.
	 */
	const marks = $derived(
		graph.nodes.map((n) => ({
			id: n.id,
			x: n.x,
			y: n.y,
			degree: n.degree,
			title: n.title,
			docType: n.docType,
			home: n.home,
		})),
	);

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

	/**
	 * The band's sentence is no longer folded into this label, and that is a repair rather than a
	 * loss: it is real text in the document now, under a `<summary>` a reader can open, instead of a
	 * string smuggled into the label of an `<svg role="img">` that reports nothing inside itself.
	 */
	const ariaLabel = 'Your resources and the edges between them';

	/**
	 * What the canvas says when this answer connected nothing.
	 *
	 * Distinct from `emptyMessage`, which says the reader has nothing here — a different and much
	 * worse claim. `no-reader-is-left-to-blame-themselves`: a blank rectangle above a caption, with
	 * no sentence in it, invites exactly the wrong conclusion. The entry read cannot reach this,
	 * because rung 2 replaces the whole canvas when nothing is eligible; a composition answer has no
	 * such rung and can.
	 */
	const nothingConnected =
		'Nothing here is connected to anything else, so there is no shape to draw — they are all listed below.';

	let hoveredEdge = $state<number | null>(null);
	let svgEl: SVGSVGElement | undefined = $state();
	/**
	 * Which mark the pointer is over. Owned HERE because the card it opens must be drawn after
	 * every mark, and only this component draws every mark.
	 */
	let hoveredId = $state<string | null>(null);
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
			{#if marks.length === 0}
				<!-- Two different facts, two different sentences: nothing came back at all, versus this
				     answer came back and connected none of it. The second used to be unreachable, because
				     the band drew those marks here. -->
				<text x={W / 2} y={H / 2} text-anchor="middle" fill="#7d8496" font-size="14"
					>{model.nodes.length === 0 ? emptyMessage : nothingConnected}</text
				>
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
							ringed={rings && !reached(n.id)}
							anchored={false}
							hovered={hoveredId === n.id}
							onHover={(over) => (hoveredId = over ? n.id : hoveredId === n.id ? null : hoveredId)}
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
					     reader has explicitly asked which this is. A selection made from the list
					     beneath finds no mark here at all, and gets no caption rather than a
					     fabricated position — the rail is what answers for it. -->
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

				<!--
					**The hover card is drawn LAST, and that is the whole fix.** `[found on production —
					2026-08-22]` It used to render inside its own `NodeChip`'s `<g>`, inside the loop
					above, so every mark after it painted over it — a card opened on an early mark was
					buried under most of the graph, one opened on the last mark looked fine.

					SVG has no `z-index`: stacking is document order. That is why the symptom was
					reported for years as a z-index problem and why setting one never changed anything —
					the property being adjusted is not one SVG reads. The captions block directly above
					already states this rule for labels; the card was the one thing breaking it.
				-->
				{#if hoveredId !== null}
					{@const h = marks.find((n) => n.id === hoveredId)}
					{@const hn = h ? nodeById.get(h.id) : undefined}
					{#if h && hn}
						<NodeHoverCard
							x={h.x}
							y={h.y}
							r={nodeRadius(h.degree)}
							title={h.title}
							docType={h.docType}
							edges={h.degree}
							excerpt={hn.excerpt}
							meta={nodeMeta(hn, legend.get(hn.arm))}
						/>
					{/if}
				{/if}
			{/if}
		</g>
	</svg>

	<!-- Beneath the drawing, never in it. The list makes no spatial claim, which is the whole of
	     what the row of marks it replaces got wrong. -->
	<UnconnectedBand nodes={parts.unconnected} total={model.nodes.length} {onSelect} {selected} />
</div>

<style>
	.graph-canvas {
		display: flex;
		/* Column, so the band sits under the drawing rather than beside it. The svg takes what is
		   left after the band has asked for what it needs. */
		flex-direction: column;
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
