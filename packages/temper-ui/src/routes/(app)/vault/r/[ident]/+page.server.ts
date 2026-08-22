import { error } from '@sveltejs/kit';
import { parseRef } from '$lib/ref';
import { ApiError, apiGet } from '$lib/server/api';
import { bounded } from '$lib/server/bounded';
import { readResourceEdges, readTrail } from '$lib/server/graph-reads';
import type { ContentResponse, ResourceView } from '$lib/types';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals, params }) => {
	const accessToken = locals.accessToken!;
	const id = parseRef(params.ident);

	// The three fill reads are started HERE, above the one remaining `await`, so none of them
	// waits on the scaffold. `id` comes from `params`, not from `resource`, so nothing about them
	// needs the resource row first.
	//
	// **None of the three is caught into a value any more, and that is the change.** The file used
	// to apply the right principle to one read of three — "an API error must surface as an error,
	// not render as an empty document" — while degrading the other two to `null` and `[]`. Those
	// degradations asserted *there is nothing here*, which is a claim about the reader's material
	// that nothing verified: a failed trail read was indistinguishable from a resource with no
	// history, and a failed edges read from a resource with no connections. Failure now travels to
	// the template, where `{:catch}` names which region failed.
	//
	// `bounded` supplies the OTHER catch (spec §5.3) on each of these: an unawaited promise that
	// rejects with nothing subscribed is an unhandled rejection and takes the server down. It is
	// not the `{:catch}` that renders the failure, and having one does not give you the other.
	// The gap it closes is real in two places here — between these lines and SvelteKit subscribing
	// in order to serialize, and on the 404 path below, which abandons all three.
	const content = bounded(
		apiGet<ContentResponse>(`/api/resources/${id}/content`, accessToken).then((r) => r.markdown),
		'document',
	);
	const trail = bounded(readTrail(accessToken, 'node', id), 'history');
	const edges = bounded(readResourceEdges(accessToken, id), 'connections');

	// GET /api/resources/{id} returns a ResourceView — the one shape, with both meta
	// tiers filled. Do NOT read the tiers off /content: get_content_select hardcodes
	// both to None (substrate_read.rs). They are dead fields.
	//
	// This is the only `await` left, and it is the scaffold: a resource that is not there is a
	// real 404, not a page frame around three failed regions.
	let resource: ResourceView;
	try {
		resource = await apiGet<ResourceView>(`/api/resources/${id}`, accessToken);
	} catch (err) {
		if (err instanceof ApiError && err.status === 404) throw error(404, 'Resource not found');
		throw err;
	}

	return { resource, content, trail, edges };
};
