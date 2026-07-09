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
import type { ContextPanorama } from '$lib/types/generated/graph_context';
import type { TeamRow } from '$lib/types/generated/team';
import type { TerritoryOverview } from '$lib/types/generated/graph_territory';
import type { ResourceRow } from '$lib/types/generated/resource';
import type { Focus } from '$lib/graph/atlas/nav';

export const atlasHomePath = (): string => '/api/graph/home';

export const cogmapPanoramaPath = (id: string, lensId?: string): string =>
	`/api/graph/cogmaps/${id}/panorama${lensId ? `?lens_id=${lensId}` : ''}`;

/** Beat D — composition drill over one or more regions (comma-joined ids). */
export const regionCompositionPath = (ids: string[], depth = 1): string =>
	`/api/graph/regions/composition?ids=${ids.join(',')}&depth=${depth}`;

/**
 * Beat E — a composition drill target: the container or bucket the user drilled. Derived
 * from `Focus` rather than re-declared — nav.ts already owns the `container`/`bucket` shapes,
 * and a second copy would drift from the URL model it must stay identical to. Importing the
 * `Focus` TYPE here is one-directional and erased at build time, so it does not couple this
 * server-only module to the client.
 */
export type CompositionTarget = Extract<Focus, { kind: 'container' } | { kind: 'bucket' }>;

/**
 * The container-membership walk depth. The panorama's residual buckets are computed by walking
 * containers to this depth; a bucket drill must resolve its members at the SAME depth or it
 * seeds a different set than the tray displayed (spec §7). The server defaults BOTH endpoints
 * to this value, and `readContextPanorama` leaves it implicit — so we echo it explicitly as
 * `container_depth` on a bucket target, where a future divergence in either default would
 * otherwise silently desync the tray from its drill.
 */
const CONTAINER_WALK_DEPTH = 2;

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

/** Beat E — the context door. `context_ref` is a decorated ref (`@me/temper`) or a bare
 *  UUID; it contains `/` and `@`, so it MUST be percent-encoded into the query string. */
export const contextPanoramaPath = (ref: string, groupBy = 'doc_type'): string =>
	`/api/graph/contexts/panorama?context_ref=${encodeURIComponent(ref)}&group_by=${encodeURIComponent(groupBy)}`;

export const contextCompositionPath = (ref: string, t: CompositionTarget, depth = 1): string => {
	const base = `/api/graph/contexts/composition?context_ref=${encodeURIComponent(ref)}`;
	if (t.kind === 'container') {
		return `${base}&container=${encodeURIComponent(t.id)}&depth=${depth}`;
	}
	// A bucket carries `container_depth` so its member walk matches the panorama's; a
	// container drill has an explicit seed and needs no such alignment.
	const group = encodeURIComponent(`${t.groupKey}:${t.value}`);
	return `${base}&group=${group}&depth=${depth}&container_depth=${CONTAINER_WALK_DEPTH}`;
};

export const readContextPanorama = (
	token: string,
	ref: string,
	groupBy = 'doc_type'
): Promise<ContextPanorama> => apiGet<ContextPanorama>(contextPanoramaPath(ref, groupBy), token);

export const readContextComposition = (
	token: string,
	ref: string,
	t: CompositionTarget,
	depth = 1
): Promise<AtlasSubgraph> => apiGet<AtlasSubgraph>(contextCompositionPath(ref, t, depth), token);
