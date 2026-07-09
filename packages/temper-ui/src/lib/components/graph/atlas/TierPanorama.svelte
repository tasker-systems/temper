<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { TerritoryOverview } from '$lib/types/generated/graph_territory';
	import type { ResidualGroups } from '$lib/types/generated/graph_context';
	import { forceTerritories } from '$lib/graph/atlas/layout/forceTerritories';
	import { intensityOf, labeledRegionIds, territoryWeight } from '$lib/graph/atlas/labels';
	import { packCogmapTerritories } from '$lib/graph/atlas/layout/cogmapTerritories';
	import { bridgeGeometry } from '$lib/graph/atlas/layout/bridges';
	import {
		buildDrillTerritoryUrl,
		buildDrillTerritoriesUrl,
		buildDrillNodeUrl
	} from '$lib/graph/atlas/nav';
	import { TERRITORY_TINTS, isDocTypeDimmed } from '$lib/graph/atlas/palette';
	import { isEmptyTerritory } from '$lib/graph/atlas/territory';
	import TerritoryCircle from './marks/TerritoryCircle.svelte';
	import OrphanNodeMark from './marks/OrphanNodeMark.svelte';
	import BridgeRibbon from './marks/BridgeRibbon.svelte';
	import ResidualTray from './ResidualTray.svelte';

	interface Props {
		overview: TerritoryOverview;
		width: number;
		height: number;
		/** Doc-types to keep at full opacity; empty = no dimming (Task 8, visual-only). */
		docTypes?: string[];
		/**
		 * Beat E context door: the residue that reaches no container, rendered as a tray
		 * doorway. Absent for region/cogmap panoramas, so the tray never renders there.
		 */
		residual?: ResidualGroups | null;
	}
	let { overview, width, height, docTypes = [], residual = null }: Props = $props();

	/** Height reserved along the bottom for the residual-tray doorway. */
	const TRAY_H = 84;

	const hasTerr = $derived(overview.territories.length > 0);
	const hasCogmaps = $derived(overview.orphan_nodes.length > 0);
	const terrBox = $derived(hasTerr && hasCogmaps ? { width, height: height * 0.55 } : { width, height });
	const cogmapBox = $derived(hasTerr && hasCogmaps ? { width, height: height * 0.45 } : { width, height });
	const cogmapOffsetY = $derived(hasTerr && hasCogmaps ? height * 0.55 : 0);
	// Force-separated layout + field-effect: weight → size + glow/opacity so the panorama
	// reads as a field; labels gated to the top-K weightiest territories.
	//
	// Weight is kind-agnostic: regions carry a normalized `salience`, contexts/cogmaps carry
	// a raw `member_count`. Gating the field on `kind === 'region'` left every context
	// territory unlabelled, unglowing, and flat — the panorama only ever worked for regions.
	//
	// Counts get a `log1p` ramp before `intensityOf`'s expansive `^1.4`. Member counts are
	// heavy-tailed (one goal at 108, the median near 3), so feeding the raw ratio to `^1.4`
	// drives every ordinary goal to the opacity floor and the field reads as dead grey.
	// `log1p` compresses the head so small territories stay legible; 0 still maps to 0, so
	// empty containers keep ghost-rendering. Regions are unaffected — `salience` is already
	// normalized and skips this branch.
	const packed = $derived(forceTerritories(overview.territories, terrBox));
	const LABEL_MAX = 10;
	const maxWeight = $derived(Math.max(0.0001, ...packed.map(territoryWeight)));
	const labeledIds = $derived(
		labeledRegionIds(
			packed.map((t) => ({ id: t.id, salience: territoryWeight(t) })),
			LABEL_MAX
		)
	);
	const coherenceById = $derived(new Map(overview.territories.map((t) => [t.id, t.coherence])));
	const cogmaps = $derived(packCogmapTerritories(overview.orphan_nodes, cogmapBox));
	const territoryPos = $derived(new Map(packed.map((t) => [t.id, { x: t.x, y: t.y }])));
	const bridgeLines = $derived(bridgeGeometry(overview.bridges, territoryPos));

	// Beat D multi-region union: shift-click toggles regions into a pending selection
	// (stays on the panorama); the "Explore N regions" button commits the union. A
	// plain click still drills into a single region.
	let unionSel = $state<string[]>([]);
	function toggleUnion(regionId: string) {
		unionSel = unionSel.includes(regionId)
			? unionSel.filter((id) => id !== regionId)
			: [...unionSel, regionId];
	}

	// Drill is a drill step — PUSH history so browser Back walks the path
	// (Atlas ← cogmap ← territory ← node). See nav.ts.
	function drillTerritory(regionId: string, kind: string, shift: boolean) {
		if (shift && kind === 'region') {
			toggleUnion(regionId);
		} else {
			goto(buildDrillTerritoryUrl($page.url, regionId));
		}
	}
	function commitUnion() {
		if (unionSel.length > 0) goto(buildDrillTerritoriesUrl($page.url, unionSel));
	}
	function drillNode(nodeId: string) {
		goto(buildDrillNodeUrl($page.url, nodeId));
	}
