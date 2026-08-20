import { redirect } from '@sveltejs/kit';
import { contextGraphHref } from '$lib/vault-url';
import type { PageServerLoad } from './$types';

// The legacy Cytoscape context graph moved to the Atlas context door, and Beat B moved it again
// when it rebuilt `/graph/[owner]` on the four params: `contextGraphHref` now emits
// `/graph/[owner]?in=ctx:<ref>`. This shim forwards through whatever that authority currently
// says rather than spelling a destination out, so old `/vault/<owner>/<slug>/graph` bookmarks
// survived both hops without this file changing. 308 (not 303) so the method and the permanence
// are preserved — this is a durable relocation, not a one-off.
export const load: PageServerLoad = async ({ params }) => {
	throw redirect(308, contextGraphHref(params.owner, params.context));
};
