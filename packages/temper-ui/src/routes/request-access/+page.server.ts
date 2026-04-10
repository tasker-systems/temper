/**
 * /request-access — landing page for users who are authenticated but do not
 * yet have system access.
 *
 * This route deliberately lives OUTSIDE the (app) layout group because the
 * (app) gate redirects unapproved users HERE. Putting it inside (app) would
 * cause an infinite redirect loop.
 *
 * The load function requires a session (else → /auth/login) but does NOT
 * require system_access. If the user already has system access (e.g. they
 * navigate here directly after approval), bounce them to /vault/all.
 *
 * Form actions:
 *   - default `submit` → POST /api/access/requests
 *   - `withdraw`       → DELETE /api/access/requests/me
 */

import type { PageServerLoad, Actions } from './$types';
import { fail, redirect } from '@sveltejs/kit';
import { apiGet, apiPost, apiDelete, ApiError } from '$lib/server/api';
import type { JoinRequest, PublicSystemSettings } from '$lib/types';

export const load: PageServerLoad = async ({ locals, url }) => {
	if (!locals.user || !locals.accessToken) {
		const returnTo = encodeURIComponent(url.pathname);
		throw redirect(303, `/auth/login?returnTo=${returnTo}`);
	}

	if (locals.entitlements?.system_access) {
		throw redirect(303, '/vault/all');
	}

	const [ownRequest, settings] = await Promise.all([
		apiGet<JoinRequest | null>('/api/access/requests/me', locals.accessToken).catch(
			(err: unknown) => {
				if (err instanceof ApiError) return null;
				throw err;
			}
		),
		apiGet<PublicSystemSettings>('/api/access/settings', locals.accessToken).catch(
			(err: unknown) => {
				if (err instanceof ApiError) return null;
				throw err;
			}
		)
	]);

	return {
		user: locals.user,
		profile: locals.profile,
		ownRequest,
		settings
	};
};

export const actions: Actions = {
	submit: async ({ request, locals }) => {
		if (!locals.accessToken) {
			throw redirect(303, '/auth/login?returnTo=/request-access');
		}

		const form = await request.formData();
		const message = (form.get('message') ?? '').toString().trim();
		const acceptedTerms = form.get('accepted_terms') === 'on';
		const termsVersion = (form.get('terms_version') ?? '').toString();

		if (!acceptedTerms) {
			return fail(400, { error: 'You must accept the terms to request access.', message });
		}

		try {
			await apiPost<JoinRequest>('/api/access/requests', locals.accessToken, {
				message: message || null,
				source: 'web',
				accepted_terms_version: termsVersion || null
			});
		} catch (err) {
			if (err instanceof ApiError) {
				return fail(err.status, {
					error: err.message,
					message
				});
			}
			throw err;
		}

		// Successful submit — fall through to a redirect so the load runs
		// fresh and the page renders the "pending" state.
		throw redirect(303, '/request-access');
	},

	withdraw: async ({ locals }) => {
		if (!locals.accessToken) {
			throw redirect(303, '/auth/login?returnTo=/request-access');
		}

		try {
			await apiDelete('/api/access/requests/me', locals.accessToken);
		} catch (err) {
			if (err instanceof ApiError) {
				return fail(err.status, { error: err.message });
			}
			throw err;
		}

		throw redirect(303, '/request-access');
	}
};
