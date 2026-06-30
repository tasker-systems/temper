/**
 * Server gate for the public marketing storefront.
 *
 * The `(public)` route group (landing, manifesto, theory, cognitive-maps,
 * operating, …) is the storefront for the canonical temperkb.io deployment.
 * App-only self-hosted installs disable it via `STOREFRONT_ENABLED=false`
 * (see `$lib/server/storefront`).
 *
 * When disabled, every route in the group — including the landing page at `/` —
 * redirects to `/auth/login`, the app entrypoint: authenticated users land in
 * the vault, everyone else enters the OIDC login flow. No source edits and no
 * per-page nav changes are needed; the Nav/Footer chrome lives only on these
 * pages, so gating the group removes them too.
 */

import type { LayoutServerLoad } from './$types';
import { redirect } from '@sveltejs/kit';
import { storefrontEnabled } from '$lib/server/storefront';

export const load: LayoutServerLoad = () => {
	if (!storefrontEnabled()) {
		throw redirect(307, '/auth/login');
	}
	return {};
};
