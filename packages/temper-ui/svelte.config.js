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
		alias: {
			'$components': 'src/lib/components'
		}
	}
};

export default config;
