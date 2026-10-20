/**
 * Form actions on a page whose URL carries state of its own.
 *
 * SvelteKit names a form action by a search key that starts with `/` — it takes the first such
 * key in the POST's URL (`@sveltejs/kit` `runtime/server/page/actions.js`, `call_action`) — and
 * does not take it back off: the page rendered after a plain, unenhanced POST is loaded with that
 * URL, and the browser shows it. Two consequences for a page that keeps a position in its query:
 *
 * - a bare `action="?/name"` REPLACES the query, so the POST drops the page's own state;
 * - a link the load builds from `url` would carry the `/name` key into every href it makes.
 *
 * `formAction` keeps the page's query and appends the action; `withoutActionKeys` is the other
 * half, for anything that builds a URL from the page's own.
 */

/** `url`'s search params with every action key (`/name`) removed. */
export function withoutActionKeys(params: URLSearchParams): URLSearchParams {
	const kept = new URLSearchParams();
	for (const [key, value] of params) {
		if (!key.startsWith('/')) kept.append(key, value);
	}
	return kept;
}

/**
 * The `action` attribute for the form action `name` on the page at `pageUrl`: the page's own
 * query, minus any action key an earlier POST left in it, plus `/name`. With no query of its own
 * this is exactly `?/name`.
 */
export function formAction(pageUrl: URL, name: string): string {
	const kept = withoutActionKeys(pageUrl.searchParams).toString();
	return kept ? `?${kept}&/${name}` : `?/${name}`;
}
