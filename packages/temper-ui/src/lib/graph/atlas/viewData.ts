// viewData.ts
/**
 * The shape of a single Atlas view — the object returned by the `/graph/[owner]`
 * page load and consumed by `AtlasPage.svelte`. Extracting it as a named type lets
 * the real route AND the dev render-harness (`/dev/atlas`) render the exact same
 * shell from the exact same data shape, so layout/legibility fixes are verified in
 * the harness and ship to the live page with no drift.
 */
import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';
import type { EventTrail } from '$lib/types/generated/element_trail';
import type { HomeCogmap, HomeTeam, LensedHome } from '$lib/types/generated/graph_home';
import type { ResourceRow } from '$lib/types/generated/resource';
import type { TeamScopeView } from '$lib/types/generated/graph_scope';
import type { TerritoryOverview, TerritorySlice } from '$lib/types/generated/graph_territory';
import type { Focus, GraphFilters, SelectedElement } from './nav';

/** Breadcrumb label for the focused territory hop (see `crumbTerritory` in load).
 *  `label` mirrors the region slice's nullable label. */
export interface CrumbTerritory {
	id: string;
	label: string | null;
}

export interface AtlasViewData {
	owner: string;
	teamId: string | null;
	cogmapId: string | null;
	cogmapName: string | null;
	scope: TeamScopeView | null;
	tier: number;
	focus: Focus;
	teams: HomeTeam[] | null;
	cogmaps: HomeCogmap[] | null;
	// SPIKE (Beat B): lensed build/research Home + committed lens. Optional so the
	// non-home page-load branches need no change during the spike (Task 7 formalizes).
	home?: LensedHome | null;
	homeLens?: 'build' | 'research' | null;
	territories: TerritoryOverview | null;
	slice: TerritorySlice | null;
	neighborhood: AtlasSubgraph | null;
	selection: SelectedElement;
	trail: EventTrail | null;
	resourceRow: ResourceRow | null;
	filters: GraphFilters;
	focusPath: Focus[];
	crumbTerritory: CrumbTerritory | null;
}
