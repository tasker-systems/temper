#!/usr/bin/env node
// `web-paints-only-roles` / `no-private-colour-names`: a component paints colour from the
// contract's roles (`var(--tp-*)`, `tp-` utilities) and nothing else, so every theme restyles
// everything. Within the paths a surface lists, this refuses:
//
//   (a) a literal colour: hex, a colour function, or a named colour;
//   (b) a Tailwind colour utility naming the stock palette, white or black, or a colour name the
//       surface declares for itself (`--color-<name>` in the theme files it lists);
//   (c) an opacity modifier on a role utility: a role at 15% is a value no theme chose;
//   (d) a colour declaration, SVG attribute, style directive or arbitrary colour utility reading
//       a variable that is not a role;
//   (e) a role mixed down: a `color-mix()` reading a role is (c) spelled in CSS.
//
// A surface commits a config naming what it guards, relative to the config's own directory:
//
//   { "paths": ["src/lib/components/Sidebar.svelte", "src/routes/(app)"],
//     "themes": ["src/app.css"],
//     "pending": [{ "var": "--hue", "until": "T10", "why": "doc-type tint, not yet bound" }] }
//
// A `pending` entry is a declared hole: a variable that is not a role yet, the task that makes it
// one, and why it waits. Its reads — and any `color-mix()` reading it — are counted and reported
// on every run rather than refused, so the hole stays stated. A literal beside it is still (a).
//
// A listed directory is walked for .svelte and .css files; a listed file is read whatever its
// extension. A surface still moving off the stock palette lists the areas it has moved, and
// each area that moves adds its paths. Comments are ignored.
//
//   node guard/colours.mjs <colour-guard.json>
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';

const USAGE = 'usage: colours.mjs <colour-guard.json>';

function fail(message) {
	console.error(`colours: ${message}`);
	process.exit(2);
}

const args = process.argv.slice(2);
if (args.length !== 1) fail(USAGE);
const configPath = resolve(args[0]);
if (!existsSync(configPath)) fail(`no config at ${configPath}`);
let config;
try {
	config = JSON.parse(readFileSync(configPath, 'utf8'));
} catch (e) {
	fail(`${configPath} is not JSON: ${e.message}`);
}
const base = dirname(configPath);
const at = (p) => (isAbsolute(p) ? p : join(base, p));
const paths = config.paths ?? [];
const themes = config.themes ?? [];
if (!Array.isArray(paths) || paths.length === 0)
	fail('"paths" is empty: an empty scope guards nothing');
if (!Array.isArray(themes)) fail('"themes" must be a list');
const pendingEntries = config.pending ?? [];
if (!Array.isArray(pendingEntries)) fail('"pending" must be a list');
const pending = new Map();
for (const entry of pendingEntries) {
	const { var: name, until, why } = entry ?? {};
	if (![name, until, why].every((v) => typeof v === 'string' && v.trim()))
		fail(`pending entry ${JSON.stringify(entry)} needs "var", "until" and "why": a hole is named`);
	if (!/^--[a-zA-Z0-9-]+$/.test(name)) fail(`pending "var" ${name} is not a custom property`);
	if (name.startsWith('--tp-')) fail(`pending "var" ${name} is a role: a role needs no excuse`);
	pending.set(name, { until, reads: 0 });
}

// The desktop's literal rule: hex, the colour functions, and named colours where a value ends.
// An id or anchor like `#r0` is not a colour; only hex-shaped runs of 3/4/6/8 count. Nor is an
// HTML numeric entity (`&#8984;`, the ⌘ glyph), which the desktop's rule would misread.
const LITERAL =
	/(?<!&)#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|oklch|oklab|lab|lch|hwb)\(|\b(?:white|black|red|green|blue|gray|grey|orange|yellow|purple|pink)\b(?=\s*[;})'"])/g;

const PREFIX =
	'(?:bg|text-shadow|text|border-[xytrbl]|border-[se]|border|ring-offset|ring|inset-ring|inset-shadow|divide|outline|placeholder|from|via|to|fill|stroke|drop-shadow|shadow|decoration|accent|caret)';
