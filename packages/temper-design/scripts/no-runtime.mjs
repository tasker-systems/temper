// `shared-never-binds-a-runtime`: nothing in the shared source requires either surface's runtime —
// the desktop's native core, or the web's server. Every module the package publishes is read for
// what it imports, and an import of either runtime (or of anything only one surface's build can
// resolve) is refused. The DOM is common ground: both surfaces render in a browser.
//
//   node scripts/no-runtime.mjs [--src <dir>]
//
// Exit 0 when every module is clear, 1 with one line per refused import, 2 on a usage error.
// Its harness is no-runtime.test.mjs; a forbidden prefix that the harness does not plant is a
// prefix nothing proves is refused.
import { readdirSync, readFileSync } from 'node:fs';
import { builtinModules } from 'node:module';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Refused by prefix: what each binds, so a refusal says why. */
export const FORBIDDEN = [
	['@tauri-apps/', "the desktop's native core"],
	['@sveltejs/kit', "the web's server framework"],
	['$app/', "SvelteKit's app modules"],
	['$env/', "SvelteKit's environment"],
	['$lib/', "one surface's own source"],
	['node:', 'Node, which only the server has'],
];

/** Refused exactly or as a subpath: the UI framework, which a pure module never needs. */
const FRAMEWORK = 'svelte';

const pkg = join(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const at = args.indexOf('--src');
if (args.length && (at !== 0 || args.length !== 2)) {
	console.error('usage: no-runtime.mjs [--src <dir>]');
	process.exit(2);
}
const src = at === 0 ? args[1] : join(pkg, 'src');

// Static imports, re-exports, dynamic imports and requires: every way a module names another.
const SPECIFIER = /(?:\bfrom\s*|\bimport\s*\(\s*|\bimport\s+|\brequire\s*\(\s*)(['"])([^'"]+)\1/g;

function why(specifier) {
	for (const [prefix, binds] of FORBIDDEN) if (specifier.startsWith(prefix)) return binds;
	if (specifier === FRAMEWORK || specifier.startsWith(`${FRAMEWORK}/`)) return 'the UI framework';
	if (builtinModules.includes(specifier.split('/')[0])) return 'Node, which only the server has';
	return null;
}

const modules = readdirSync(src, { recursive: true })
	.map(String)
	.filter((file) => /\.(ts|js|mjs)$/.test(file) && !/\.test\.(ts|js|mjs)$/.test(file))
	.sort();

const refusals = [];
for (const file of modules) {
	const text = readFileSync(join(src, file), 'utf8');
	for (const [, , specifier] of text.matchAll(SPECIFIER)) {
		const binds = why(specifier);
		if (binds) refusals.push(`  ${specifier} — in ${relative(src, join(src, file))}: binds ${binds}`);
	}
}

if (refusals.length) {
	console.error('The shared source imports a runtime one surface lacks:');
	for (const line of refusals) console.error(line);
	process.exit(1);
}
console.log(`ok   ${modules.length} shared modules bind no surface's runtime`);
