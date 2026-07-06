<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { TerritorySlice } from '$lib/types/generated/graph_territory';
	import { packRegionMembers } from '$lib/graph/atlas/layout/regionInterior';
	import { hullPath } from '$lib/graph/atlas/layout/hull';
	import { buildDrillNodeUrl } from '$lib/graph/atlas/nav';
	import { TERRITORY_TINTS } from '$lib/graph/atlas/palette';
	import MemberChip from './marks/MemberChip.svelte';

	interface Props {
		slice: TerritorySlice;
		width: number;
		height: number;
	}
	let { slice, width, height }: Props = $props();

	const members = $derived(packRegionMembers(slice.members, { width, height: Math.max(1, height - 60) }));
	const hull = $derived(hullPath(members.map((m) => [m.x, m.y] as [number, number]), 26));

	// Drill is a drill step — PUSH history so browser Back walks the path. See nav.ts.
	function drill(nodeId: string) {
		goto(buildDrillNodeUrl($page.url, nodeId));
	}
</script>

<g transform="translate(0, 40)">
	{#if hull}
		<path d={hull} fill={TERRITORY_TINTS.region} fill-opacity="0.05" stroke={TERRITORY_TINTS.region} stroke-opacity="0.4" stroke-width="1.5" stroke-dasharray="7 5" />
	{/if}
	{#each members as m (m.id)}
		<MemberChip x={m.x} y={m.y} r={m.r} title={m.title} docType={m.docType} onEnter={() => drill(m.id)} />
	{/each}
</g>

<text x="24" y="28" fill={TERRITORY_TINTS.region} font-size="12" font-weight="600" letter-spacing="1">{(slice.label ?? 'REGION').toUpperCase()} · interior</text>