</script>

<g>
	<!-- aggregate bridges: render beneath territory circles -->
	{#each bridgeLines as bl, i (i)}
		<BridgeRibbon x1={bl.x1} y1={bl.y1} x2={bl.x2} y2={bl.y2} edgeCount={bl.edgeCount} />
	{/each}

	<!-- dense territories: regions drill to Tier 1 -->
	{#each packed as t (t.id)}
		<TerritoryCircle
			x={t.x}
			y={t.y}
			r={t.r}
			kind={t.kind}
			label={t.label}
			memberCount={t.member_count}
			onEnter={(o) => drillTerritory(t.id, t.kind, o.shift)}
			selected={unionSel.includes(t.id)}
			ghost={isEmptyTerritory(t)}
			showLabel={labeledIds.has(t.id)}
			intensity={intensityOf(territoryWeight(t), maxWeight)}
			salience={t.salience}
			coherence={coherenceById.get(t.id) ?? null}
		/>
	{/each}

	<!-- sparse state: region-less cogmaps drawn as territories with clickable facet dots -->
	<g transform={`translate(0, ${cogmapOffsetY})`}>
		{#each cogmaps as cm (cm.cogmapId)}
			<g class="cogmap-territory">
				<circle cx={cm.x} cy={cm.y} r={cm.r} fill={TERRITORY_TINTS.cogmap} fill-opacity="0.06" stroke={TERRITORY_TINTS.cogmap} stroke-opacity="0.4" stroke-width="1.5" stroke-dasharray="6 4" />
				<text x={cm.x} y={cm.y - cm.r - 6} text-anchor="middle" fill={TERRITORY_TINTS.cogmap} font-size="11" font-weight="600">{cm.label}</text>
				{#each cm.facets as f (f.id)}
					<OrphanNodeMark
						x={f.x}
						y={f.y}
						r={Math.min(7, f.r)}
						title={f.title}
						docType={f.docType}
						dim={isDocTypeDimmed(f.docType, docTypes)}
						onEnter={() => drillNode(f.id)}
					/>
				{/each}
			</g>
		{/each}
	</g>

	<!-- Union commit affordance: appears while regions are shift-selected. -->
	{#if unionSel.length > 0}
		<g
			class="union-commit atlas-focusable"
			role="button"
			tabindex="0"
			aria-label={`Explore ${unionSel.length} selected ${unionSel.length === 1 ? 'region' : 'regions'} together`}
			onclick={commitUnion}
			onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), commitUnion())}
			style="cursor:pointer"
		>
			<rect
				x={width / 2 - 96}
				y={14}
				width={192}
				height={30}
				rx={15}
				fill="#1b2733"
				stroke="#cfd6e2"
				stroke-opacity="0.55"
				stroke-width="1.5"
			/>
			<text
				x={width / 2}
				y={33}
				text-anchor="middle"
				fill="#e6ebf2"
				font-size="13"
				font-weight="600"
			>
				Explore {unionSel.length} {unionSel.length === 1 ? 'region' : 'regions'} →
			</text>
		</g>
	{/if}

	<!--
		The residual tray is page chrome — an HTML doorway, not a field landmark. It is
		mounted via a foreignObject here because TierPanorama is instantiated inside
		AtlasCanvas's camera-transformed <g>, so it is the only render hook available in
		this task's scope. See the component comment and the Task 8 report: the tray
		presently pans/scales with the d3 camera; lifting it to a true HTML sibling of the
		<svg> (so it stays fixed) is a one-line move in AtlasCanvas, out of Task 8's files.
	-->
	{#if residual && residual.buckets.length > 0}
		<foreignObject x="0" y={height - TRAY_H} width={width} height={TRAY_H}>
			<ResidualTray buckets={residual.buckets} groupKey={residual.group_key} {width} />
		</foreignObject>
	{/if}
</g>
