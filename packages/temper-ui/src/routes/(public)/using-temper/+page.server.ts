import { APP_BASE_URL } from '$lib/server/oidc';
import type { PageServerLoad } from './$types';

/**
 * Expose this instance's public origin so the MCP connection example reflects
 * the actual deployment rather than a hardcoded temperkb.io. `APP_BASE_URL` is
 * the operator-configured `APP_URL`; it falls back to the canonical origin when
 * unset (dev/local).
 */
export const load: PageServerLoad = () => ({
	appUrl: APP_BASE_URL || 'https://temperkb.io',
});
