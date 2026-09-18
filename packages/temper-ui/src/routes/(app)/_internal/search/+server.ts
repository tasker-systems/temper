import { json } from '@sveltejs/kit';
import { runSearch } from '$lib/server/vault-search';
import type { RequestHandler } from './$types';

/**
 * The palette's proxy onto the real asking door, `POST /api/search` (spec D1, the graph page's
 * `/api/query` precedent). The session token stays server-side; the browser never sees it.
 *
 * The upstream answer is handed through **unchanged** — both arms, each with its own disposition —
 * because arm selection happens in the display, over the answer this door already returned. No
 * `arms` param is sent and none is accepted here: requests stay byte-identical to any current
 * client's.
 */
export const POST: RequestHandler = async ({ request, locals }) => {
	const { q } = (await request.json()) as { q?: string };
	const query = q ?? '';
	if (!query.trim()) {
		// An empty question is declined here rather than read upstream — kept from the route's
		// incumbent GET, which never spent a read on a blank query either. Plain JSON, same
		// body shape as the 503: a caller checking the body shape reads both as non-answers.
		return json({ error: 'empty query' }, { status: 400 });
	}

	try {
		const result = await runSearch(locals.accessToken!, query);
		return json(result);
	} catch (err) {
		console.error('search proxy failed', {
			// Caller-chosen content at caller-chosen length — bounded before it reaches the log
			// stream, same as the OIDC callback's query parameters.
			q: query.slice(0, 128),
			err,
		});
		// A failure must stay distinguishable from a successful empty answer: the body is not
		// arms-shaped, so a client that checks the status never mistakes this for a result.
		return json({ error: 'search unavailable' }, { status: 503 });
	}
};
