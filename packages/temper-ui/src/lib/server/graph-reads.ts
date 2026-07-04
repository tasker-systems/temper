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
import type { TeamScopeView } from '$lib/types/generated/graph_scope';
import type { TeamRow } from '$lib/types/generated/team';
import type { TerritoryOverview, TerritorySlice } from '$lib/types/generated/graph_territory';

export const teamScopePath = (teamId: string): string => `/api/teams/${teamId}/graph-scope`;

export const territoriesPath = (teamId: string, lensId: string | null): string =>
	lensId
		? `/api/teams/${teamId}/graph/territories?lens_id=${encodeURIComponent(lensId)}`
		: `/api/teams/${teamId}/graph/territories`;

export const regionSlicePath = (regionId: string): string => `/api/graph/regions/${regionId}/slice`;

export const neighborhoodSlicePath = (teamId: string): string => `/api/teams/${teamId}/graph/slice`;

export const trailPath = (kind: ElementKind, id: string): string =>
	`/api/graph/elements/${kind}/${id}/trail`;

export const teamsListPath = (): string => `/api/teams`;

export const readTeamScope = (token: string, teamId: string): Promise<TeamScopeView> =>
	apiGet<TeamScopeView>(teamScopePath(teamId), token);

export const readTerritories = (
	token: string,
	teamId: string,
	lensId: string | null
): Promise<TerritoryOverview> => apiGet<TerritoryOverview>(territoriesPath(teamId, lensId), token);

export const readRegionSlice = (token: string, regionId: string): Promise<TerritorySlice> =>
	apiGet<TerritorySlice>(regionSlicePath(regionId), token);

export const readNeighborhood = (
	token: string,
	teamId: string,
	req: SliceRequest
): Promise<AtlasSubgraph> => apiPost<AtlasSubgraph>(neighborhoodSlicePath(teamId), token, req);

export const readTrail = (token: string, kind: ElementKind, id: string): Promise<EventTrail> =>
	apiGet<EventTrail>(trailPath(kind, id), token);

export const listTeams = (token: string): Promise<TeamRow[]> =>
	apiGet<TeamRow[]>(teamsListPath(), token);
