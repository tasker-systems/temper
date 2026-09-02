import adapter from '@sveltejs/adapter-vercel';
import { relative, sep } from 'node:path';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	compilerOptions: {
		runes: ({ filename }) => {
			const relativePath = relative(import.meta.dirname, filename);
			const pathSegments = relativePath.toLowerCase().split(sep);
			const isExternalLibrary = pathSegments.includes('node_modules');
			return isExternalLibrary ? undefined : true;
		}
	},
	kit: {
		adapter: adapter({
			runtime: 'nodejs22.x'
		}),
		// Disable the built-in origin CSRF check. It runs *ahead* of the handle
		// hook, so it would block the SAML ACS proxy POST — a legitimately
		// cross-origin form POST from the IdP — before the proxy short-circuit can
		// forward it upstream. The equivalent guard is re-implemented in
		// hooks.server.ts (see $lib/server/csrf), scoped to the UI's own routes.
		// (`trustedOrigins: ['*']` is the non-deprecated way to disable it.)
		csrf: { trustedOrigins: ['*'] },
		csp: {
			// `auto` = nonce on dynamically rendered pages, hashes only where
			// prerendering would force them (there are no prerendered routes here).
			// Hash mode alone is not an option: it is incompatible with response
			// streaming, and the graph loads stream eleven fields
			// ($lib/server/bounded.ts).
			//
			// THE NONCE-VS-'unsafe-inline' RESOLUTION, recorded so the next reader
			// does not rediscover it from a browser console:
			//
			// - A CSP directive that carries a nonce (or hash) makes browsers IGNORE
			//   'unsafe-inline' in that same directive. SvelteKit appends a per-request
			//   nonce to `script-src` (its inline hydration script) and to `style-src`
			//   *if the build emits inline `<style>` elements*. That is why
			//   'unsafe-inline' must never be placed in `style-src`: on a build that
			//   inlines styles it would be silently voided — an unstyled page with no
			//   console error saying so.
			// - Style ATTRIBUTES cannot carry a nonce at all, and the graph views set
			//   them at runtime (d3), as does `app.html`'s `style="display: contents"`.
			//   `style-src-attr` governs that case separately, and SvelteKit leaves any
			//   style directive that already contains 'unsafe-inline' untouched (it
			//   appends nonces only to directives that lack it), so this directive
			//   ships verbatim in production.
			// - The dev server is not evidence about the shipped header: in dev, Kit
			//   *adds* 'unsafe-inline' to every style directive and strips nonce/hash
			//   values (node_modules/@sveltejs/kit/src/runtime/server/page/csp.js,
			//   the DEV block). Verify against a production build, never `vite dev`.
			// - `connect-src 'self'` holds for every deployment shape because
			//   hooks.server.ts reverse-proxies all browser API/MCP/OAuth traffic
			//   same-origin ($lib/server/proxy). Anything that makes the browser talk
			//   to another origin directly must widen this — see
			//   docs/playbooks/deploy-the-web-ui.md.
			//
			// Kit emits this header inside `resolve`, so it lands on exactly the
			// responses this app renders — never on proxied ones, which carry the
			// upstream's own policy. That is why CSP is not in
			// $lib/server/security-headers: one owner per response.
			//
			// Verified against a production build on 2026-08-29: the emitted header
			// carried a per-request nonce in `script-src` only; this build emits no
			// inline `<style>`, so `style-src` shipped as written; `style-src-attr`
			// verbatim; the graph harness rendered 1697 SVG nodes with 131
			// runtime-written style attributes and zero console violations.
			mode: 'auto',
			directives: {
				'default-src': ['none'],
				// Hydration inline script: admitted by the appended nonce.
				'script-src': ['self'],
				// Stylesheets are same-origin only — fonts are self-hosted
				// (@fontsource-variable, +layout.svelte), so no Google Fonts origin.
				'style-src': ['self'],
				// d3 + app.html inline style attributes — see the note above.
				'style-src-attr': ['unsafe-inline'],
				// Self-hosted woff2 files, bundled into _app/immutable by Vite.
				'font-src': ['self'],
				'img-src': ['self', 'data:'],
				'connect-src': ['self'],
				'form-action': ['self'],
				'frame-ancestors': ['none'],
				'base-uri': ['self'],
				'object-src': ['none']
			}
		},
		alias: {
			'$components': 'src/lib/components'
		}
	}
};

export default config;
