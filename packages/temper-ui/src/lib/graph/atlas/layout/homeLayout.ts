/**
 * Home field layout (Beat B): map each lens's members to `Territory`s and lay them
 * out with the shared, deterministic `forceTerritories`. Build = your contexts
 * (sized by resource count); research = the cogmaps you can reach (sized by region
 * count). Pure; the visual field-effect + per-scope tint live in TierHome.
 */
import type { AtlasHome } from '$lib/types/generated/graph_home';
import type { Territory } from '$lib/types/generated/graph_territory';
import { forceTerritories } from './forceTerritories';
import type { PositionedTerritory } from './packTerritories';

export function buildLensTerritories(home: AtlasHome): Territory[] {
	return home.build.map((c) => ({
		id: c.id,
		kind: 'context',
		label: c.name,
		member_count: c.resource_count,
		salience: null,
		coherence: null,
		anchor_id: c.id
	}));
}

export function researchLensTerritories(home: AtlasHome): Territory[] {
	return home.research.map((m) => ({
		id: m.id,
		kind: 'cogmap',
		label: m.name,
		member_count: m.region_count,
		salience: null,
		coherence: null,
		anchor_id: m.id
	}));
}

/** Named seam over `forceTerritories` (one lens at a time). Deterministic. */
export function layoutHomeLens(
	territories: Territory[],
	size: { width: number; height: number }
): PositionedTerritory[] {
	return forceTerritories(territories, size);
}
