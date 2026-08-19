import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';
import { configDefaults } from 'vitest/config';

export default defineConfig({
	plugins: [tailwindcss(), sveltekit()],
	test: {
		// Two projects because the two kinds of test want different environments, and the
		// pure-module suite must not pay for the DOM one. What belongs in each — and what a
		// component test is NOT allowed to do — is `src/test/README.md`.
		projects: [
			{
				extends: true,
				test: {
					name: 'unit',
					include: ['src/**/*.test.ts'],
					// Spread, don't replace: `exclude` is a plain default, so a bare list drops
					// vitest's own — `**/node_modules/**` included.
					exclude: [...configDefaults.exclude, 'src/**/*.component.test.ts'],
					environment: 'node',
					globals: false
				}
			},
			{
				extends: true,
				// Without `browser`, Svelte resolves to `index-server.js` and every `render()`
				// throws `lifecycle_function_unavailable`.
				resolve: { conditions: ['browser'] },
				test: {
					name: 'component',
					include: ['src/**/*.component.test.ts'],
					environment: 'jsdom',
					globals: false,
					setupFiles: ['src/test/component-setup.ts']
				}
			}
		]
	}
});
