import { extractVaultLinks, type LinkResolution } from '@tasker-systems/temper-design/links';
import type { ResourceListResponse } from '$lib/types';
import { apiGet } from './api';

/**
 * Answers for a body's `./<uuid>` links, read in ONE request: the resource list's `ids` filter,
 * which returns the reader's visible, complete resources among the ids and nothing else.
 *
 * What a returned row says is `resolved`, with the title temper holds. An asked id missing from a
 * page that holds every row is `absent` — the filter leaves no other trace of why, by design, so a
 * deleted resource and one the reader may not see read the same. A page that came back
 * `truncated` proves nothing about the ids it did not return, so those get no answer and render
 * unchecked, as does every link when the read fails: this never rejects, because a body whose
 * links could not be checked is still a body.
 *
 * Entries, not a `Map`, so the answer streams through the page's data as plain JSON.
 */
export async function resolveBodyLinks(
	markdown: string,
	accessToken: string,
): Promise<[string, LinkResolution][]> {
	const { ids } = extractVaultLinks(markdown);
	if (ids.length === 0) return [];
	const params = new URLSearchParams({ ids: ids.join(','), limit: String(ids.length) });
	let page: ResourceListResponse;
	try {
		page = await apiGet<ResourceListResponse>(`/api/resources?${params}`, accessToken);
	} catch {
		return [];
	}
	const held = new Map(page.rows.map((row) => [String(row.id).toLowerCase(), row.title]));
	return ids.flatMap((id): [string, LinkResolution][] => {
		const title = held.get(id);
		if (title !== undefined) return [[id, { state: 'resolved', title }]];
		return page.truncated ? [] : [[id, { state: 'absent' }]];
	});
}
