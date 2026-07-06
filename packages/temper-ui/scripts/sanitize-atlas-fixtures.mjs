#!/usr/bin/env node
// sanitize-atlas-fixtures.mjs — turn a raw, real-data Atlas fixture capture into a
// committable, personal-data-free bundle WITHOUT changing its shape.
//
// The `/dev/atlas` harness renders the real `AtlasPage` against a bundle of captured
// `AtlasViewData` scenarios (see src/routes/dev/atlas/README.md). A raw capture holds
// real resource titles, team/cogmap names, owner handles and ids from a personal team,
// so it must not be committed. This script transforms a raw capture into a synthetic
// bundle that is *structurally identical* (every key, type, enum, tier, timestamp and
// edge-grammar label preserved) but carries no personal data:
//
//   • Every UUID is remapped, first-seen order, to a deterministic well-formed synthetic
//     UUIDv7 — so cross-references (a region id in `territories` and in `slice.focus`,
//     an actor id across trail events) stay linked.
//   • Sensitive free-text (titles, team/cogmap/context names, owner handles, slugs,
//     origin URIs) is replaced value-consistently: the same source string always maps
//     to the same synthetic string, so a cogmap name shared across scenarios stays one
//     name. Titles get length-varied synthetic phrases so Tier-2 label-collision cases
//     still exercise the layout.
//   • Grammar/enum fields (edge_kind, polarity, kind, doc_type, home, element_kind,
//     relationship `label`), hashes (body_hash), numbers and timestamps are kept as-is —
//     they carry no personal data and the edge-grammar legend depends on real labels.
//
// Deterministic: same input → same output, so re-running never churns the committed file.
//
// Usage:
//   node scripts/sanitize-atlas-fixtures.mjs [inPath] [outPath]
//   defaults: static/dev/atlas-fixtures.local.json → static/dev/atlas-fixtures.json

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const uiRoot = resolve(here, '..');
const inPath = resolve(uiRoot, process.argv[2] ?? 'static/dev/atlas-fixtures.local.json');
const outPath = resolve(uiRoot, process.argv[3] ?? 'static/dev/atlas-fixtures.json');

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const UUID_IN_STR = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi;

// Keys whose free-text values are personal and must be replaced. Grammar/enum/label
// keys are deliberately absent so the edge-grammar legend keeps rendering real labels.
const SENSITIVE_TEXT_KEYS = new Set([
	'title',
	'name',
	'cogmapName',
	'cogmap_name',
	'anchor_label',
	'owner_handle',
	'slug',
	'origin_uri'
]);

// Deterministic well-formed synthetic UUIDv7 from a first-seen counter. Version nibble
// 7, variant nibble 8, counter encoded in the low 12 hex so ids stay distinct + ordered.
const uuidMap = new Map();
function synthUuid(orig) {
	let u = uuidMap.get(orig);
	if (u) return u;
	const n = uuidMap.size + 1;
	const hi = n.toString(16).padStart(4, '0');
	const lo = n.toString(16).padStart(12, '0');
	u = `0191d0c0-${hi}-7000-8000-${lo}`;
	uuidMap.set(orig, u);
	return u;
}

// Neutral word bank for length-varied synthetic titles/labels.
const WORDS = [
	'meadow', 'signal', 'harbor', 'lattice', 'ember', 'orbit', 'cedar', 'ripple',
	'quartz', 'marble', 'thicket', 'summit', 'delta', 'beacon', 'willow', 'cobalt',
	'prairie', 'anchor', 'lucid', 'verdant', 'cascade', 'atlas', 'meridian', 'kestrel'
];
const textMap = new Map();
function synthText(orig) {
	let t = textMap.get(orig);
	if (t) return t;
	const seed = textMap.size;
	// 2..7 words, deterministic by insertion order, so titles vary in length.
	const count = 2 + (seed % 6);
	const words = [];
	for (let i = 0; i < count; i++) words.push(WORDS[(seed * 7 + i * 3) % WORDS.length]);
	t = words.join(' ');
	t = t.charAt(0).toUpperCase() + t.slice(1);
	textMap.set(orig, t);
	return t;
}

function sanitizeSensitive(key, value) {
	if (typeof value !== 'string' || value.length === 0) return value;
	// origin_uri: keep the scheme/shape, remap any embedded UUID, redact the rest.
	if (key === 'origin_uri') {
		return value.replace(UUID_IN_STR, (m) => synthUuid(m.toLowerCase()));
	}
	if (key === 'slug' || key === 'owner_handle') {
		// keep slug/handle-shaped (kebab), value-consistent
		return synthText(value).toLowerCase().replace(/[^a-z0-9]+/g, '-');
	}
	return synthText(value);
}

function walk(node, keyHint) {
	if (Array.isArray(node)) return node.map((v) => walk(v, keyHint));
	if (node && typeof node === 'object') {
		const out = {};
		for (const [k, v] of Object.entries(node)) out[k] = walk(v, k);
		return out;
	}
	if (typeof node === 'string') {
		if (SENSITIVE_TEXT_KEYS.has(keyHint)) return sanitizeSensitive(keyHint, node);
		// Bare UUID values anywhere (ids, focus.id, actor_entity_id, …) → remapped.
		if (UUID_RE.test(node)) return synthUuid(node.toLowerCase());
		return node;
	}
	return node;
}

const raw = JSON.parse(readFileSync(inPath, 'utf8'));
const sanitized = {};
for (const [scenario, view] of Object.entries(raw)) {
	if (scenario === '_meta') continue; // dropped; replaced with a neutral stamp below
	sanitized[scenario] = walk(view, scenario);
}
sanitized._meta = {
	synthetic: true,
	note: 'Synthetic, personal-data-free Atlas fixtures. Regenerate via scripts/sanitize-atlas-fixtures.mjs from a local capture (see README).'
};

writeFileSync(outPath, JSON.stringify(sanitized, null, '\t') + '\n');
console.log(
	`sanitized ${Object.keys(sanitized).length - 1} scenarios: ${Object.keys(sanitized).filter((k) => k !== '_meta').join(', ')}`
);
console.log(`  ${uuidMap.size} uuids remapped, ${textMap.size} text values replaced`);
console.log(`  → ${outPath}`);
