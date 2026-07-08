<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';
	import { forceNeighborhood } from '$lib/graph/atlas/layout/forceNeighborhood';
	import { labelAnchors } from '$lib/graph/atlas/labels';
	import { buildDrillNodeUrl, buildEdgeSelectUrl, buildNodeSelectUrl } from '$lib/graph/atlas/nav';
	import { isDocTypeDimmed } from '$lib/graph/atlas/palette';
	import NodeChip from './marks/NodeChip.svelte';
	import Edge from './marks/Edge.svelte';

	interface Props {
		subgraph: AtlasSubgraph;
		seedId: string;
		width: number;
		height: number;
		/** Doc-types to keep at full opacity; empty = no dimming (Task 8, visual-only). */
		docTypes?: string[];
	}
	let { subgraph, seedId, width, height, docTypes = [] }: Props = $props();

	const graph = $derived(forceNeighborhood(subgraph, [seedId], { width, height }));
	const anchors = $derived(labelAnchors(graph.nodes, seedId, 5));
	let hoveredEdge = $state<number | null>(null);

	// Drill is a drill step — PUSH history so browser Back walks the path (see nav.ts).
	function drill(nodeId: string) {
		goto(buildDrillNodeUrl($page.url, nodeId));
	}

	// Edge selection is ephemeral panel state (?sel), not a scope change — REPLACE
	// so it doesn't clutter the drill history the Back button walks.
	function selectEdge(edgeId: string) {
		goto(buildEdgeSelectUrl($page.url, edgeId), { replaceState: true });
	}

	// A context-resource (builder-axis) node opens its TrailRail via ?sel — a
	// cogmap-scoped drill would fall out of scope and dead-end (Beat D). Facets
	// (cogmap-homed) still drill into their neighborhood.
	function selectNode(nodeId: string) {
		goto(buildNodeSelectUrl($page.url, nodeId), { replaceState: true });
	}
	function activate(node: (typeof graph.nodes)[number]) {
		if (node.home === 'context') selectNode(node.id);
		else drill(node.id);
	}

	function nodeRadius(degree: number): number {
		return 8 + Math.min(10, degree);
	}
</script>

<defs>
	<marker id="arrow-end" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
		<path d="M0,0 L10,5 L0,10 z" fill="context-stroke" />
	</marker>
	<marker id="arrow-start" viewBox="0 0 10 10" refX="1" refY="5" markerWidth="7" markerHeight="7" orient="auto">
		<path d="M10,0 L0,5 L10,10 z" fill="context-stroke" />
	</marker>
</defs>

{#each graph.edges as e, i (i)}
	<g role="presentation" onmouseenter={() => (hoveredEdge = i)} onmouseleave={() => (hoveredEdge = null)}>
		<Edge
			x1={e.source.x}
			y1={e.source.y}
			x2={e.target.x}
			y2={e.target.y}
			edge={e.edge}
			label={hoveredEdge === i}
			onSelect={() => selectEdge(e.edge.id)}
		/>
	</g>
{/each}

{#each graph.nodes as n (n.id)}
	<NodeChip
		x={n.x}
		y={n.y}
		r={nodeRadius(n.degree)}
		title={n.title}
		docType={n.docType}
		home={n.home}
		seed={n.isSeed}
		anchored={anchors.has(n.id)}
		dim={isDocTypeDimmed(n.docType, docTypes)}
		edges={n.degree}
		excerpt={n.excerpt ?? null}
		onEnter={() => activate(n)}
	/>
{/each}
