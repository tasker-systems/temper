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
//     EDGE `label`), hashes (body_hash), numbers and timestamps are kept as-is —
//     they carry no personal data and the edge-grammar legend depends on real labels.
//
// Deterministic: same input → same output, so re-running never churns the committed file.
//
// ── `label` is two different fields under one key ──────────────────────────────────────
//
// An EDGE label is relationship grammar (`derived_from`, `advances`) and must survive — the
// legend renders it. A TERRITORY label is not: `graph_cogmap_territories` computes it as
// `COALESCE(reg.label, seen.rep_title)`, so an unlabelled region borrows a member RESOURCE
// TITLE, and a context container's label IS its goal's title. Sanitizing by key alone would
// therefore either destroy the edge grammar or publish real titles. The rule is path-scoped:
// `label` is grammar under `neighborhood.edges`, and personal everywhere else.
//
// Usage:
//   node scripts/sanitize-atlas-fixtures.mjs [inPath] [outPath]
//   defaults: static/dev/atlas-fixtures.local.json → static/dev/atlas-fixtures.json

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
// Shared with `fixtures.test.ts`, which asserts the committed bundle's free text is drawn
// from this bank and nothing else. See that module's header for why it is separate.
import { WORDS } from './atlas-synthetic-vocabulary.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const uiRoot = resolve(here, '..');

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const UUID_IN_STR = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi;

