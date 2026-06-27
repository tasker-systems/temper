import type { PageServerLoad } from './$types';
import { apiGet } from '$lib/server/api';
import type { SubgraphResponse } from '$lib/types/generated/graph';

export const load: PageServerLoad = async ({ locals, params }) => {
	// Build the decorated context ref from the route segments.
	// [owner] is already in sigil form (@me, @handle, +team-slug) and [context] is the slug,
	// so the decorated ref is `<owner>/<context>`. The API resolves this server-side via
	// parse_context_ref → resolve_context_ref (visibility-gated) rather than accepting a
	// bare context name.
	const contextRef = `${params.owner}/${params.context}`;
	const query = new URLSearchParams({ context_ref: contextRef });
	const subgraph = await apiGet<SubgraphResponse>(
		`/api/graph/subgraph?${query}`,
		locals.accessToken!
	);

	return {
		owner: params.owner,
		context: params.context,
		subgraph
	};
};
