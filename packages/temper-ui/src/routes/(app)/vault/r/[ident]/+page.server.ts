import type { PageServerLoad } from './$types';
import { error } from '@sveltejs/kit';
import { apiGet, ApiError } from '$lib/server/api';
import { readTrail, readResourceEdges } from '$lib/server/graph-reads';
import { parseRef } from '$lib/ref';
import type { ResourceDetail } from '$lib/types/resource-detail';
import type { ContentResponse } from '$lib/types';
import type { EventTrail } from '$lib/types/generated/element_trail';
import type { GraphEdgeRow } from '$lib/types/generated/graph';

export const load: PageServerLoad = async ({ locals, params }) => {
	const accessToken = locals.accessToken!;
	const id = parseRef(params.ident);

	// GET /api/resources/{id} returns ResourceDetail — the row AND both meta
	// tiers. Do NOT read the tiers off /content: get_content_select hardcodes
	// both to None (substrate_read.rs:292-297). They are dead fields.
	let resource: ResourceDetail;
	try {
		resource = await apiGet<ResourceDetail>(`/api/resources/${id}`, accessToken);
	} catch (err) {
		if (err instanceof ApiError && err.status === 404) throw error(404, 'Resource not found');
		throw err;
	}

	// The rail degrades independently: a failure there must not blank the body.
	// The content read is deliberately NOT caught — an API error must surface as
	// an error, not render as an empty document.
	const [content, trail, edges] = await Promise.all([
		apiGet<ContentResponse>(`/api/resources/${id}/content`, accessToken).then((r) => r.markdown),
		readTrail(accessToken, 'node', id).catch((): EventTrail | null => null),
		readResourceEdges(accessToken, id).catch((): GraphEdgeRow[] => [])
	]);

	return { resource, content, trail, edges };
};
