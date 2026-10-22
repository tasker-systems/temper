// Harness for colours.mjs. Each case writes a scratch surface: a clean component, one planted
// component, and the guard's config naming them. The guard must refuse the planted colour and
// name its file and line. The clean surface must pass, or a guard that refused everything would
// satisfy every refusal below without ever being right.
//
//   node --test guard/colours.test.mjs
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { afterEach, test } from 'node:test';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const GUARD = join(HERE, 'colours.mjs');

// Every way a component legitimately paints: role utilities with variants and directives,
// role vars in scoped styles and declarations, and colour words that are not colours.
const CLEAN = `<script lang="ts">
	// bg-zinc-800 in a line comment is not painted
	let open = $state(false);
</script>

<!-- text-zinc-500 and #ffffff in markup comments are not painted -->
<div class="flex bg-tp-ground text-tp-text hover:bg-tp-accent-hover border-t border-tp-rule text-sm" class:bg-tp-accent-wash={open}>
	<a href="#ab12c" class="ring-2 ring-tp-focus shadow-lg outline-none border-b-2 text-[10px]">x</a>
	<kbd>&#8984;K</kbd>
	<svg><rect fill="var(--tp-surface)" stroke="currentColor" /><stop stop-color="var(--tp-accent)" /></svg>
	<span style:color="var(--tp-text)">y</span>
</div>

<style>
	/* color: #fff; background: var(--obsidian); */
	.x {
		color: var(--tp-text-muted);
		border: 1px solid var(--tp-rule);
		background: var(--tp-surface);
		font-family: var(--font-mono);
		padding: var(--space-column);
	}
</style>
`;

// A theme file declaring the surface's own colour names, as the web's app.css does.
const THEME = `@theme {
	--color-quiet-accent: #7eb8da;
	--color-temper-500: #0c99e9;
	--font-sans: "Inter";
}
`;

let root;
afterEach(() => root && rmSync(root, { recursive: true, force: true }));

function surface(planted, config = {}) {
	root = mkdtempSync(join(tmpdir(), 'colour-guard-'));
	mkdirSync(join(root, 'src'));
	writeFileSync(join(root, 'src', 'Clean.svelte'), CLEAN);
	writeFileSync(join(root, 'src', 'theme.css'), THEME);
	if (planted !== undefined) writeFileSync(join(root, 'src', 'Planted.svelte'), planted);
	const paths = ['src/Clean.svelte', ...(planted === undefined ? [] : ['src/Planted.svelte'])];
	writeFileSync(
		join(root, 'colour-guard.json'),
		JSON.stringify({ paths, themes: ['src/theme.css'], ...config }),
	);
	return join(root, 'colour-guard.json');
}

function guard(...args) {
	return spawnSync(process.execPath, [GUARD, ...args], { encoding: 'utf8' });
}

function assertRefused(run, line, found) {
	assert.equal(run.status, 1, `expected a refusal, got ${run.status}:\n${run.stdout}${run.stderr}`);
	assert.ok(
		run.stderr.includes(`src/Planted.svelte:${line}: `) && run.stderr.includes(found),
		`refusal does not name Planted.svelte:${line} and ${found}:\n${run.stderr}`,
	);
}

test('a surface painting only roles passes', () => {
	const run = guard(surface());
	assert.equal(run.status, 0, run.stderr);
});

test("the contract's own recipes pass", () => {
	const dir = mkdtempSync(join(tmpdir(), 'colour-guard-'));
	root = dir;
	const contract = join(HERE, '..', 'contract');
	writeFileSync(
		join(dir, 'colour-guard.json'),
		JSON.stringify({
			paths: ['base.css', 'region.css', 'shadcn-bridge.css'].map((f) => join(contract, f)),
		}),
	);
	const run = guard(join(dir, 'colour-guard.json'));
	assert.equal(run.status, 0, run.stderr);
});

