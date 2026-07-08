// graph-reads.ts
/**
 * Server-only wrappers for the Atlas reads R1–R5 (+ teams list). These use apiGet/
 * apiPost, which read the encrypted session token — so this module may be imported
 * ONLY from `.server.ts` / `+server.ts`. Path builders are pure and unit-tested;
 * the async wrappers are thin pass-throughs.
 */
import { apiGet, apiPost } from '$lib/server/api';
import type { AtlasSubgraph, SliceRequest } from '$lib/types/generated/graph_atlas';
import type { EventTrail, ElementKind } from '$lib/types/generated/element_trail';
import type { AtlasHome } from '$lib/types/generated/graph_home';
import type { TeamRow } from '$lib/types/generated/team';
import type { TerritoryOverview, TerritorySlice } from '$lib/types/generated/graph_territory';
import type { ResourceRow } from '$lib/types/generated/resource';

export const atlasHomePath = (): string => '/api/graph/home';

export const cogmapPanoramaPath = (id: string, lensId?: string): string =>
	`/api/graph/cogmaps/${id}/panorama${lensId ? `?lens_id=${lensId}` : ''}`;

export const regionSlicePath = (regionId: string): string => `/api/graph/regions/${regionId}/slice`;

/** Beat D — composition drill over one or more regions (comma-joined ids). */
export const regionCompositionPath = (ids: string[], depth = 1): string =>
	`/api/graph/regions/composition?ids=${ids.join(',')}&depth=${depth}`;

export const cogmapNeighborhoodSlicePath = (cogmapId: string): string =>
	`/api/cogmaps/${cogmapId}/graph/slice`;

export const trailPath = (kind: ElementKind, id: string): string =>
	`/api/graph/elements/${kind}/${id}/trail`;

export const teamsListPath = (): string => `/api/teams`;

export const resourceRowPath = (id: string): string => `/api/resources/${id}`;

export const readAtlasHome = (token: string): Promise<AtlasHome> =>
	apiGet<AtlasHome>(atlasHomePath(), token);

export const readCogmapPanorama = (
	token: string,
	id: string,
	lensId?: string
): Promise<TerritoryOverview> => apiGet<TerritoryOverview>(cogmapPanoramaPath(id, lensId), token);

export const readRegionSlice = (token: string, regionId: string): Promise<TerritorySlice> =>
	apiGet<TerritorySlice>(regionSlicePath(regionId), token);

export const readRegionComposition = (
	token: string,
	ids: string[],
	depth = 1
): Promise<AtlasSubgraph> => apiGet<AtlasSubgraph>(regionCompositionPath(ids, depth), token);

export const readCogmapNeighborhood = (
	token: string,
	cogmapId: string,
	req: SliceRequest
): Promise<AtlasSubgraph> => apiPost<AtlasSubgraph>(cogmapNeighborhoodSlicePath(cogmapId), token, req);

export const readTrail = (token: string, kind: ElementKind, id: string): Promise<EventTrail> =>
	apiGet<EventTrail>(trailPath(kind, id), token);

export const listTeams = (token: string): Promise<TeamRow[]> =>
	apiGet<TeamRow[]>(teamsListPath(), token);

export const readResourceRow = (token: string, id: string): Promise<ResourceRow> =>
	apiGet<ResourceRow>(resourceRowPath(id), token);
