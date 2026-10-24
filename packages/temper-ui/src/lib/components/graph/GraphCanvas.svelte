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
	import {
		edgeGeometry,
		frame,
		nodeRadius,
		pairEnds,
		pairStroke,
		placeCaptions,
		settle,
		truncate,
	} from '@tasker-systems/temper-design/graph';
	import { CANVAS_BG } from '$lib/graph/palette';
	import type { GraphEdge, GraphModel } from '$lib/graph/model';
	import { armsDistinguish, nodeMeta } from '$lib/graph/presentation';
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

	/**
	 * The shared geometry (`@tasker-systems/temper-design/graph`): what this answer connected is
	 * placed, what it did not is returned to be listed beneath, and every relation between one pair
	 * is joined onto one line. Radial runs only when the view holds both a core and what was
	 * reached from it — the entry read, all one arm, settles as force rather than being packed
	 * into the centre.
	 *
	 * The edges carry their row, so the pair's relations can be named in the read's own words.
	 */
	const settled = $derived(
		settle(
			model.nodes.map((n) => ({ id: n.id, core: !reached(n.id) })),
			model.edges.map((e) => ({
				source: e.source,
				target: e.target,
				edgeKind: e.edge_kind,
				polarity: e.polarity,
				label: e.label,
				weight: e.weight,
				row: e,
			})),
			'radial',
			{ width: W, height: H },
		),
	);

	// The ring encodes a contrast between arms. Where every mark shares one — which is every entry
	// read, since `buildEntryGraph` has no arms to tell apart — there is no contrast to encode and
	// the channel is not spent.
	const rings = $derived(armsDistinguish(model.nodes));

	const nodeById = $derived(new Map(model.nodes.map((n) => [n.id, n])));

	/**
	 * Every mark on the canvas — which is now every node this answer **connected**, and only those.
	 *
	 * Kept as its own list rather than read off `settled.nodes` at each use because label placement,
	 * the selected-node fallback and the hover card all ask it the same question, and one shape for
	 * one question is what kept the band from ever becoming a second kind of mark while it existed.
	 *
	 * `degree` is the geometry's — distinct neighbours, the lines drawn — and sizes the mark.
	 * `edges` is the model's count of relations, which the hover card says.
	 */
	const marks = $derived(
		settled.nodes.flatMap((p) => {
			const n = nodeById.get(p.id);
			return n
				? [
						{
							id: p.id,
							x: p.x,
							y: p.y,
							degree: p.degree,
							edges: n.degree,
							title: n.title,
							docType: n.doc_type,
							home: n.home,
						},
					]
				: [];
		}),
	);
	const markById = $derived(new Map(marks.map((m) => [m.id, m])));
	const unconnected = $derived(
		settled.unconnected.flatMap((id) => {
			const n = nodeById.get(id);
			return n ? [n] : [];
		}),
	);

	/**
	 * Where a line ends at a mark, along the line's direction: past a circle's rim, past a square's
	 * edge (which reaches √2 further at its corner), and past the arm ring when the mark carries
	 * one, so a head is never drawn over by the mark or the ring it points at.
	 */
	const rimAt = (id: string, dx: number, dy: number): number => {
		const m = markById.get(id);
		if (!m) return 0;
		const r = nodeRadius(m.degree);
		const len = Math.hypot(dx, dy) || 1;
		const shape = m.home === 'cogmap' ? r : r / Math.max(Math.abs(dx) / len, Math.abs(dy) / len);
		return Math.max(shape, rings && !reached(id) ? r + 6 : 0) + 1.5;
	};

	/** A relation in words: the read's label (or its kind), and its weight or that none was read. */
	const relation = (e: GraphEdge): string =>
		`${e.label ?? e.edge_kind} · ${e.weight == null ? 'weight not reported' : `weight ${e.weight.toFixed(2)}`}`;

	const lines = $derived(
		settled.edges.map((pair) => {
			const dx = pair.target.x - pair.source.x;
			const dy = pair.target.y - pair.source.y;
			return {
				key: `${pair.source.id}|${pair.target.id}`,
				geometry: edgeGeometry(pair.source, pair.target, pairEnds(pair.edges), {
					source: rimAt(pair.source.id, dx, dy),
					target: rimAt(pair.target.id, -dx, -dy),
				}),
				stroke: pairStroke(pair.edges),
				relations: pair.edges.map(({ edge }) => relation(edge.row)),
			};
		}),
	);

	// G2 — captions are placed so none lands on another caption or on another node's mark.
	// Every node is still drawn and still hoverable; what is bounded is the always-on label.
	const labels = $derived(
		placeCaptions(
			marks.map((n) => ({ id: n.id, x: n.x, y: n.y, degree: n.degree, label: n.title })),
			{ max: 14 },
		),
	);
	const labelled = $derived(new Set(labels.map((l) => l.id)));

	/**
	 * The viewBox: the plane, grown to take in every mark and caption the layout placed. Nodes
	 * settle where the forces put them, not inside the plane, so a spread-out answer would
	 * otherwise lose its edges off the canvas before the reader had panned. Never smaller than the
	 * plane, so a small answer is not blown up to fill it.
	 */
	const box = $derived.by(() => {
		const f = frame(settled.nodes, labels, 24, { width: W, height: H });
		return {
			x: Math.min(0, f.x1),
			y: Math.min(0, f.y1),
			w: Math.max(W, f.x2) - Math.min(0, f.x1),
			h: Math.max(H, f.y2) - Math.min(0, f.y1),
		};
	});

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

<div class="graph-canvas">
	<svg bind:this={svgEl} viewBox={`${box.x} ${box.y} ${box.w} ${box.h}`} role="img" aria-label={ariaLabel}>
		<rect x={box.x} y={box.y} width={box.w} height={box.h} style:fill={CANVAS_BG} />
		<g bind:this={viewportEl}>
			{#if marks.length === 0}
				<!-- Two different facts, two different sentences: nothing came back at all, versus this
				     answer came back and connected none of it. The second used to be unreachable, because
				     the band drew those marks here. -->
				<text x={W / 2} y={H / 2} text-anchor="middle" style="fill: var(--tp-text-subtle)" font-size="14"
					>{model.nodes.length === 0 ? emptyMessage : nothingConnected}</text
				>
			{:else}
				<!-- Edges first, so a mark is never hidden under a stroke. One line per pair, keyed on
				     the pair; every relation between the two rides it and is named on hover. -->
				{#each lines as l, i (l.key)}
					<g
						role="presentation"
						onmouseenter={() => (hoveredEdge = i)}
						onmouseleave={() => (hoveredEdge = null)}
					>
						<Edge
							{...l.geometry}
							stroke={l.stroke}
							relations={l.relations}
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
							style:fill={l.id === selected ? 'var(--tp-text)' : 'var(--tp-text-muted)'}>{l.text}</text
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
							style="fill: var(--tp-text)">{truncate(s.title, 28)}</text
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
							edges={h.edges}
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
	<UnconnectedBand nodes={unconnected} total={model.nodes.length} {onSelect} {selected} />
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
		stroke: var(--tp-surface-raised);
		stroke-width: 3px;
		stroke-linejoin: round;
	}
</style>
