import type { PageServerLoad } from './$types';
import { error } from '@sveltejs/kit';
import { apiGet, ApiError } from '$lib/server/api';
import type { ResourceRow, ContentResponse } from '$lib/types';

export const load: PageServerLoad = async ({ locals, params }) => {
	const accessToken = locals.accessToken!;

	let resource: ResourceRow;
	try {
		const queryParams = new URLSearchParams({
			owner: params.owner,
			context: params.context,
			doc_type: params.doc_type,
			ident: params.ident
		});
		resource = await apiGet<ResourceRow>(
			`/api/resources/by-uri?${queryParams}`,
			accessToken
		);
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
