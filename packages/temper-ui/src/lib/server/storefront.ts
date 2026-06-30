import { env } from '$env/dynamic/private';

/**
 * The public marketing storefront — the `(public)` route group (landing,
 * manifesto, theory, cognitive-maps, operating, …) — is on by default so the
 * canonical temperkb.io deployment keeps working with zero env changes.
 *
 * An app-only self-hosted operator opts OUT by setting `STOREFRONT_ENABLED` to
 * a falsy token. When disabled, `(public)/+layout.server.ts` redirects the whole
 * group (including `/`) to the app entrypoint.
 *
 * Pure for testability — the env read is isolated in {@link storefrontEnabled}.
 */
export function storefrontEnabledFrom(value: string | undefined): boolean {
	if (value === undefined) return true;
	const token = value.trim().toLowerCase();
	return !(token === 'false' || token === '0' || token === 'off' || token === 'no');
}

/** Read the operator-configured `STOREFRONT_ENABLED` flag at runtime. */
export function storefrontEnabled(): boolean {
	return storefrontEnabledFrom(env.STOREFRONT_ENABLED);
}
