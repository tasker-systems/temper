import type { PageServerLoad } from './$types';
import { redirect } from '@sveltejs/kit';
import { parseRef } from '$lib/ref';

/**
 * Legacy context-shaped resource URL. Resolution was always trailing-UUID-only,
 * so the owner/context/doc_type segments never carried meaning — and presuming a
 * context home left 533 cogmap-homed resources unaddressable (spec D1). Alias to
 * the home-agnostic route; existing links and bookmarks keep working.
 */
export const load: PageServerLoad = async ({ params }) => {
	redirect(303, `/vault/r/${parseRef(params.ident)}`);
};
