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

/** Refused exactly or as a subpath, what each binds. */
const EXACT = [
	['svelte', 'the UI framework'],
	['$lib', "one surface's own source"],
	// `/// <reference types="node" />` names Node's types by the bare word.
	['node', 'Node, which only the server has'],
];

const pkg = join(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const at = args.indexOf('--src');
if (args.length && (at !== 0 || args.length !== 2)) {
	console.error('usage: no-runtime.mjs [--src <dir>]');
	process.exit(2);
}
const src = at === 0 ? args[1] : join(pkg, 'src');

// Static imports, re-exports, dynamic imports (template literals too), requires, and triple-slash
// references: every way a module names another.
const SPECIFIER =
	/(?:\bfrom\s*|\bimport\s*\(\s*|\bimport\s+|\brequire\s*\(\s*|<reference\s+(?:types|path)\s*=\s*)(['"`])([^'"`]+)\1/g;

function why(specifier) {
	for (const [prefix, binds] of FORBIDDEN) if (specifier.startsWith(prefix)) return binds;
	for (const [name, binds] of EXACT) {
		if (specifier === name || specifier.startsWith(`${name}/`)) return binds;
	}
	if (builtinModules.includes(specifier.split('/')[0])) return 'Node, which only the server has';
	return null;
}

const modules = readdirSync(src, { recursive: true })
	.map(String)
	.filter((file) => /\.[cm]?[jt]sx?$/.test(file) && !/\.test\.[cm]?[jt]sx?$/.test(file))
	.sort();

// A scan of nothing is not a pass: a wrong path, or an emptied tree, would otherwise read clean.
if (modules.length === 0) {
	console.error(`no modules to check under ${src}`);
	process.exit(1);
}

const refusals = [];
for (const file of modules) {
	const text = readFileSync(join(src, file), 'utf8');
	for (const [, , specifier] of text.matchAll(SPECIFIER)) {
		const binds = why(specifier);
		if (binds)
			refusals.push(`  ${specifier} — in ${relative(src, join(src, file))}: binds ${binds}`);
	}
}

if (refusals.length) {
	console.error('The shared source imports a runtime one surface lacks:');
	for (const line of refusals) console.error(line);
	process.exit(1);
}
console.log(`ok   ${modules.length} shared modules bind no surface's runtime`);
