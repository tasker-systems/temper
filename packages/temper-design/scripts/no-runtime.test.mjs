// Harness for no-runtime.mjs. Each case writes a scratch source tree beside a clean module and
// plants one import; the check must refuse it and name it. The clean tree must pass, or a check
// that refused everything would satisfy every refusal below without ever being right. The
// package's own source must pass too: it is what the check exists to hold.
//
//   node --test scripts/no-runtime.test.mjs
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { afterEach, test } from 'node:test';
import { fileURLToPath } from 'node:url';

const CHECK = join(dirname(fileURLToPath(import.meta.url)), 'no-runtime.mjs');
const CLEAN = "import { Marked } from 'marked';\nimport { highlightCode } from './highlight.js';\n";

let root;
afterEach(() => root && rmSync(root, { recursive: true, force: true }));

function tree(planted) {
	root = mkdtempSync(join(tmpdir(), 'no-runtime-'));
	writeFileSync(join(root, 'clean.ts'), CLEAN);
	if (planted) writeFileSync(join(root, 'planted.ts'), planted);
	return root;
}

function check(...args) {
	return spawnSync(process.execPath, [CHECK, ...args], { encoding: 'utf8' });
}

function assertRefused(run, specifier) {
	assert.equal(run.status, 1, `expected a refusal, got ${run.status}:\n${run.stdout}${run.stderr}`);
	assert.ok(
		run.stderr.includes(`  ${specifier} — `),
		`refusal does not name ${specifier}:\n${run.stderr}`,
	);
}

test('a tree importing only shared dependencies passes', () => {
	const run = check('--src', tree());
	assert.equal(run.status, 0, run.stderr);
});

test("the package's own source binds no runtime", () => {
	const run = check();
	assert.equal(run.status, 0, run.stderr);
});

const PLANTED = [
	['@tauri-apps/api/core', "import { invoke } from '@tauri-apps/api/core';"],
	['@sveltejs/kit', "import { error } from '@sveltejs/kit';"],
	['$app/environment', "import { browser } from '$app/environment';"],
	['$env/dynamic/private', "import { env } from '$env/dynamic/private';"],
	['$lib/server/bounded', "export { GaveUp } from '$lib/server/bounded';"],
	['$lib', "import { thing } from '$lib';"],
	['node:path', 'const path = await import(`node:path`);'],
	['node', '/// <reference types="node" />'],
	['node:fs', "const fs = await import('node:fs');"],
	['fs', "const fs = require('fs');"],
	['svelte', "import { mount } from 'svelte';"],
	['svelte/store', "import 'svelte/store';"],
];

for (const [specifier, line] of PLANTED) {
	test(`an import of ${specifier} is refused`, () => {
		assertRefused(check('--src', tree(`${line}\n`)), specifier);
	});
}

test('a module of every script extension is read', () => {
	for (const ext of ['mts', 'cts', 'tsx', 'cjs', 'jsx']) {
		const src = tree();
		writeFileSync(join(src, `planted.${ext}`), "const fs = require('fs');\n");
		assertRefused(check('--src', src), 'fs');
		rmSync(root, { recursive: true, force: true });
	}
	root = undefined;
});

test('a tree with no modules is refused, not passed', () => {
	root = mkdtempSync(join(tmpdir(), 'no-runtime-'));
	assert.equal(check('--src', root).status, 1);
});

test('a test file is not a published module, and is not read', () => {
	const src = tree();
	writeFileSync(
		join(src, 'clean.test.ts'),
		"import { describe } from 'vitest';\nimport 'node:fs';\n",
	);
	assert.equal(check('--src', src).status, 0);
});

test('a malformed invocation is a usage error, not a pass', () => {
	assert.equal(check('--src').status, 2);
	assert.equal(check('--bogus', 'x').status, 2);
});
