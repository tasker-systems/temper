/**
 * /admin/access — admin queue for system access join requests.
 *
 * Sits inside the (app) layout group, so the parent layout's load already
 * enforced session + system_access. Here we additionally require
 * `entitlements.is_admin` (owner-role member of the temper-system team).
 *
 * Calls `/api/access/admin/requests` (gated; the API also checks is_admin
 * server-side and returns 403 if not — defense in depth).
 *
 * Form actions:
 *   - approve → PATCH /api/access/admin/requests/{id} { status: "approved" }
 *   - reject  → PATCH /api/access/admin/requests/{id} { status: "rejected", decision_note }
 */

import type { PageServerLoad, Actions } from './$types';
import { fail, redirect, error } from '@sveltejs/kit';
import { apiGet, apiPatch, ApiError } from '$lib/server/api';
import type { JoinRequest, JoinRequestWithProfile } from '$lib/types';

export const load: PageServerLoad = async ({ locals }) => {
	if (!locals.entitlements?.is_admin) {
		throw error(404, 'Not found');
	}

	const requests = await apiGet<JoinRequestWithProfile[]>(
		'/api/access/admin/requests',
		locals.accessToken!
	);

	return { requests };
};

async function review(
	accessToken: string | null,
	requestId: string,
	status: 'approved' | 'rejected',
	decisionNote: string | null
) {
	if (!accessToken) {
		throw redirect(303, '/auth/login?returnTo=/admin/access');
	}

	await apiPatch<JoinRequest>(`/api/access/admin/requests/${requestId}`, accessToken, {
		status,
		decision_note: decisionNote
	});
}

export const actions: Actions = {
	approve: async ({ request, locals }) => {
		const form = await request.formData();
		const id = (form.get('id') ?? '').toString();
		if (!id) return fail(400, { error: 'Missing request id' });

		try {
			await review(locals.accessToken, id, 'approved', null);
		} catch (err) {
			if (err instanceof ApiError) return fail(err.status, { error: err.message });
			throw err;
		}

		throw redirect(303, '/admin/access');
	},

	reject: async ({ request, locals }) => {
		const form = await request.formData();
		const id = (form.get('id') ?? '').toString();
		const note = (form.get('decision_note') ?? '').toString().trim();
		if (!id) return fail(400, { error: 'Missing request id' });

		try {
			await review(locals.accessToken, id, 'rejected', note || null);
		} catch (err) {
			if (err instanceof ApiError) return fail(err.status, { error: err.message });
			throw err;
		}

		throw redirect(303, '/admin/access');
	}
};
