// graph-reads.ts
/**
 * Server-only wrappers for the reads the vault and graph surfaces still make. These use
 * `apiGet`, which reads the encrypted session token — so this module may be imported ONLY
 * from `.server.ts` / `+server.ts`. Path builders are pure and unit-tested; the async
 * wrappers are thin pass-throughs.
 *
 * **What used to be here, and why it is not** `[Beat D — 2026-08-20]`. This file held the
 * TypeScript callers of the nine bespoke `/api/graph/*` endpoints — home, cogmap panorama,
 * region composition, the cogmap neighborhood slice, and the two context-door reads. The
 * Atlas was their only caller and the Atlas is gone, so the callers went with it. **The
 * endpoints themselves still exist and now have none.** Deleting them is Rust and a separate
 * PR; it is named here so the leftover reads as a known remainder rather than an oversight.
 * See the successor spec §4.2, *Out of scope*.
 */

import { apiGet } from '$lib/server/api';
import type { ElementKind, EventTrail } from '$lib/types/generated/element_trail';
import type { GraphEdgeRow } from '$lib/types/generated/graph';
import type { ResourceView } from '$lib/types/generated/resource_view';
import type { TeamRow } from '$lib/types/generated/team';

export const trailPath = (kind: ElementKind, id: string): string =>
	`/api/graph/elements/${kind}/${id}/trail`;

export const teamsListPath = (): string => `/api/teams`;

export const resourceRowPath = (id: string): string => `/api/resources/${id}`;

export const resourceEdgesPath = (id: string): string => `/api/resources/${id}/edges`;

export const readTrail = (token: string, kind: ElementKind, id: string): Promise<EventTrail> =>
	apiGet<EventTrail>(trailPath(kind, id), token);

export const listTeams = (token: string): Promise<TeamRow[]> =>
	apiGet<TeamRow[]>(teamsListPath(), token);

export const readResourceRow = (token: string, id: string): Promise<ResourceView> =>
	apiGet<ResourceView>(resourceRowPath(id), token);

/** Edges incident to one resource. Rows are peer-denormalized — no subgraph load. */
export const readResourceEdges = (token: string, id: string): Promise<GraphEdgeRow[]> =>
	apiGet<GraphEdgeRow[]>(resourceEdgesPath(id), token);
