import type { Profile, Entitlements } from '$lib/types';

declare global {
	namespace App {
		interface Locals {
			/**
			 * OIDC user identity from the id_token claims.
			 * Null when the request has no valid session cookie.
			 */
			user: {
				sub: string;
				email: string | null;
				name: string | null;
				picture: string | null;
			} | null;

			/**
			 * Bearer token to send to `/api/*` calls. Auto-refreshed in
			 * hooks.server.ts when the cached token is within 60 seconds of expiry.
			 */
			accessToken: string | null;

			/**
			 * The temper Profile resolved from `GET /api/profile` after auth.
			 * Null until the user has authenticated and the profile has been fetched.
			 */
			profile: Profile | null;

			/**
			 * Entitlements returned alongside the profile — drives the
			 * system-access gate and admin route gating.
			 */
			entitlements: Entitlements | null;
		}
	}
}

export {};
