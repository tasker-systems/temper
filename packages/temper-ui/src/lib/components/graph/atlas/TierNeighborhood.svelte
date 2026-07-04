<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';
	import { forceNeighborhood } from '$lib/graph/atlas/layout/forceNeighborhood';
	import { buildDrillNodeUrl } from '$lib/graph/atlas/nav';
	import { EDGE_COLORS } from '$lib/graph/atlas/palette';
	import NodeChip from './marks/NodeChip.svelte';
	import Edge from './marks/Edge.svelte';

	interface Props {
		subgraph: AtlasSubgraph;
		seedId: string;
		width: number;
		height: number;
	}
	let { subgraph, seedId, width, height }: Props = $props();

	const graph = $derived(forceNeighborhood(subgraph, [seedId], { width, height }));
	let hoveredEdge = $state<number | null>(null);

	function drill(nodeId: string) {
		goto(buildDrillNodeUrl($page.url, nodeId), { replaceState: true });
	}

	function nodeRadius(degree: number): number {
		return 8 + Math.min(10, degree);
	}
</script>

<defs>
	<marker id="arrow-end" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
		<path d="M0,0 L10,5 L0,10 z" fill={EDGE_COLORS.structural} />
	</marker>
	<marker id="arrow-start" viewBox="0 0 10 10" refX="1" refY="5" markerWidth="7" markerHeight="7" orient="auto">
		<path d="M10,0 L0,5 L10,10 z" fill={EDGE_COLORS.structural} />
	</marker>
</defs>

{#each graph.edges as e, i (i)}
	<g role="presentation" onmouseenter={() => (hoveredEdge = i)} onmouseleave={() => (hoveredEdge = null)}>
		<Edge x1={e.source.x} y1={e.source.y} x2={e.target.x} y2={e.target.y} edge={e.edge} label={hoveredEdge === i} />
	</g>
{/each}

{#each graph.nodes as n (n.id)}
	<NodeChip x={n.x} y={n.y} r={nodeRadius(n.degree)} title={n.title} docType={n.docType} home={n.home} seed={n.isSeed} onEnter={() => drill(n.id)} />
{/each}
