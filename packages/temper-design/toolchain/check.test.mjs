// Harness for check.mjs. Each case builds a scratch surface whose node_modules holds the named
// tools at chosen versions, beside a copy of the checker and a declaration of its own. The
// clean surface must pass, or a check that refused everything would satisfy every refusal
// below without ever being right.
//
//   node --test toolchain/check.test.mjs
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { afterEach, test } from 'node:test';
import { fileURLToPath } from 'node:url';

const CHECK = join(dirname(fileURLToPath(import.meta.url)), 'check.mjs');
const DECLARED = {
	majors: { typescript: 6, svelte: 5, vitest: 5 },
	adapters: { '@sveltejs/adapter-vercel': 5, '@sveltejs/adapter-static': 3 }
};
const ON_MAJORS = {
	typescript: '6.0.3',
	svelte: '5.57.2',
	vitest: '5.0.3',
	'@sveltejs/adapter-static': '3.0.10'
};

let root;
afterEach(() => rmSync(root, { recursive: true, force: true }));

// root/checker/{check.mjs,majors.json}, and root/repo/app as the surface.
function scratch() {
	root = mkdtempSync(join(tmpdir(), 'toolchain-'));
	mkdirSync(join(root, 'checker'));
	copyFileSync(CHECK, join(root, 'checker', 'check.mjs'));
	writeFileSync(join(root, 'checker', 'majors.json'), JSON.stringify(DECLARED));
	mkdirSync(join(root, 'repo', 'app'), { recursive: true });
	return join(root, 'repo', 'app');
}

function install(dir, packages) {
	for (const [name, version] of Object.entries(packages)) {
		const at = join(dir, 'node_modules', name);
		mkdirSync(at, { recursive: true });
		writeFileSync(join(at, 'package.json'), JSON.stringify({ name, version }));
	}
}

function check(surface, ...args) {
	return spawnSync(process.execPath, [join(root, 'checker', 'check.mjs'), ...args], {
		cwd: surface,
		encoding: 'utf8'
	});
}

function assertRefused(run, names) {
	assert.equal(run.status, 1, `expected a refusal, got ${run.status}:\n${run.stdout}${run.stderr}`);
	assert.match(run.stderr, new RegExp(`  ${names.replace(/[/@.]/g, '\\$&')} — `));
}

test('a surface on every declared major passes', () => {
	const app = scratch();
	install(app, ON_MAJORS);
	const run = check(app, '--adapter', '@sveltejs/adapter-static');
	assert.equal(run.status, 0, run.stderr);
	assert.match(run.stdout, /4 tools on the declared majors/);
});

test('a tool on a different major is refused', () => {
	const app = scratch();
	install(app, { ...ON_MAJORS, typescript: '7.0.0' });
	assertRefused(check(app, '--adapter', '@sveltejs/adapter-static'), 'typescript');
});

test('a named tool the surface does not resolve is refused', () => {
	const app = scratch();
	const { vitest, ...rest } = ON_MAJORS;
	install(app, rest);
	assertRefused(check(app, '--adapter', '@sveltejs/adapter-static'), 'vitest');
});

test('an adapter on a different major is refused', () => {
	const app = scratch();
	install(app, { ...ON_MAJORS, '@sveltejs/adapter-static': '4.0.0' });
	assertRefused(check(app, '--adapter', '@sveltejs/adapter-static'), '@sveltejs/adapter-static');
});

test('an adapter the declaration does not name is refused', () => {
	const app = scratch();
	install(app, { ...ON_MAJORS, '@sveltejs/adapter-node': '5.0.0' });
	assertRefused(check(app, '--adapter', '@sveltejs/adapter-node'), '@sveltejs/adapter-node');
});

test('a tool hoisted above the surface is found, as Node would find it', () => {
	const app = scratch();
	const { typescript, ...rest } = ON_MAJORS;
	install(app, rest);
	install(join(root, 'repo'), { typescript });
	assert.equal(check(app, '--adapter', '@sveltejs/adapter-static').status, 0);
});

test("the surface's own copy wins over a hoisted one, as Node would resolve it", () => {
	const app = scratch();
	install(app, { ...ON_MAJORS, typescript: '5.9.3' });
	install(join(root, 'repo'), { typescript: '6.0.3' });
	assertRefused(check(app, '--adapter', '@sveltejs/adapter-static'), 'typescript');
});

test('no adapter named is a usage error, not a pass', () => {
	const app = scratch();
	install(app, ON_MAJORS);
	assert.equal(check(app).status, 2);
});
