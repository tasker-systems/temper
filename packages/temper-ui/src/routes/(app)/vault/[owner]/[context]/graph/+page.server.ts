import type { PageServerLoad } from './$types';
import { redirect } from '@sveltejs/kit';
import { contextGraphHref } from '$lib/vault-url';

// Beat E: the legacy Cytoscape context graph moved to the Atlas context door
// (`/graph/[owner]?context=<slug>`). This 308 keeps old `/vault/<owner>/<slug>/graph`
// bookmarks working until Task 12 deletes this route wholesale. 308 (not 303) so the
// method and the permanence are preserved — this is a durable relocation, not a one-off.
export const load: PageServerLoad = async ({ params }) => {
	throw redirect(308, contextGraphHref(params.owner, params.context));
};
