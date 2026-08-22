#!/usr/bin/env node
// sanitize-graph-fixtures.mjs — turn a raw graph/analysis capture into a committable,
// personal-data-free fixture WITHOUT changing its shape.
//
// This repository is public. A raw capture from `scripts/capture-graph-fixtures.ts` holds real
// resource titles, refs, owner handles, context names, first-paragraph excerpts and ids, so it
// must not be committed. This script transforms a raw capture into a synthetic one that is
// *structurally identical* — every key, type, enum, number, timestamp, hash and edge-grammar
// label preserved — but carries no personal data.
//
//   • Every UUID is remapped, first-seen order, to a deterministic well-formed synthetic UUIDv7,
//     so cross-references (a region id in `shape_rows` and in `disclosed_regions`, a resource id
//     in a hit and in a `via` entry) stay linked. A remap that broke those links would destroy
//     the very structure the fixtures exist to witness.
//   • Sensitive free text is replaced value-consistently: the same source string always maps to
//     the same synthetic string, so one region named across two scenarios stays one region.
//   • Grammar and arithmetic are untouched: enums, numbers, timestamps, hashes, the query trace's
//     contract vocabulary (`means`, `field`, `scale`, `reason`) and edge labels all survive.
//
// Deterministic: same input → same output, so re-running never churns a committed file.
//
// ── Three rules that are load-bearing and easy to break by "simplifying" ──────────────────────
//
// 1. **Replacement preserves the original's LENGTH** (to the nearest word). The harness exists to
//    check legibility at the sizes the corpus actually reaches, and a label's rendered width is
//    the thing being checked — so a 90-character region label must not sanitize down to two
//    words, or the harness reports a layout as legible that production truncates.
//
// 2. **`label` is two different fields under one key**, and the rule is PATH-scoped, not
//    key-scoped. Measured across all three bundles at the time this was written: `label` under
//    `via[]` and `edges[]` is relationship grammar — 7,719 + 378 values across 19 and 32 distinct
//    strings (`advances`, `derived_from`, `relates_to`). `label` under `shape[]` / `shape_rows[]`
//    is a region's borrowed resource title — 2,872 values across ~2,850 distinct strings, i.e.
//    essentially 1:1. Sanitizing by key alone would either destroy the edge grammar the readout
//    decodes, or publish real titles.
//
// 3. **The leak guard is positive, not a denylist.** `fixtures.test.ts` asserts every free-text
//    value is built from this module's own word bank. A denylist only catches strings someone
//    thought to list.
//
// Usage:
//   node scripts/sanitize-graph-fixtures.mjs <in.json> <out.json>

import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { WORDS } from './graph-synthetic-vocabulary.mjs';

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const UUID_IN_STR = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi;

/**
 * Keys whose free-text values are personal wherever they appear.
 *
 * Built by enumerating every string-valued key across all three bundles rather than from memory
 * of the atlas list — the successor's payloads carry four keys the atlas ones never had
 * (`context_name`, `context_ref`, `cogmap_name`, and the decorated `ref`).
 */
const SENSITIVE_TEXT_KEYS = new Set([
	'title',
	'excerpt',
	'cogmap_name',
	'context_name',
	'owner_handle',
	// Slug-shaped; kept kebab so anything that composes a URL out of one still composes.
	'context_slug',
	'slug',
	// Owner-scope refs — the SIGIL is grammar and survives, the handle after it does not.
	'owner',
	'owner_ref',
	'context_owner_ref',
	'context_ref',
	// The decorated `sluggify(title)-<uuid>` form, which leaks the title AND the id.
	'ref',
	'origin_uri'
]);

/**
 * Keys that LOOK like free text and are not personal — enumerated so their survival is a
 * decision on the record rather than an omission.
 *
 * `means`, `field`, `scale`, `reason` and `bounds` are the query trace's own contract vocabulary
 * (`"alpha * sal_norm + beta * query_cos …"`, `"[-0.57, 1.05]"`). The readout renders reasoning
 * built from them, and `analysis.captured.test.ts` asserts a rendered range off `bounds`.
 * `body_hash` is a digest. Timestamps carry no identity. `temper-*` managed keys are enums.
 */
const GRAMMAR_KEYS = new Set([
	'edge_kind', 'polarity', 'doc_type', 'doc_type_name', 'home', 'ingest_state', 'body_storage',
	'score_kind', 'act', 'disposition', 'extent', 'stage', 'kind', 'relation', 'produced',
	'op', 'as', 'from', 'source', 'means', 'field', 'scale', 'reason', 'bounds', 'body_hash',
	'created', 'updated', 'materialized_at', 'latest_touch'
]);

