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
			locals.accessToken!
		);
		return json(result);
	} catch {
		return json({ rows: [], total: 0 }, { status: 503 });
	}
};
