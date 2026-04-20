<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import cytoscape, { type Core } from 'cytoscape';
	// @ts-expect-error — cytoscape-fcose ships no .d.ts; safe at runtime.
	import fcose from 'cytoscape-fcose';

	import type { GraphEdge, GraphNode } from '$lib/types/generated/graph';
	import { toCytoscapeElements } from '$lib/graph/elements';
	import { defaultFcoseConfig } from '$lib/graph/layout';
	import { buildStylesheet } from '$lib/graph/styling';

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
	}

	let { nodes, edges }: Props = $props();

	let containerEl: HTMLDivElement | undefined = $state();
	let cy: Core | undefined;

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

<div bind:this={containerEl} class="h-full w-full bg-neutral-950" data-testid="knowledge-graph"></div>