const PLANTED = [
	// (a) literal colours, as the desktop's check finds them
	['a hex literal', '<style>\n.x { color: #e8e4df; }\n</style>\n', 2, '#e8e4df'],
	[
		'a colour function',
		'<style>\n.x { background: rgba(255, 255, 255, 0.06); }\n</style>\n',
		2,
		'rgba(',
	],
	['a named colour', '<style>\n.x { color: white; }\n</style>\n', 2, 'white'],
	['a hex literal in an arbitrary utility', '<div class="bg-[#09090b]"></div>\n', 1, '#09090b'],
	// (b) the stock palette
	['a stock palette utility', '<p class="text-zinc-500">x</p>\n', 1, 'text-zinc-500'],
	[
		'a stock utility behind a variant, with opacity',
		'<p class="flex hover:bg-zinc-800/50">x</p>\n',
		1,
		'hover:bg-zinc-800/50',
	],
	[
		'a stock utility in a class directive',
		'<p class:border-zinc-700={on}>x</p>\n',
		1,
		'border-zinc-700',
	],
	[
		'a one-side border in the stock palette',
		'<p class="border-t border-t-zinc-800">x</p>\n',
		1,
		'border-t-zinc-800',
	],
	['white and black utilities', '<p class="bg-black/60">x</p>\n', 1, 'bg-black/60'],
	[
		'a stock utility built in script',
		"<script>\nconst c = on ? 'text-amber-300' : '';\n</script>\n",
		2,
		'text-amber-300',
	],
	// (b) the surface's own colour names, read from its theme files
	['a private colour name', '<p class="text-quiet-accent">x</p>\n', 1, 'text-quiet-accent'],
	['a private scale colour', '<p class="\n  bg-temper-500">x</p>\n', 2, 'bg-temper-500'],
	// (c) a role at a strength no theme chose
	['an opacity modifier on a role', '<p class="bg-tp-accent/15">x</p>\n', 1, 'bg-tp-accent/15'],
	// (d) a colour painted from a variable the contract lacks
	[
		'a private var in a colour declaration',
		'<style>\n.x { color: var(--parchment); }\n</style>\n',
		2,
		'var(--parchment)',
	],
	[
		'a private var in a border shorthand',
		'<style>\n.x { border: 1px solid var(--rule); }\n</style>\n',
		2,
		'var(--rule)',
	],
	[
		'a private var in an arbitrary utility',
		'<p class="bg-[var(--obsidian)]">x</p>\n',
		1,
		'--obsidian',
	],
	['a private var in a shorthand utility', '<p class="text-(--chalk)">x</p>\n', 1, '--chalk'],
	[
		'a private var in an SVG fill',
		'<svg>\n<rect fill="var(--obsidian)" />\n</svg>\n',
		2,
		'--obsidian',
	],
	[
		'a private var in an SVG stop colour',
		'<svg><stop stop-color="var(--obsidian)" /></svg>\n',
		1,
		'--obsidian',
	],
	[
		'a private var in a style directive',
		'<div style:color="var(--obsidian)">x</div>\n',
		1,
		'--obsidian',
	],
	// (b) the colour utilities Tailwind v4 added beside the classic ones
	['a stock text shadow', '<p class="text-shadow-zinc-500">x</p>\n', 1, 'text-shadow-zinc-500'],
	['a stock inset ring', '<p class="inset-ring-zinc-700">x</p>\n', 1, 'inset-ring-zinc-700'],
	['a stock inset shadow', '<p class="inset-shadow-zinc-800">x</p>\n', 1, 'inset-shadow-zinc-800'],
	['a stock drop shadow', '<p class="drop-shadow-black/50">x</p>\n', 1, 'drop-shadow-black/50'],
	// a refusal is reported on its own line, whatever comments and blank lines precede it
	[
		'a refusal after blank lines and a line comment',
		"<script>\n\n\n// a comment\nconst c = 'text-zinc-500';\n</script>\n",
		5,
		'text-zinc-500',
	],
];

for (const [name, planted, line, found] of PLANTED) {
	test(`${name} is refused`, () => {
		assertRefused(guard(surface(planted)), line, found);
	});
}

test('a component of any listed directory is read', () => {
	const config = surface('<p class="text-zinc-500">x</p>\n', { paths: ['src'] });
	assertRefused(guard(config), 1, 'text-zinc-500');
});

test('an unterminated comment blanks the rest of the file, not the next line only', () => {
	const run = guard(surface('<style>\n/* open\n.x { color: #fff; }\n</style>\n'));
	assert.equal(run.status, 0, run.stderr);
});

test('a config whose scope is empty is refused, not passed', () => {
	assert.equal(guard(surface(undefined, { paths: [] })).status, 2);
});

test('a listed path that does not exist is refused, not skipped', () => {
	assert.equal(
		guard(surface(undefined, { paths: ['src/Clean.svelte', 'src/Gone.svelte'] })).status,
		2,
	);
});

test('a listed directory with no components is refused, not passed', () => {
	const config = surface(undefined);
	mkdirSync(join(root, 'empty'));
	writeFileSync(config, JSON.stringify({ paths: ['empty'] }));
	assert.equal(guard(config).status, 2);
});

test('a listed theme file that does not exist is refused, not skipped', () => {
	assert.equal(guard(surface(undefined, { themes: ['src/gone.css'] })).status, 2);
});

test('a themes value that is not a list is refused', () => {
	assert.equal(guard(surface(undefined, { themes: { path: 'src/theme.css' } })).status, 2);
});

test('a malformed invocation is a usage error, not a pass', () => {
	assert.equal(guard().status, 2);
	assert.equal(guard('a.json', 'b.json').status, 2);
	assert.equal(guard('/nonexistent/colour-guard.json').status, 2);
});
