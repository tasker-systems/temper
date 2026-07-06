// +page.ts — dev-only Atlas render harness loader.
//
// Loads real-shaped JSON fixtures and hands them to the harness page so Atlas
// layout/legibility can be iterated in-branch without auth or a merge-to-prod.
// 404s outside `dev` so the route is inert in any deployed build.
//
// Two sources, in precedence order:
//   1. `static/dev/atlas-fixtures.local.json` — a raw personal capture (gitignored),
//      if you have one. Lets you eyeball the harness against your own real data.
//   2. `static/dev/atlas-fixtures.json` — the committed, synthetic, personal-data-free
//      bundle. The default; drives the harness on a fresh checkout and in tests.
// See README.md for capture + sanitize workflow.
import { dev } from '$app/environment';
import { error } from '@sveltejs/kit';
import type { AtlasViewData } from '$lib/graph/atlas/viewData';
import type { PageLoad } from './$types';

export interface AtlasFixtureBundle {
	_meta?: { captured_from?: string; team?: string; note?: string; synthetic?: boolean };
	[scenario: string]: AtlasViewData | AtlasFixtureBundle['_meta'];
}

export const load: PageLoad = async ({ fetch }) => {
	if (!dev) throw error(404, 'Not found');

	// Prefer a local capture if present; otherwise fall back to the committed default.
	const local = await fetch('/dev/atlas-fixtures.local.json');
	const res = local.ok ? local : await fetch('/dev/atlas-fixtures.json');
	if (!res.ok) {
		throw error(
			500,
			'Atlas fixtures missing at static/dev/atlas-fixtures.json — the committed bundle should always exist (see src/routes/dev/atlas/README.md).'
		);
	}
	const fixtures = (await res.json()) as AtlasFixtureBundle;
	return { fixtures };
};
