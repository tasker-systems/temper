<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import cytoscape, { type Core } from 'cytoscape';
	// @ts-expect-error — cytoscape-fcose ships no .d.ts; safe at runtime.
	import fcose from 'cytoscape-fcose';

	import type { GraphEdge, GraphNode } from '$lib/types/generated/graph';
	import { toCytoscapeElements } from '$lib/graph/elements';
	import { defaultFcoseConfig } from '$lib/graph/layout';
	import { buildStylesheet } from '$lib/graph/styling';
	import ResourcePeek from './ResourcePeek.svelte';

	// Register fcose once per page — cytoscape tolerates re-registration but
	// only the first call binds the layout name.
	let fcoseRegistered = false;
	function registerFcose() {
		if (fcoseRegistered) return;
		cytoscape.use(fcose);
		fcoseRegistered = true;
	}

	interface Props {
		nodes: GraphNode[];
		edges: GraphEdge[];
		owner: string;
		context: string;
	}

	let { nodes, edges, owner, context }: Props = $props();

	let containerEl: HTMLDivElement | undefined = $state();
	let cy: Core | undefined;

	// Right-docked peek state — a stack of node ids representing the drill
	// path. Empty array = peek closed. The *current* focused node is always
	// the last entry. A fresh tap on the graph replaces the trail; drilling
	// via a peek row appends; a breadcrumb click slices back to that depth.
	let peekTrail: string[] = $state([]);
	const peekNode = $derived.by(() => {
		if (peekTrail.length === 0) return null;
		const currentId = peekTrail[peekTrail.length - 1];
		return nodes.find((n) => n.id === currentId) ?? null;
	});

	// Animate the camera to center on a given node. 380ms ease-in-out matches
	// kg-handoff.md's peek behavior spec.
	function recenterTo(id: string) {
		if (!cy) return;
		const target = cy.$id(id);
		if (!target || target.length === 0) return;
		cy.animate({
			center: { eles: target },
			zoom: Math.max(cy.zoom(), 0.9),
			duration: 380,
			easing: 'ease-in-out'
		});
	}

	/** Fresh tap on a graph node — reset the trail with this node as its root. */
	function openPeek(id: string) {
		peekTrail = [id];
		recenterTo(id);
	}

	/** Row-click from the peek — drill deeper by pushing onto the trail. */
	function drillInto(id: string) {
		// Guard against no-op clicks on the already-current node.
		if (peekTrail[peekTrail.length - 1] === id) return;
		peekTrail = [...peekTrail, id];
		recenterTo(id);
	}

	/** Breadcrumb click — slice back to the given depth. */
	function sliceTrail(depth: number) {
		const sliced = peekTrail.slice(0, depth + 1);
		if (sliced.length === 0) return;
		peekTrail = sliced;
		recenterTo(sliced[sliced.length - 1]);
	}

	function closePeek() {
		peekTrail = [];
	}

	onMount(() => {
		if (!containerEl) return;
		registerFcose();

		const elements = toCytoscapeElements(nodes, edges);

		cy = cytoscape({
			container: containerEl,
			elements,
			minZoom: 0.25,
			maxZoom: 3,
			wheelSensitivity: 0.2,
			style: buildStylesheet(),
			layout: defaultFcoseConfig()
		});

		// Node tap — open the peek on that node and recenter.
		cy.on('tap', 'node', (evt) => {
			openPeek(evt.target.id());
		});

		// Background tap (target === cy) — close the peek.
		cy.on('tap', (evt) => {
			if (evt.target === cy) closePeek();
		});

		// fcose occasionally quiesces in an alpha-0 state until a user
		// interaction forces a render. Kick a paint after layoutstop and a
		// short fallback in case layoutstop fired synchronously.
		const forcePaint = () => {
			requestAnimationFrame(() => {
				cy?.fit(undefined, 100);
			});
		};
		cy.one('layoutstop', forcePaint);
		setTimeout(forcePaint, 50);
		setTimeout(forcePaint, 300);
	});

	onDestroy(() => {
		cy?.destroy();
		cy = undefined;
	});
</script>

<div class="relative h-full w-full bg-neutral-950">
	<div bind:this={containerEl} class="absolute inset-0" data-testid="knowledge-graph"></div>

	{#if peekNode}
		<ResourcePeek
			node={peekNode}
			{nodes}
			{edges}
			{owner}
			{context}
			trail={peekTrail}
			onClose={closePeek}
			onFocus={drillInto}
			onCrumbClick={sliceTrail}
		/>
	{/if}
</div>
