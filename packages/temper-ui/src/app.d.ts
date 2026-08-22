import type { Profile, Entitlements } from '$lib/types';

declare global {
	namespace App {
		/**
		 * What a rejection becomes on its way to the browser.
		 *
		 * SvelteKit sanitises every thrown or rejected error to `{ message: 'Internal Error' }` before
		 * it crosses, and runs `handleError` first so an app may add to that shape. `gaveUp` is the
		 * one addition: `[found — 2026-08-21]` without it a read the system **stopped waiting for**
		 * and a read that **failed** arrive at a client `{:catch}` as the same object, so the region
		 * renders the same words for both and the refusal has no reader.
		 *
		 * A field and not a class, deliberately — a prototype does not survive serialisation. It
		 * carries the region's label rather than `true` so the value names *which* read the system
		 * declined to keep waiting for, which is the whole content of spec §5.4's refusal.
		 *
		 * Written by `describeFailure` in `$lib/server/bounded`; read by `regionStateFor` in
		 * `$lib/region`.
		 */
		interface Error {
			message: string;
			gaveUp?: string;
		}

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
