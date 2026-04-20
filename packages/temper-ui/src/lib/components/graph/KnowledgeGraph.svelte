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

	// Right-docked peek state. `null` = peek closed. Derived node lookup in
	// the template keeps the state minimal (just the id).
	let peekNodeId: string | null = $state(null);
	const peekNode = $derived(
		peekNodeId === null ? null : (nodes.find((n) => n.id === peekNodeId) ?? null)
	);

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

	function openPeek(id: string) {
		peekNodeId = id;
		recenterTo(id);
	}

	function closePeek() {
		peekNodeId = null;
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
			onClose={closePeek}
			onFocus={openPeek}
		/>
	{/if}
</div>