// Keys whose free-text values are personal and must be replaced wherever they appear.
// Grammar/enum keys are deliberately absent.
//
// `excerpt` and `actor_name` are here because the Rust capture path surfaces both and the
// browser capture it replaced did not: `AtlasNode.excerpt` is a first-paragraph body preview
// (real prose), and `EventTrail.events[].actor_name` is a real person's display name.
const SENSITIVE_TEXT_KEYS = new Set([
	'title',
	'name',
	'cogmapName',
	'cogmap_name',
	'anchor_label',
	'owner_handle',
	'slug',
	// `AtlasViewData.contextSlug` — the same value as the wire-level `slug` above, under the
	// view's camelCase name. Listing only `slug` covered the payload and missed the wrapper.
	'contextSlug',
	'origin_uri',
	'excerpt',
	'actor_name',
	// Owner-scope refs — see `OWNER_REF_KEYS` / `sanitizeOwnerRef` for why the sigil survives
	// and the handle does not. All three were live leaks in the first capture of this refresh.
	'owner',
	'owner_ref',
	'context_owner_ref'
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

const textMap = new Map();

/**
 * Value-consistent synthetic replacement that PRESERVES THE ORIGINAL'S LENGTH.
 *
 * Length matters: `/dev/atlas` exists to check legibility at the sizes the corpus actually
 * reaches, and a label's rendered width is the thing being checked. The previous version
 * varied length by insertion order (2..7 words regardless of input), which silently
 * decoupled the fixture's label widths from the real ones — so a 90-character region label
 * could sanitize down to two words and the harness would report a layout as legible that
 * production truncates.
 *
 * Length is matched to the nearest WORD BOUNDARY, not exactly: the replacement is always
 * whole words from the bank, so `fixtures.test.ts` can assert the committed vocabulary
 * positively. A one-word floor means a very short original may come back slightly longer;
 * that direction is harmless for a legibility check, which is bounded by the long tail.
 *
 * Deterministic: same input → same output. The insertion-order seed is mixed into word
 * selection so two distinct originals of equal length still get distinct replacements.
 */
export function synthText(orig) {
	let t = textMap.get(orig);
	if (t) return t;
	const seed = textMap.size;
	const target = orig.length;
	const words = [];
	let len = 0;
	for (let i = 0; ; i++) {
		const w = WORDS[(seed * 7 + i * 3) % WORDS.length];
		const grown = len + (words.length > 0 ? 1 : 0) + w.length; // +1 for the joining space
		if (words.length > 0 && grown > target) break;
		words.push(w);
		len = grown;
	}
	t = words.join(' ');
	t = t.charAt(0).toUpperCase() + t.slice(1);
	textMap.set(orig, t);
	return t;
}

/**
 * Owner-scope refs: the `/graph/[owner]` route param plus the two the reads return
 * (`HomeContext`/`HomeCogmap.owner_ref`, `ResourceRow.context_owner_ref`).
 *
 * These are NOT free text — the Home lens tints by `owner_ref`, and a `context_owner_ref`
 * composes into the `owner/slug` a vault route resolves. So the SIGIL is grammar and must
 * survive, while the handle after it is personal and must not.
 */
const OWNER_REF_KEYS = new Set(['owner', 'owner_ref', 'context_owner_ref']);

/** Sigil-preserving owner-ref replacement. `@me` and `shared` are PII-free and pass through. */
function sanitizeOwnerRef(key, value) {
	if (value === '@me' || value === 'shared') return value;
	// The `/graph/[owner]` route param specifically. A capture is always taken AS the
	// authenticated profile, so a literal handle here denotes that profile itself — and the
	// self-addressed form of "me" is `@me`, which is both the canonical value every real page
	// load carries and free of personal data. Rendering it as a synthetic handle instead would
	// be strictly worse: still anonymous, but no longer the value the route actually produces.
	// This does NOT apply to `owner_ref` / `context_owner_ref`, which can denote a TEAM
	// (`+slug`) and must keep their distinct identity.
	if (key === 'owner') return '@me';
	const sigil = value.startsWith('@') || value.startsWith('+') ? value[0] : '';
	const body = sigil ? value.slice(1) : value;
	return sigil + synthText(body).toLowerCase().replace(/[^a-z0-9]+/g, '-');
}

function sanitizeSensitive(key, value) {
	if (typeof value !== 'string' || value.length === 0) return value;
	// origin_uri: keep the scheme/shape, remap any embedded UUID, redact the rest.
	if (key === 'origin_uri') {
		return value.replace(UUID_IN_STR, (m) => synthUuid(m.toLowerCase()));
	}
	if (OWNER_REF_KEYS.has(key)) return sanitizeOwnerRef(key, value);
	if (key === 'slug' || key === 'contextSlug' || key === 'owner_handle') {
		// keep slug/handle-shaped (kebab), value-consistent
		return synthText(value).toLowerCase().replace(/[^a-z0-9]+/g, '-');
	}
	return synthText(value);
}

/**
 * Is this `label` an edge's relationship grammar, or a territory's borrowed title?
 * Edge labels live under `neighborhood.edges[]`; every other `label` in an `AtlasViewData`
 * (territory, container, crumbTerritory) traces back to a resource title.
 */
const isEdgeGrammarLabel = (path) => path.includes('edges');

/**
 * `open_meta` is caller-authored JSON of no fixed shape, so no key list can cover it.
 * Every string leaf beneath it is treated as free text.
 */
const isOpenMeta = (path) => path.includes('open_meta');

function walk(node, keyHint, path = []) {
	if (Array.isArray(node)) return node.map((v) => walk(v, keyHint, path));
	if (node && typeof node === 'object') {
		const out = {};
		for (const [k, v] of Object.entries(node)) out[k] = walk(v, k, [...path, k]);
		return out;
	}
	if (typeof node === 'string') {
		// Bare UUID values anywhere (ids, focus.id, actor_entity_id, …) → remapped. Checked
		// first so a UUID sitting under a sensitive key stays a well-formed, cross-linked id
		// rather than becoming prose.
		if (UUID_RE.test(node)) return synthUuid(node.toLowerCase());
		if (SENSITIVE_TEXT_KEYS.has(keyHint)) return sanitizeSensitive(keyHint, node);
		if (keyHint === 'label' && !isEdgeGrammarLabel(path)) return synthText(node);
		if (isOpenMeta(path) && node.length > 0) return synthText(node);
		return node;
	}
	return node;
}

// Beat 2b: ensure the harness can render node content even from a pre-backend capture.
// Synthesize an excerpt on neighborhood nodes and a payload + actor_name on trail events
// when a (pre-backend) capture lacks them. Deterministic, personal-data-free.
function ensureNodeContent(view) {
	if (view && view.neighborhood && Array.isArray(view.neighborhood.nodes)) {
		view.neighborhood.nodes.forEach((n, i) => {
			if (n.excerpt === undefined)
				n.excerpt =
					i % 3 === 0
						? null
						: `${synthText('excerpt' + i)} — ${synthText('excerpt-tail' + i)}.`;
		});
	}
	if (view && view.trail && Array.isArray(view.trail.events)) {
		view.trail.events.forEach((e, i) => {
			if (e.actor_name === undefined) e.actor_name = i % 2 === 0 ? 'system' : synthText('actor' + i);
			if (e.payload === undefined)
				e.payload =
					e.kind === 'property_set'
						? { property_key: 'temper-stage', value: 'in-progress', weight: 1 }
						: e.kind === 'relationship_asserted'
							? { label: 'derived_from', target: { id: view.neighborhood?.nodes?.[0]?.id ?? '0' } }
							: { note: synthText('payload' + i) };
		});
	}
	return view;
}

/**
 * Sanitize a whole raw bundle. Exported, and separated from the CLI entry point below, so this
 * module can be imported without reading and WRITING files as an import side effect.
 */
export function sanitizeBundle(raw) {
	const sanitized = {};
	for (const [scenario, view] of Object.entries(raw)) {
		if (scenario === '_meta') continue; // dropped; replaced with a neutral stamp below
		sanitized[scenario] = walk(view, scenario, []);
	}
	// Second pass, after every scenario's real data has been walked: synthesize the new
	// Beat 2b fields. Kept out of the walk loop above so the excerpt/payload synthText()
	// calls (which mint brand-new first-seen strings) can never interleave with — and
	// thereby shift the seed counter for — a later scenario's real title/text sanitization.
	for (const scenario of Object.keys(sanitized)) {
		ensureNodeContent(sanitized[scenario]);
	}
	sanitized._meta = {
		synthetic: true,
		note: 'Synthetic, personal-data-free Atlas fixtures. Regenerate via scripts/sanitize-atlas-fixtures.mjs from a local capture (see README).'
	};
	return sanitized;
}

// CLI entry point — guarded so importing this module (for `WORDS`) neither reads nor writes.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
	const inPath = resolve(uiRoot, process.argv[2] ?? 'static/dev/atlas-fixtures.local.json');
	const outPath = resolve(uiRoot, process.argv[3] ?? 'static/dev/atlas-fixtures.json');
	const sanitized = sanitizeBundle(JSON.parse(readFileSync(inPath, 'utf8')));
	writeFileSync(outPath, JSON.stringify(sanitized, null, '\t') + '\n');
	console.log(
		`sanitized ${Object.keys(sanitized).length - 1} scenarios: ${Object.keys(sanitized).filter((k) => k !== '_meta').join(', ')}`
	);
	console.log(`  ${uuidMap.size} uuids remapped, ${textMap.size} text values replaced`);
	console.log(`  → ${outPath}`);
}
