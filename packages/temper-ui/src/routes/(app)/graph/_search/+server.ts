import { json } from '@sveltejs/kit';
import { readAtlasSearch } from '$lib/server/graph-reads';
import type { RequestHandler } from './$types';

export const GET: RequestHandler = async ({ url, locals }) => {
	const team = url.searchParams.get('team');
	const q = url.searchParams.get('q')?.trim() ?? '';
	if (!team || q.length === 0) {
		return json([]);
	}

	try {
		const hits = await readAtlasSearch(locals.accessToken!, team, q);
		return json(hits);
	} catch {
		return json([], { status: 503 });
	}
};
