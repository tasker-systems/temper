/**
 * The web's appearance: where the reader's theme preference is kept and how a page is served in
 * it. What a preference means — follow the system within a family, or hold one theme — is the
 * design package's (`@tasker-systems/temper-design/theme`), the same model the desktop resolves;
 * this module only keeps it in a cookie and writes it onto the document.
 *
 * **A cookie, not localStorage**, so the server can render the page in the reader's theme and a
 * light reader never sees a dark first paint. A fixed preference resolves on the server. "Follow
 * the system" cannot — the server does not know the reader's appearance — so the page carries both
 * candidates and `app.html`'s inline script picks one before first paint.
 *
 * **Marketing is always Quiet Instrument.** Its pages paint from the web's own retired names,
 * designed for dark; served in a light theme they would take a light ground under dark-designed
 * copy. Ruled 2026-10-22: marketing stays dark-only.
 */
import {
	coercePreference,
	resolveTheme,
	type ThemePreference,
} from '@tasker-systems/temper-design/theme';

export const THEME_COOKIE = 'temper.theme';

/** The theme every marketing page is served in, whatever the reader prefers. */
export const MARKETING_THEME = 'quiet-instrument';

/** One year: an appearance is a standing preference, not a session fact. */
export const THEME_COOKIE_MAX_AGE = 60 * 60 * 24 * 365;

/** `system:<family>` or `fixed:<name>` — a cookie value with nothing to escape. Pure. */
export function encodePreference(pref: ThemePreference): string {
	return pref.follow === 'system' ? `system:${pref.family}` : `fixed:${pref.name}`;
}

/** The preference a cookie holds. Absent or unrecognisable reads as the default. Pure. */
export function decodePreference(raw: string | null | undefined): ThemePreference {
	const at = raw?.indexOf(':') ?? -1;
	if (!raw || at < 0) return coercePreference(null);
	const follow = raw.slice(0, at);
	const value = raw.slice(at + 1);
	return coercePreference(
		follow === 'system' ? { follow, family: value } : { follow, name: value },
	);
}

/** Whether a route is a marketing page: the `(public)` group and the access request. Pure. */
export function isMarketingRoute(routeId: string | null): boolean {
	return (
		routeId !== null && (routeId.startsWith('/(public)') || routeId.startsWith('/request-access'))
	);
}

/**
 * The `data-theme` a page is served with, and — when the preference follows the system and the
 * family's two appearances differ — the light candidate the inline script switches to. Pure.
 */
export function pageTheme(
	pref: ThemePreference,
	marketing: boolean,
): { theme: string; light: string | null } {
	if (marketing) return { theme: MARKETING_THEME, light: null };
	const dark = resolveTheme(pref, true);
	if (pref.follow === 'fixed') return { theme: dark, light: null };
	const light = resolveTheme(pref, false);
	return { theme: dark, light: light === dark ? null : light };
}

/** The attribute `app.html` is written with, and the string that replaces it. */
const SERVED_ATTRIBUTE = `data-theme="${MARKETING_THEME}"`;

/** `app.html`'s root attribute rewritten for one page. Pure. */
export function withPageTheme(
	html: string,
	served: { theme: string; light: string | null },
): string {
	const attrs = served.light
		? `data-theme="${served.theme}" data-theme-light="${served.light}"`
		: `data-theme="${served.theme}"`;
	return html.replace(SERVED_ATTRIBUTE, attrs);
}
