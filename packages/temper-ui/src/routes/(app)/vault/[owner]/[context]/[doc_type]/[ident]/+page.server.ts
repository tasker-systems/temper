import type { PageServerLoad } from './$types';
import { error } from '@sveltejs/kit';
import { apiGet, ApiError } from '$lib/server/api';
import { parseRef } from '$lib/ref';
import type { ResourceRow, ContentResponse } from '$lib/types';

export const load: PageServerLoad = async ({ locals, params }) => {
	const accessToken = locals.accessToken!;

	// Resolve by id from the decorated ref in the `[ident]` segment (trailing-UUID-only).
	// The owner/context/doc_type segments stay in the URL for presentation only.
	const id = parseRef(params.ident);

	let resource: ResourceRow;
	try {
		resource = await apiGet<ResourceRow>(`/api/resources/${id}`, accessToken);
	} catch (err) {
		if (err instanceof ApiError && err.status === 404) {
			throw error(404, 'Resource not found');
		}
		throw err;
	}

	let content = '';
	try {
		const contentRes = await apiGet<ContentResponse>(
			`/api/resources/${resource.id}/content`,
			accessToken
		);
		content = contentRes.markdown;
	} catch {
		// Content may not be available yet (not synced); show empty state
	}

	return {
		resource,
		content
	};
};
