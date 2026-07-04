<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { TerritoryOverview } from '$lib/types/generated/graph_territory';
	import type { TeamZone } from '$lib/types/generated/graph_scope';
	import { packTerritories } from '$lib/graph/atlas/layout/packTerritories';
	import { packCogmapTerritories } from '$lib/graph/atlas/layout/cogmapTerritories';
	import { buildScopeUrl, buildDrillTerritoryUrl, buildDrillNodeUrl } from '$lib/graph/atlas/nav';
	import { TERRITORY_TINTS } from '$lib/graph/atlas/palette';
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

	const ZONE_BAND = 120;
	const ZONE_W = 170;
	const ZONE_H = 90;

	const bodyHeight = $derived(Math.max(1, height - ZONE_BAND));
	const packed = $derived(packTerritories(overview.territories, { width, height: bodyHeight }));
	const cogmaps = $derived(packCogmapTerritories(overview.orphan_nodes, { width, height: bodyHeight }));

	function enterZone(teamId: string) {
		goto(buildScopeUrl($page.url, teamId), { replaceState: true });
	}
	function drillTerritory(regionId: string) {
		goto(buildDrillTerritoryUrl($page.url, regionId), { replaceState: true });
	}
	function drillNode(nodeId: string) {
		goto(buildDrillNodeUrl($page.url, nodeId), { replaceState: true });
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

<g transform={`translate(0, ${ZONE_BAND})`}>
	<!-- dense territories: regions drill to Tier 1, contexts are inert -->
	{#each packed as t (t.id)}
		<TerritoryCircle
			x={t.x}
			y={t.y}
			r={t.r}
			kind={t.kind}
			label={t.label}
			onEnter={t.kind === 'region' ? () => drillTerritory(t.id) : undefined}
		/>
	{/each}

	<!-- sparse state: region-less cogmaps drawn as territories with clickable facet dots -->
	{#each cogmaps as cm (cm.cogmapId)}
		<g class="cogmap-territory">
			<circle cx={cm.x} cy={cm.y} r={cm.r} fill={TERRITORY_TINTS.cogmap} fill-opacity="0.06" stroke={TERRITORY_TINTS.cogmap} stroke-opacity="0.4" stroke-width="1.5" stroke-dasharray="6 4" />
			<text x={cm.x} y={cm.y - cm.r - 6} text-anchor="middle" fill={TERRITORY_TINTS.cogmap} font-size="11" font-weight="600" letter-spacing="1" style="text-transform:uppercase">{cm.label}</text>
			{#each cm.facets as f (f.id)}
				<OrphanNodeMark x={f.x} y={f.y} r={Math.min(7, f.r)} title={f.title} docType={f.docType} onEnter={() => drillNode(f.id)} />
			{/each}
		</g>
	{/each}
</g>
