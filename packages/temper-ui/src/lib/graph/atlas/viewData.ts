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
import type { AtlasHome } from '$lib/types/generated/graph_home';
import type { ResourceRow } from '$lib/types/generated/resource';
import type { TerritoryOverview } from '$lib/types/generated/graph_territory';
import type { Focus, GraphFilters, SelectedElement } from './nav';

/** Breadcrumb label for the focused territory hop (see `crumbTerritory` in load).
 *  `label` mirrors the region slice's nullable label. */
export interface CrumbTerritory {
	id: string;
	label: string | null;
}

export interface AtlasViewData {
	owner: string;
	cogmapId: string | null;
	cogmapName: string | null;
	tier: number;
	focus: Focus;
	// Atlas Home (Beat B): the build/research footprint. Null on the scoped
	// (team/cogmap) branches, which don't render Home. The committed lens is not
	// carried here — TierHome derives it from the URL (`?home`).
	home: AtlasHome | null;
	territories: TerritoryOverview | null;
	neighborhood: AtlasSubgraph | null;
	selection: SelectedElement;
	trail: EventTrail | null;
	resourceRow: ResourceRow | null;
	filters: GraphFilters;
	focusPath: Focus[];
	crumbTerritory: CrumbTerritory | null;
	// Beat C: the committed Home `?scope` narrow (from `parseScopeFilter`), or null.
	// Meaningful only on the Home branch — the cogmap-door branch carries it too for
	// AtlasViewData shape uniformity, but crumbModel suppresses it once a cogmap is set.
	scopeFilter: string | null;
}
