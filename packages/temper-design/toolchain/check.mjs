#!/usr/bin/env node
// Every temper surface builds on the same majors of one named toolchain: the language, the
// framework and its Vite plugin, the type-checker, the bundler, the test runner, the styling
// engine and the linter. A major upgrade of any of them reaches every surface together.
//
// toolchain/majors.json, beside this file, declares those majors. Each surface runs this check
// from its own directory and it fails when the surface resolves a different major of a named
// tool, or does not resolve one at all. Inside the temper repository the web reads the
// declaration from its working tree, so a web major cannot move unless the declaration moves in
// the same change. Outside it, a surface reads the declaration at the package version it pins,
// so the pin bump that carries a new major fails until that surface has taken it too.
//
// Adapters differ by surface; each surface names its own with --adapter, and that adapter must
// be declared.
//
// A tool is resolved the way Node resolves a bare import, by looking for
// node_modules/<tool>/package.json from the surface's directory upward. It does not read a
// lockfile, and it does not go through a package's `exports`, which may not expose
// package.json.
//
//   node toolchain/check.mjs --adapter @sveltejs/adapter-vercel [--surface <dir>]
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const USAGE = 'usage: check.mjs --adapter <adapter package> [--surface <dir>]';

function fail(message) {
	console.error(`toolchain: ${message}`);
	process.exit(2);
}

let adapter = null;
let surface = process.cwd();
const args = process.argv.slice(2);
for (let i = 0; i < args.length; i++) {
	if (args[i] === '--adapter' && args[i + 1]) adapter = args[++i];
	else if (args[i] === '--surface' && args[i + 1]) surface = resolve(args[++i]);
	else fail(USAGE);
}
if (!adapter) fail(USAGE);

const declared = JSON.parse(
	readFileSync(join(dirname(fileURLToPath(import.meta.url)), 'majors.json'), 'utf8')
);

function resolvedVersion(name) {
	for (let dir = surface; ; dir = dirname(dir)) {
		const manifest = join(dir, 'node_modules', name, 'package.json');
		if (existsSync(manifest)) return JSON.parse(readFileSync(manifest, 'utf8')).version;
		if (dirname(dir) === dir) return null;
	}
}

const wanted = Object.entries(declared.majors);
const refusals = [];
if (Object.hasOwn(declared.adapters, adapter)) wanted.push([adapter, declared.adapters[adapter]]);
else refusals.push(`${adapter} — not a declared adapter (${Object.keys(declared.adapters).join(', ')})`);

for (const [name, major] of wanted) {
	const version = resolvedVersion(name);
	if (version === null) refusals.push(`${name} — not installed; declared major ${major}`);
	else if (Number(version.split('.')[0]) !== major)
		refusals.push(`${name} — resolves ${version}; declared major ${major}`);
	else console.log(`  ${name} ${version}`);
}

if (refusals.length > 0) {
	for (const refusal of refusals) console.error(`  ${refusal}`);
	console.error(
		'toolchain: this surface is off the declared majors (above). A major moves on every surface\n' +
			'together: change toolchain/majors.json in @tasker-systems/temper-design with the upgrade.'
	);
	process.exit(1);
}
console.log(`toolchain: ${wanted.length} tools on the declared majors`);