/**
 * `name` is two different fields under one key — the same trap as `label`, found the same way.
 *
 * Under `composition.stages[]` and `trace.stages[]` a `name` is a STAGE id (`s1`, `m1`, `w`): pure
 * grammar, and the readout pairs arms to stages by it. Under `borrowedFrom` it is a **cogmap's
 * name**, which is authored and personal. It sat on the grammar list on the first pass and the
 * real map name went straight through into the output.
 *
 * Worth recording how it was caught: a **denylist probe** on the output — grep for a handful of
 * known-real strings. That is precisely the check this file's own header says is insufficient,
 * and it is insufficient; it caught this one only because the map's name happens to contain
 * `temper`. The positive word-bank guard in `fixtures.test.ts` is what must catch the next one,
 * because the next one will be a string nobody thought to grep for.
 */
const isStageName = (path) => path.includes('stages');

// ── UUID remap ──────────────────────────────────────────────────────────────────────────────

const uuidMap = new Map();
function synthUuid(orig) {
	const key = orig.toLowerCase();
	let u = uuidMap.get(key);
	if (u) return u;
	const n = uuidMap.size + 1;
	u = `0191d0c0-${n.toString(16).padStart(4, '0')}-7000-8000-${n.toString(16).padStart(12, '0')}`;
	uuidMap.set(key, u);
	return u;
}

// ── Length-preserving text remap ────────────────────────────────────────────────────────────

const textMap = new Map();
const usedText = new Set();

/** One candidate replacement of roughly `target` characters, selected by `salt`. */
function candidate(salt, target) {
	const words = [];
	let len = 0;
	for (let i = 0; ; i++) {
		const w = WORDS[(salt * 7 + i * 3) % WORDS.length];
		const grown = len + (words.length > 0 ? 1 : 0) + w.length;
		if (words.length > 0 && grown > target) break;
		words.push(w);
		len = grown;
	}
	const t = words.join(' ');
	return t.charAt(0).toUpperCase() + t.slice(1);
}

/**
 * Value-consistent synthetic replacement that preserves the original's LENGTH **and its
 * DISTINCTNESS**.
 *
 * Length is matched to the nearest WORD BOUNDARY, not exactly: the replacement is always whole
 * words from the bank, so the guard can assert the committed vocabulary positively. A one-word
 * floor means a very short original may come back slightly longer; that direction is harmless
 * for a legibility check, which is bounded by the long tail.
 *
 * **Injectivity is the second requirement, and it was learned by breaking it.** A first version
 * preserved length only. Two distinct region labels of similar length collided onto one synthetic
 * string, and `GraphPage` renders groupings in a KEYED `{#each}` — so Svelte threw
 * `each_key_duplicate` and twelve component tests failed. The fixture was still the right shape
 * and the right size, and it had quietly stopped being the right *cardinality*.
 *
 * That is this repository's own standing lesson turning up one level down: *a trim that preserves
 * one property destroys another*. The remap is a trim over the space of strings, length was the
 * property it was written to preserve, and uniqueness was the one it destroyed.
 *
 * So: retry with a fresh salt until the candidate is unused. If the target length is too short to
 * hold a unique combination — the bank is finite, and a 6-character original has few — grow by one
 * word and try again. That terminates, because the space grows without bound as words are added,
 * and it errs toward *longer*, which is the harmless direction already argued for above.
 */
export function synthText(orig) {
	const existing = textMap.get(orig);
	if (existing) return existing;
	let target = orig.length;
	for (let attempt = 0; ; attempt++) {
		// Widen only after the salt space at this length has been given a fair run.
		if (attempt > 0 && attempt % 64 === 0) target += 8;
		const t = candidate(textMap.size + attempt * 131, target);
		if (usedText.has(t)) continue;
		usedText.add(t);
		textMap.set(orig, t);
		return t;
	}
}

const kebab = (s) => synthText(s).toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');

/**
 * Owner-scope refs. The sigil is grammar — `@` denotes a profile and `+` a team, and the two
 * must stay distinguishable — while the handle after it is personal.
 *
 * `@me` and `shared` pass through: both are PII-free, and `@me` is the canonical self-addressed
 * form every real page load carries, so replacing it would be strictly worse — still anonymous,
 * but no longer the value the route actually produces.
 */
