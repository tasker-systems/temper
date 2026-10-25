import { HANDOFF_LABEL, nextSection } from '@tasker-systems/temper-design/handoff';
import { WAYS_IN } from '@tasker-systems/temper-design/ways-in';
import {
	type FirstViewList,
	handoffPath,
	recentPath,
	wayInPath,
	wayInShown,
} from '$lib/first-view';
import { apiGet } from '$lib/server/api';
import { bounded, derive } from '$lib/server/bounded';
import type { ContentResponse, ResourceListResponse } from '$lib/types';
import type { PageServerLoad } from './$types';

function listOf(response: ResourceListResponse): FirstViewList {
	return { rows: response.rows, total: Number(response.total) };
}

/**
 * The first view (`$lib/first-view`). Every read is started here and streamed, none awaited, so
 * one slow or failed list never holds up or blanks another: each reaches the page as a promise
 * whose failure its own region names. `bounded` is also each one's server-side catch.
 */
export const load: PageServerLoad = async ({ locals, url }) => {
	const accessToken = locals.accessToken!;

	const session = bounded(
		apiGet<ResourceListResponse>(handoffPath(), accessToken).then((r) => r.rows[0] ?? null),
		HANDOFF_LABEL,
	);
	// The next steps are a second read, of the session's body, with its own bound that starts when
	// the session arrives — never the remainder of the session read's. A reader with no session has
	// none to quote, and a session with no "Next" heading quotes nothing: both are `null`, never a
	// summary. A failed body read fails this region alone; the session itself still shows. A failed
	// session read is the handoff region's failure, and this region is never drawn under it, so it
	// answers `null` rather than reporting the same failure twice.
	const next = derive(
		session.then(
			(s) => s,
			() => null,
		),
		(s) =>
			s
				? bounded(
						apiGet<ContentResponse>(`/api/resources/${s.id}/content`, accessToken).then((c) =>
							nextSection(c.markdown),
						),
						'next steps',
					)
				: null,
	);

	const ways = WAYS_IN.map((way) => {
		const shown = wayInShown(url.searchParams, way);
		return {
			way,
			shown,
			list: bounded(
				apiGet<ResourceListResponse>(wayInPath(way, shown), accessToken).then(listOf),
				way.label,
			),
		};
	});

	const recent = bounded(
		apiGet<ResourceListResponse>(recentPath(), accessToken).then(listOf),
		'recently updated',
	);

	return { session, next, ways, recent };
};