const VARIANTS = '(?:[a-z0-9-]+:)*';
// A colour utility with a plain value: `hover:bg-zinc-800/50`, `class:border-zinc-700`.
const UTILITY = new RegExp(
	`(?<![\\w-])${VARIANTS}${PREFIX}-([a-z0-9-]+)(/(?:\\d+|\\[[^\\]\\s]+\\]))?(?![\\w-])`,
	'g',
);
// A colour utility with an arbitrary value reading a variable: `bg-[var(--obsidian)]`,
// `text-(--chalk)`. Only a variable outside the contract is refused.
const ARBITRARY_VAR = new RegExp(
	`(?<![\\w-])${VARIANTS}${PREFIX}-(?:\\[|\\()[^\\]\\)\\s]*?(--(?!tp-)[a-zA-Z0-9-]+)`,
	'g',
);
const STOCK =
	/^(?:(?:slate|gray|zinc|neutral|stone|mauve|olive|mist|taupe|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-\d{2,3}|white|black)$/;
// A declaration that paints a colour, and the value it paints.
const DECLARATION =
	/(?<![\w-])(color|background(?:-color)?|border(?:-(?:top|right|bottom|left|inline|block)(?:-(?:start|end))?)?(?:-color)?|outline(?:-color)?|fill|stroke|box-shadow|text-decoration(?:-color)?|caret-color|accent-color|stop-color|column-rule(?:-color)?)\s*:\s*([^;}"'\n]*)/g;
// The same, painted through markup: an SVG presentation attribute (`fill="var(--x)"`) or a
// Svelte style directive (`style:color="var(--x)"`).
const ATTRIBUTE =
	/(?<![\w-])(?:style:)?(fill|stroke|stop-color|flood-color|lighting-color|color|background(?:-color)?|border(?:-[a-z]+)*-color|outline-color|caret-color|accent-color|text-decoration-color)\s*=\s*["'{]([^"'}\n]*)/g;
const PRIVATE_VAR = /var\(\s*(--(?!tp-)[a-zA-Z0-9-]+)/g;
const ROLE_VAR = /var\(\s*--tp-/;
const PENDING_VAR = /var\(\s*(--[a-zA-Z0-9-]+)/g;

// Each `color-mix(…)` on a line, with its balanced argument list. Tailwind's arbitrary values
// spell spaces as `_`, which changes nothing here.
function* mixes(line) {
	for (let at = line.indexOf('color-mix('); at >= 0; at = line.indexOf('color-mix(', at + 1)) {
		let depth = 0;
		let end = at + 'color-mix'.length;
		for (; end < line.length; end++) {
			if (line[end] === '(') depth++;
			else if (line[end] === ')' && --depth === 0) break;
		}
		yield line.slice(at, end + 1);
	}
}

// Blank out every `open … close` span, keeping its newlines so reported line numbers stay true.
// A scan rather than a regex replace: an unterminated comment blanks the rest of the file instead
// of leaving a fragment behind.
function blank(text, open, close) {
	let out = '';
	let i = 0;
	for (;;) {
		const start = text.indexOf(open, i);
		if (start < 0) return out + text.slice(i);
		out += text.slice(i, start);
		const end = text.indexOf(close, start + open.length);
		const stop = end < 0 ? text.length : end + close.length;
		out += text.slice(start, stop).replace(/[^\n]/g, '');
		if (end < 0) return out;
		i = stop;
	}
}

const stripComments = (text) =>
	blank(blank(text, '/*', '*/'), '<!--', '-->').replace(/^[ \t]*\/\/.*$/gm, '');

// The colour names a surface declares for itself: every `--color-<name>` in its theme files that
// is not a role. These are what `no-private-colour-names` refuses.
const privateNames = new Set();
for (const theme of themes) {
	const path = at(theme);
	if (!existsSync(path)) fail(`listed theme ${theme} does not exist`);
	const text = stripComments(readFileSync(path, 'utf8'));
	for (const m of text.matchAll(/--color-([a-z0-9-]+)\s*:/g)) {
		if (!m[1].startsWith('tp-')) privateNames.add(m[1]);
	}
}

function* walk(dir) {
	for (const name of readdirSync(dir)) {
		const path = join(dir, name);
		if (statSync(path).isDirectory()) yield* walk(path);
		else if (/\.(svelte|css)$/.test(name)) yield path;
	}
}

const files = [];
for (const listed of paths) {
	const path = at(listed);
	if (!existsSync(path))
		fail(`listed path ${listed} does not exist: a scope that names nothing guards nothing`);
	if (statSync(path).isDirectory()) {
		const found = [...walk(path)];
		if (found.length === 0) fail(`listed directory ${listed} holds no .svelte or .css file`);
		files.push(...found);
	} else files.push(path);
}

const readsPending = (text) => [...text.matchAll(PENDING_VAR)].some((m) => pending.has(m[1]));

function refusals(line) {
	const out = [];
	for (const mix of mixes(line)) {
		if (ROLE_VAR.test(mix) && !readsPending(mix))
			out.push(
				`${mix} mixes a role to a strength no theme chose — use a role (*-wash, accent-hover, …)`,
			);
	}
	for (const m of line.matchAll(PENDING_VAR)) {
		if (pending.has(m[1])) pending.get(m[1]).reads++;
	}
	for (const m of line.matchAll(LITERAL)) {
		if (m[0].startsWith('#') && ![4, 5, 7, 9].includes(m[0].length)) continue;
		out.push(`literal colour ${m[0]} — use a theme role`);
	}
	for (const m of line.matchAll(UTILITY)) {
		const [utility, value, modifier] = m;
		if (value.startsWith('tp-')) {
			if (modifier) out.push(`${utility} sets a role's strength — use its -wash or -hover role`);
		} else if (STOCK.test(value)) {
			out.push(`${utility} paints from Tailwind's palette — use a tp- role`);
		} else if (privateNames.has(value)) {
			out.push(`${utility} paints a colour name the contract lacks — use a tp- role`);
		}
	}
	for (const m of line.matchAll(ARBITRARY_VAR)) {
		if (pending.has(m[1])) continue;
		out.push(`${m[0]} reads ${m[1]}, which is not a role`);
	}
	for (const m of [...line.matchAll(DECLARATION), ...line.matchAll(ATTRIBUTE)]) {
		for (const v of m[2].matchAll(PRIVATE_VAR)) {
			if (pending.has(v[1])) continue;
			out.push(`${m[1]} reads var(${v[1]}), which is not a role`);
		}
	}
	return out;
}

let failures = 0;
for (const path of files) {
	const lines = stripComments(readFileSync(path, 'utf8')).split('\n');
	lines.forEach((line, i) => {
		for (const why of refusals(line)) {
			console.error(`${relative(base, path)}:${i + 1}: ${why}`);
			failures++;
		}
	});
}
for (const [name, { until, reads }] of pending) {
	console.log(`pending: ${name} read ${reads} time(s) until ${until} — not a role yet`);
}
if (failures) {
	console.error(
		`\n${failures} colour(s) outside the contract. Components paint --tp-* roles only (@tasker-systems/temper-design).`,
	);
	process.exit(1);
}
console.log(`ok: ${files.length} file(s) paint only contract roles`);
