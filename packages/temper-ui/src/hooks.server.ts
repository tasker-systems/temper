import type { Handle } from '@sveltejs/kit';

export const handle: Handle = async ({ event, resolve }) => {
	// Auth middleware — implemented in session 2.
	// For now, locals are null (unauthenticated).
	event.locals.user = null;
	event.locals.accessToken = null;

	return resolve(event);
};
