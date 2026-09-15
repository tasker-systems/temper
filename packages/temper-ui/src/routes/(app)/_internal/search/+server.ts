import { json } from '@sveltejs/kit';
import { apiGet } from '$lib/server/api';
import type { RequestHandler } from './$types';

export const GET: RequestHandler = async ({ url, locals }) => {
	const q = url.searchParams.get('q') ?? '';
	if (!q.trim()) {
		return json({ rows: [], total: 0 });
	}

	try {
		const result = await apiGet(
			`/api/resources?q=${encodeURIComponent(q)}&limit=10`,
			locals.accessToken!,
		);
		return json(result);
	} catch (err) {
		console.error('search proxy failed', {
			// Caller-chosen content at caller-chosen length — bounded before it reaches the log
			// stream, same as the OIDC callback's query parameters.
			q: q.slice(0, 128),
			err,
		});
		// A failure must stay distinguishable from a successful empty answer: the body is not
		// rows-shaped, so a client that checks the status never mistakes this for a result.
		return json({ error: 'search unavailable' }, { status: 503 });
	}
};
