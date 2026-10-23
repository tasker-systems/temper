import {
	DEFAULT_PREFERENCE,
	resolveTheme,
	type ThemePreference,
} from '@tasker-systems/temper-design/theme';
import { browser } from '$app/environment';
import { decodePreference, encodePreference, THEME_COOKIE, THEME_COOKIE_MAX_AGE } from '$lib/theme';

/**
 * The reader's appearance on the authed app, live: what they chose, what the system says, and
 * the theme the two resolve to. The server already rendered the page in that theme
 * (`hooks.server.ts`); this keeps the document in step when the reader chooses again or the
 * system's appearance changes, without a reload.
 *
 * Applied to the root only from the authed app. A marketing page is served dark and paints its
 * own wrapper dark; arriving at the app from one, `init` puts the reader's theme back.
 */

/** The cookie's value in `document.cookie`, or `null`. Pure. */
export function readCookie(cookies: string, name: string): string | null {
	for (const part of cookies.split(';')) {
		const [key, ...rest] = part.trim().split('=');
		if (key === name) return decodeURIComponent(rest.join('='));
	}
	return null;
}

let pref = $state<ThemePreference>(DEFAULT_PREFERENCE);
let systemDark = $state(true);
let media: MediaQueryList | null = null;

function apply() {
	const root = document.documentElement;
	root.setAttribute('data-theme', resolveTheme(pref, systemDark));
	// The first-paint candidate has done its work; the store owns the root from here.
	root.removeAttribute('data-theme-light');
}

export const themeStore = {
	get pref() {
		return pref;
	},
	get active() {
		return resolveTheme(pref, systemDark);
	},
	/** Read the cookie and the system, apply, and follow the system from now on. */
	init() {
		if (!browser) return;
		pref = decodePreference(readCookie(document.cookie, THEME_COOKIE));
		if (!media) {
			media = matchMedia('(prefers-color-scheme: dark)');
			media.addEventListener('change', (e) => {
				systemDark = e.matches;
				apply();
			});
		}
		systemDark = media.matches;
		apply();
	},
	choose(next: ThemePreference) {
		pref = next;
		if (!browser) return;
		// biome-ignore lint/suspicious/noDocumentCookie: the Cookie Store API is not in every browser the web serves
		document.cookie = `${THEME_COOKIE}=${encodeURIComponent(encodePreference(next))}; Path=/; Max-Age=${THEME_COOKIE_MAX_AGE}; SameSite=Lax`;
		apply();
	},
};
