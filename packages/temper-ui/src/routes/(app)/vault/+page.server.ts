import { WAYS_IN } from '@tasker-systems/temper-design/ways-in';
import {
	type FirstViewList,
	openingOf,
	PLACES_SCOPE,
	placesPath,
	RESUME_LABEL,
	recentPath,
	resumeOf,
	resumePath,
	wayInPath,
	wayInShown,
} from '$lib/first-view';
import { apiGet } from '$lib/server/api';
import { bounded, derive } from '$lib/server/bounded';
import type { ContentResponse, ContextActivity, ResourceListResponse } from '$lib/types';
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

	// What the reader last changed. Its time and its order are the same changes the places' times
	// count (`resources_changed_by` and `contexts_changed_by` share one definition), so "your last
	// change" means one thing on this page. Its own read: a failure fails resume alone.
	const resume = bounded(
		apiGet<ResourceListResponse>(resumePath(), accessToken).then((r) => resumeOf(r.rows)),
		RESUME_LABEL,
	);
	// The opening of that work is a second read, of its body, with its own bound that starts when
	// the return arrives. A failed body read fails the opening alone: the return still shows. A
	// failed or empty resume read has no opening to read, so this answers `null` rather than
	// reporting the same failure twice.
	const opening = derive(
		resume.then(
			(r) => r,
			() => null,
		),
		(r) =>
			r
				? bounded(
						apiGet<ContentResponse>(`/api/resources/${r.resource.id}/content`, accessToken).then(
							(c) => openingOf(c.markdown),
						),
						'opening',
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

	// The reader's own latest change in each place. It carries no names: the page names each place
	// from the contexts the layout already sent (`placesInUse`). Joining here would mean awaiting
	// `parent()`, which holds every read below behind the layout's and, on a client-side rerun,
	// re-runs the layout's reads too.
	const places = bounded(apiGet<ContextActivity[]>(placesPath(), accessToken), PLACES_SCOPE);

	const recent = bounded(
		apiGet<ResourceListResponse>(recentPath(), accessToken).then(listOf),
		'recently updated',
	);

	return { resume, opening, places, ways, recent };
};
