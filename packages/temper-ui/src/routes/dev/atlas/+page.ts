// +page.ts — dev-only Atlas render harness loader.
//
// Loads real-shaped JSON fixtures (captured from prod, see README.md) and hands
// them to the harness page so Atlas layout/legibility can be iterated in-branch
// without auth or a merge-to-prod. 404s outside `dev` so the route is inert in
// any deployed build; the fixtures live under `static/dev/` (gitignored) and are
// never bundled into the client.
import { dev } from '$app/environment';
import { error } from '@sveltejs/kit';
import type { AtlasViewData } from '$lib/graph/atlas/viewData';
import type { PageLoad } from './$types';

export interface AtlasFixtureBundle {
	_meta?: { captured_from?: string; team?: string; note?: string };
	[scenario: string]: AtlasViewData | AtlasFixtureBundle['_meta'];
}

export const load: PageLoad = async ({ fetch }) => {
	if (!dev) throw error(404, 'Not found');

	const res = await fetch('/dev/atlas-fixtures.json');
	if (!res.ok) {
		throw error(
			500,
			'Atlas fixtures missing at static/dev/atlas-fixtures.json — regenerate them (see src/routes/dev/atlas/README.md).'
		);
	}
	const fixtures = (await res.json()) as AtlasFixtureBundle;
	return { fixtures };
};