function sanitizeRefLike(value) {
	if (value === '@me' || value === 'shared') return value;
	const sigil = value[0] === '@' || value[0] === '+' ? value[0] : '';
	const body = sigil ? value.slice(1) : value;
	// `@owner/slug` — two personal parts, each remapped independently so a slug shared across
	// owners stays shared.
	if (body.includes('/')) {
		const [owner, ...rest] = body.split('/');
		return `${sigil}${kebab(owner)}/${rest.map(kebab).join('/')}`;
	}
	// The decorated `sluggify(title)-<uuid>` form. The uuid half is the resolvable part and must
	// stay linked to the same id everywhere; the slug half is presentation built from the title.
	const m = body.match(/^(.*?)-?([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$/i);
	if (m) return `${sigil}${m[1] ? `${kebab(m[1])}-` : ''}${synthUuid(m[2])}`;
	return sigil + kebab(body);
}

/** Keep the scheme and the segment count; remap embedded uuids; synthesize the rest. */
function sanitizeOriginUri(value) {
	const m = value.match(/^([a-z0-9+.-]+:\/\/)(.*)$/i);
	if (!m) return value.replace(UUID_IN_STR, (u) => synthUuid(u));
	const segs = m[2].split('/').map((s) => (UUID_RE.test(s) ? synthUuid(s) : s ? kebab(s) : s));
	return m[1] + segs.join('/');
}

function sanitizeSensitive(key, value) {
	if (key === 'origin_uri') return sanitizeOriginUri(value);
	if (key === 'ref' || key === 'owner' || key === 'owner_ref' || key === 'context_owner_ref' || key === 'context_ref')
		return sanitizeRefLike(value);
	if (key === 'slug' || key === 'context_slug' || key === 'owner_handle') return kebab(value);
	return synthText(value);
}

/**
 * Is this `label` an edge's relationship grammar, or a region's borrowed resource title?
 *
 * Edge labels live under `via[]` (a `QueryResponse` hit's provenance) and `edges[]` (the entry
 * read and the traversal subgraph). Every other `label` traces back to a resource title:
 * `anchor_shape_select` computes a region's as `COALESCE(reg.label, rep_title)`, so an
 * unlabelled region borrows a member's TITLE.
 */
const isEdgeGrammarLabel = (path) => path.includes('via') || path.includes('edges');

/** `open_meta` is caller-authored JSON of no fixed shape, so no key list can cover it. */
const isOpenMeta = (path) => path.includes('open_meta');

/** Keys this bundle's own annotations live under — prose we wrote, about the fixture itself. */
const isAnnotation = (key) => typeof key === 'string' && key.startsWith('_');

function walk(node, keyHint, path) {
	if (Array.isArray(node)) return node.map((v) => walk(v, keyHint, path));
	if (node && typeof node === 'object') {
		const out = {};
		for (const [k, v] of Object.entries(node)) out[k] = walk(v, k, [...path, k]);
		return out;
	}
	if (typeof node !== 'string') return node;

	// Bare UUIDs anywhere → remapped. Checked FIRST so a uuid sitting under a sensitive key stays
	// a well-formed, cross-linked id rather than becoming prose.
	if (UUID_RE.test(node)) return synthUuid(node);
	// Our own annotations about the fixture, and the question the capture asked, are not personal.
	// Our own annotations ABOUT the fixture are prose we wrote, and are not personal.
	if (isAnnotation(keyHint)) return node;
	// `question` / `query` are NOT safe to pass through, and the reason is §2.2. When the reader
	// asks, the question is theirs and harmless; when they name a map and ask nothing, `questionFor`
	// **borrows the map's charter** — authored prose out of the vault — and it arrives under exactly
	// the same key. One key, two provenances, and only one of them is publishable. Length-preserving,
	// so the ask box is still exercised at the width a real charter reaches.
	if (keyHint === 'query' || keyHint === 'question') return synthText(node);
	if (keyHint === 'name') return isStageName(path) ? node : synthText(node);
	if (GRAMMAR_KEYS.has(keyHint)) return node;
	if (SENSITIVE_TEXT_KEYS.has(keyHint)) return sanitizeSensitive(keyHint, node);
	if (keyHint === 'label') return isEdgeGrammarLabel(path) ? node : synthText(node);
	if (isOpenMeta(path) && node.length > 0) return synthText(node);
	// Anything with an embedded uuid (a branch name, a charter reference) still gets it remapped.
	if (UUID_IN_STR.test(node)) return node.replace(UUID_IN_STR, (u) => synthUuid(u));
	return node;
}

/** Sanitize a whole bundle. Exported and separate from the CLI, so importing has no side effect. */
export function sanitizeBundle(raw) {
	const out = walk(raw, null, []);
	out._sanitized = {
		synthetic: true,
		note:
			'Structure-preserving synthetic remap of a real capture. Every id, count, number, ' +
			'timestamp and edge label is the real one; every title, label, ref, handle and excerpt ' +
			'is drawn from scripts/graph-synthetic-vocabulary.mjs. Regenerate with ' +
			'scripts/sanitize-graph-fixtures.mjs from a local capture.',
		uuids_remapped: uuidMap.size,
		texts_remapped: textMap.size
	};
	return out;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
	const [, , inPath, outPath] = process.argv;
	if (!inPath || !outPath) {
		console.error('usage: node scripts/sanitize-graph-fixtures.mjs <in.json> <out.json>');
		process.exit(1);
	}
	const raw = JSON.parse(readFileSync(inPath, 'utf8'));
	const clean = sanitizeBundle(raw);
	writeFileSync(outPath, `${JSON.stringify(clean, null, '\t')}\n`);
	console.error(
		`${inPath} → ${outPath}: ${uuidMap.size} uuids, ${textMap.size} texts remapped`
	);
}
