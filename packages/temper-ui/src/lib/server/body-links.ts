import { extractVaultLinks, type LinkResolution } from '@tasker-systems/temper-design/links';
import type { ResourceListResponse } from '$lib/types';
import { apiGet } from './api';
import { GIVE_UP_AFTER_MS } from './bounded';

/**
 * How long the link read may take before its links settle unchecked: the page's own give-up
 * bound ({@link GIVE_UP_AFTER_MS}). Nothing waits on it — the body has already painted, its links
 * inert — so it gets the whole bound rather than a share of the document's. The list read computes
 * the reader's whole visible set before the id filter narrows it, so its cost grows with the
 * reader's corpus, not with the number of links.
 */
export const LINK_READ_BUDGET_MS = GIVE_UP_AFTER_MS;

/**
 * Answers for a body's `./<uuid>` links, read in ONE request: the resource list's `ids` filter,
 * which returns the reader's visible, complete resources among the ids and nothing else.
 *
 * What a returned row says is `resolved`, with the title temper holds. An asked id missing from a
 * page that holds every row is `absent` — the filter leaves no other trace of why, by design, so a
 * deleted resource and one the reader may not see read the same. A page that came back
 * `truncated` proves nothing about the ids it did not return, so those get no answer and render
 * unchecked, as does every link when the read fails, answers malformed, or outruns
 * {@link LINK_READ_BUDGET_MS}: this never rejects and never stalls, because a body whose links
 * could not be checked is still a body.
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
	let timer: ReturnType<typeof setTimeout> | undefined;
	const outrun = new Promise<null>((resolve) => {
		timer = setTimeout(() => resolve(null), LINK_READ_BUDGET_MS);
	});
	try {
		const page = await Promise.race([
			apiGet<ResourceListResponse>(`/api/resources?${params}`, accessToken),
			outrun,
		]);
		if (!page) return [];
		const held = new Map(page.rows.map((row) => [String(row.id).toLowerCase(), row.title]));
		return ids.flatMap((id): [string, LinkResolution][] => {
			const title = held.get(id);
			if (title !== undefined) return [[id, { state: 'resolved', title }]];
			return page.truncated ? [] : [[id, { state: 'absent' }]];
		});
	} catch {
		return [];
	} finally {
		clearTimeout(timer);
	}
}
