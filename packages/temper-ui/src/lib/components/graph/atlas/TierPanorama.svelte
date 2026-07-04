<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { TerritoryOverview } from '$lib/types/generated/graph_territory';
	import type { TeamZone } from '$lib/types/generated/graph_scope';
	import { packTerritories } from '$lib/graph/atlas/layout/packTerritories';
	import { buildScopeUrl } from '$lib/graph/atlas/nav';
	import TerritoryCircle from './marks/TerritoryCircle.svelte';
	import TeamZoneMark from './marks/TeamZoneMark.svelte';
	import OrphanNodeMark from './marks/OrphanNodeMark.svelte';

	interface Props {
		overview: TerritoryOverview;
		zones: TeamZone[];
		width: number;
		height: number;
	}
	let { overview, zones, width, height }: Props = $props();

	// Zones occupy a top band; territories pack the rest.
	const ZONE_BAND = 120;
	const ZONE_W = 170;
	const ZONE_H = 90;

	const packed = $derived(
		packTerritories(overview.territories, { width, height: Math.max(1, height - ZONE_BAND) })
	);

	function enterZone(teamId: string) {
		goto(buildScopeUrl($page.url, teamId), { replaceState: true });
	}
</script>

<!-- team-DAG zones (enterable, membership-gated) -->
{#each zones as zone, i (zone.id)}
	<TeamZoneMark
		x={10 + i * (ZONE_W + 14)}
		y={16}
		width={ZONE_W}
		height={ZONE_H}
		name={zone.name}
		resourceCount={zone.resource_count}
		onEnter={() => enterZone(zone.id)}
	/>
{/each}

<!-- this scope's own territories -->
<g transform={`translate(0, ${ZONE_BAND})`}>
	{#each packed as t (t.id)}
		<TerritoryCircle x={t.x} y={t.y} r={t.r} kind={t.kind} label={t.label} />
	{/each}

	<!-- sparsity fallback: orphan salient nodes drawn directly -->
	{#each overview.orphan_nodes as o, i (o.id)}
		<OrphanNodeMark x={40} y={20 + i * 22} title={o.title} docType={o.doc_type} />
	{/each}
</g>
