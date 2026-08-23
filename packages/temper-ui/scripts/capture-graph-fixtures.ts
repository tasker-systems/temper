/**
 * capture-graph-fixtures — build a real-shaped graph/analysis fixture bundle from production.
 *
 * # Why this is TypeScript and not a second Rust example
 *
 * `crates/temper-api/examples/capture_atlas_fixtures.rs` captured `AtlasViewData` by calling the
 * same service functions the HTTP handlers call, so its payloads were the wire DTOs *by
 * construction*. That argument still holds, but the successor's payloads are not assembled
 * server-side at all: the graph route POSTs a **composition the client builds** and renders what
 * comes back. The thing that could drift is therefore the *plan*, not the response.
 *
 * So this tool imports `buildGraphPlan`, `readableAnchors`, `resolveAnchors` and `questionFor` —
 * the very modules `+page.server.ts` imports — and sends what they produce. A captured response is
 * the answer to the composition the route would actually have sent, by construction, and there is
 * no hand-written plan here that could diverge from the one that ships.
 *
 * # What it does NOT do
 *
 * It does not sanitize. Output holds real titles, refs, handles, excerpts and ids, so it is written
 * to the **gitignored** `*.local.json` and passed through `scripts/sanitize-graph-fixtures.mjs` to
 * produce the committed bundle. See `src/routes/dev/graph/README.md`.
 *
 * Anchors are **discovered, never hardcoded** — the zero-region map, the zero-region context and
 * the densest anchors are all chosen from what production answers with at capture time. Region ids
 * in particular are ephemeral; the atlas capture learned that from a hard 500.
 *
 * # Usage
 *
 * ```bash
 * cd packages/temper-ui
 * TEMPER_TOKEN="$(temper auth export-token | tr -d '"')" \
 *   bun run scripts/capture-graph-fixtures.ts \
 *     --out src/test/fixtures/graph-harness.local.json
 * ```
 */

import { buildGraphPlan, type Anchor } from '../src/lib/graph/composition';
import { questionFor, readableAnchors } from '../src/lib/graph/entry';
import { jsonBody } from '../src/lib/server/json-body';
import type { AnchorShape } from '../src/lib/types/generated/cognitive_maps';

const BASE = process.env.TEMPER_API_BASE ?? 'https://temperkb.io';
const TOKEN = process.env.TEMPER_TOKEN ?? '';
const OUT = argOf('--out') ?? 'src/test/fixtures/graph-harness.local.json';

function argOf(flag: string): string | null {
	const i = process.argv.indexOf(flag);
	return i >= 0 && i + 1 < process.argv.length ? process.argv[i + 1] : null;
}

if (!TOKEN) {
	console.error('TEMPER_TOKEN is empty. Run: TEMPER_TOKEN="$(temper auth export-token | tr -d \'"\')"');
	process.exit(1);
}

/** Report every pick to stderr — that output is the record of which real places the bundle stands on. */
const note = (...m: unknown[]) => console.error(...m);
/** A shape that has vanished from the corpus is SKIPPED LOUDLY, never emitted as something else. */
const warn = (m: string) => console.error(`warning: ${m}`);

async function get<T>(path: string): Promise<T> {
	const res = await fetch(`${BASE}${path}`, { headers: { Authorization: `Bearer ${TOKEN}` } });
	if (!res.ok) throw new Error(`GET ${path} → HTTP ${res.status}: ${(await res.text()).slice(0, 300)}`);
	return (await res.json()) as T;
}

async function post<T>(path: string, body: unknown): Promise<T> {
	const res = await fetch(`${BASE}${path}`, {
		method: 'POST',
		headers: { Authorization: `Bearer ${TOKEN}`, 'Content-Type': 'application/json' },
		// The SAME encoder the route uses. `Composition.terms` carries `bigint` (ts-rs maps Rust's
		// `u64` that way), and a bare `JSON.stringify` throws on it — a composition built faithfully
		// against its own generated type cannot be sent without this.
		body: jsonBody(body),
	});
	if (!res.ok) throw new Error(`POST ${path} → HTTP ${res.status}: ${(await res.text()).slice(0, 300)}`);
	return (await res.json()) as T;
}

/** One anchor's regions, by the same two doors `readAnchorRegions` uses. */
const shapePath = (a: Anchor) =>
	a.kind === 'cogmap' ? `/api/cognitive-maps/${a.id}/shape` : `/api/contexts/${a.id}/shape`;

/**
 * Both doors answer an `AnchorShape` envelope, so the rows are at `.regions`. The envelope type is
 * the GENERATED one (`src/lib/types/generated/cognitive_maps`, ts-rs) — the same import
 * `src/lib/server/graph-query.ts:4` takes — rather than a hand-written mirror: a local
 * `{ regions: unknown[] }` would keep compiling after the Rust struct moved, which is exactly the
 * drift the capture exists to rule out.
 *
 * `.regions` is taken here rather than stored whole, because the bundle's `shape_rows` field is
 * consumed as `RegionLookup.rows` by `harness.ts` — the same unwrap `readAnchorRegions` does, at
 * the same place, so the capture keeps standing for what the load actually hands the builders.
 */
async function shapeRows(anchors: Anchor[]): Promise<unknown[]> {
	const reads = await Promise.allSettled(anchors.map((a) => get<AnchorShape>(shapePath(a))));
	return reads.flatMap((r) => (r.status === 'fulfilled' ? r.value.regions : []));
}

// ── Discover ────────────────────────────────────────────────────────────────────────────────────

const [contexts, cogmaps] = await Promise.all([
	get<Record<string, unknown>[]>('/api/contexts'),
	get<Record<string, unknown>[]>('/api/cognitive-maps'),
]);
const readable = readableAnchors({ contexts, cogmaps } as never);
note(`anchors readable: ${readable.length} (${contexts.length} contexts, ${cogmaps.length} maps)`);

const byMaterial = [...readable].sort((a, b) => b.resourceCount - a.resourceCount);
const densestContext = byMaterial.find((a) => a.kind === 'context') ?? null;
const densestMap = byMaterial.find((a) => a.kind === 'cogmap') ?? null;

/** A map with material and ZERO live regions — the acceptance's zero-region corpus, cogmap side. */
const coldMapRow = cogmaps.find((m) => Number(m.region_count) === 0 && Number(m.resource_count) > 0);
const coldMap = coldMapRow ? readable.find((a) => a.id === coldMapRow.id) ?? null : null;

/**
 * The MOST material context with zero shape rows — the same shape at the other door.
 *
 * Densest-first, not sparsest-first. A first pass took the sparsest and picked a one-resource
 * context, which is the *empty* case wearing the zero-region case's name: what
 * `the-unstructured-reader-is-never-worse-off` is about is a reader with real volume and no derived
 * structure at all, and a corpus of 1 cannot show that.
 */
let coldContext: Anchor | null = null;
for (const a of byMaterial.filter((x) => x.kind === 'context' && x.resourceCount > 0)) {
	if ((await get<AnchorShape>(shapePath(a))).regions.length === 0) {
		coldContext = a;
		break;
	}
}

/**
 * An anchor whose entry read ranks NOTHING eligible — the only way to reach rung 2.
 *
 * Discovered by asking, because `eligible` is a number the read reports and no list row predicts it.
 */
let rungTwo: { anchor: Anchor; bounds: Record<string, number> } | null = null;
for (const a of byMaterial) {
	const b = (await get<{ bounds: Record<string, number> }>(`/api/graph/entry?in=${a.id}`)).bounds;
	if (b.eligible === 0) {
		rungTwo = { anchor: a, bounds: b };
		break;
	}
}

note(`densest context : ${densestContext?.ref} (${densestContext?.resourceCount} resources)`);
note(`densest cogmap  : ${densestMap?.ref} (${densestMap?.resourceCount} resources)`);
note(`cold map        : ${coldMap?.ref ?? 'NOT FOUND'} (${coldMapRow?.name ?? '—'}, 0 regions)`);
note(`cold context    : ${coldContext?.ref ?? 'NOT FOUND'} (${coldContext?.resourceCount ?? 0} resources, 0 shape rows)`);
note(`rung 2 anchor   : ${rungTwo?.anchor.ref ?? 'NOT FOUND'} (bounds ${JSON.stringify(rungTwo?.bounds)})`);

// ── Capture ─────────────────────────────────────────────────────────────────────────────────────

type Bundle = Record<string, unknown>;
const bundle: Bundle = {
	_captured: {
		when: new Date().toISOString().slice(0, 10),
		against: BASE,
		synthetic: false,
		note: 'RAW capture — real titles, refs, handles and ids. Gitignored; sanitize before committing.',
	},
	/**
	 * Every readable anchor, with the display name its home carries.
	 *
	 * The load builds this from its two list reads and passes it to `buildEntryGraph` /
	 * `buildTraversal` as `homes`. Captured rather than reconstructed in the harness, because a
	 * harness-side reconstruction would be a second copy of `homesOf` free to disagree with the one
	 * that ships — and a node's home label is on screen.
	 */
	_anchors: readable.map((a) => ({
		...a,
		name:
			a.kind === 'cogmap'
				? (cogmaps.find((m) => m.id === a.id)?.name ?? a.ref)
				: a.ref,
	})),
};

/** Run one composition scenario end to end: plan → response → the shape rows that name its groupings. */
async function composition(name: string, anchors: Anchor[], asked: string | null, why: string) {
	if (anchors.length === 0) return warn(`${name}: no anchor available — skipped`);
	const question = questionFor(asked, anchors, cogmaps as never);
	const outcome = buildGraphPlan({
		anchors,
		question: question.text,
		seeds: null,
		available: anchors.length,
	});
	if (!outcome.ok) return warn(`${name}: builder refused (${outcome.reason}) — skipped`);

	const response = await post<Record<string, unknown>>('/api/query', outcome.plan.composition);
	const returned = (response.returned ?? {}) as Record<string, Record<string, unknown>>;
	const rows = await shapeRows(outcome.plan.anchorsAsked);

	bundle[name] = {
		_why: why,
		question: question.text,
		borrowedFrom: question.borrowedFrom,
		anchorsAsked: outcome.plan.anchorsAsked,
		anchorsAvailable: outcome.plan.anchorsAvailable,
		surveyStages: outcome.plan.surveyStages,
		walkStage: outcome.plan.walkStage,
		// The plan is stored so a reader can see WHICH composition this answers, and so a future
		// builder change that would have sent a different one is visible as a diff.
		composition: JSON.parse(jsonBody(outcome.plan.composition)),
		response,
		shape_rows: rows,
	};
	const stages = Object.keys(returned);
	const walk = returned[outcome.plan.walkStage] as { extent?: unknown } | undefined;
	note(
		`  ${name}: ${stages.length} stages answered, walk extent=${JSON.stringify(walk?.extent)}, ` +
			`${rows.length} shape rows`,
	);
}

// §2.1 — unaddressed, with a question. Every readable anchor, bounded by the ceiling.
await composition(
	'question',
	readable,
	'how does the graph surface decide what to show a reader',
	'§2.1 — unaddressed with a question: one survey per anchor, unioned, then walked. UNTRIMMED.',
);

// §2.2 — a named cogmap, no question. `questionFor` borrows the charter off the list row.
if (densestMap) {
	await composition('mapCharter', [densestMap], null, '§2.2 — a map surveyed under its own charter.');
}

// §2.2 at the zero-region corpus — material, no derived structure to survey.
if (coldMap) {
	await composition(
		'mapColdStart',
		[coldMap],
		null,
		'§2.2 on a map with material and ZERO live regions — the zero-region corpus, cogmap door.',
	);
}

// §2.3 — a named context, no question: find-resources-with → follow-from, no ceiling on the seeds.
if (densestContext) {
	await composition(
		'contextEverything',
		[densestContext],
		null,
		'§2.3 — a context with no question shows everything in it, at real corpus scale.',
	);
}

// §2.3 at the zero-region corpus.
if (coldContext) {
	await composition(
		'contextZeroRegion',
		[coldContext],
		null,
		'§2.3 on a context with material and zero regions — the-unstructured-reader-is-never-worse-off.',
	);
}

// The FOURTH branch the spec's §2 does not name: the entry read. The screen every reader meets
// first, and the one the unconnected band and rung 2 live on.
const entry = await get<Record<string, unknown>>('/api/graph/entry');
bundle.entry = {
	_why: 'The entry read — no question, no seeds. The first screen; carries the unconnected band.',
	entry,
};
const eNodes = (entry.nodes ?? []) as { id: string; degree?: number }[];
const eEdges = (entry.edges ?? []) as { source: string; target: string }[];
const touched = new Set(eEdges.flatMap((e) => [e.source, e.target]));
const degreeZero = eNodes.filter((n) => !touched.has(n.id)).length;
note(
	`  entry: ${eNodes.length} nodes, ${eEdges.length} edges, ${degreeZero} degree-zero ` +
		`(${((degreeZero / Math.max(eNodes.length, 1)) * 100).toFixed(1)}%), bounds=${JSON.stringify(entry.bounds)}`,
);

// The entry read confined to the zero-region map.
//
// It does NOT reach rung 2, and saying so is the point: `eligible` counts what the read could rank,
// not what has been clustered, so an anchor with zero regions still ranks its connected nodes fine.
// An earlier draft of this file labelled this scenario "the only branch that can reach rung 2" —
// true of the BRANCH and false of this fixture, which is the species of claim that renders as
// coverage and is not.
if (coldMap) {
	const narrow = await get<Record<string, unknown>>(`/api/graph/entry?in=${coldMap.id}`);
	bundle.entryZeroRegion = {
		_why: 'The entry read confined to a zero-region anchor — no derived structure, still a graph.',
		_does_not_witness: 'Rung 2. This anchor ranks nodes eligible; see `entryTooLittleStructure`.',
		entry: narrow,
	};
	note(`  entryZeroRegion: bounds=${JSON.stringify(narrow.bounds)}`);
}

// Rung 2 — `eligible === 0`, the verdict that replaces the canvas with a sentence.
if (rungTwo) {
	const narrow = await get<Record<string, unknown>>(`/api/graph/entry?in=${rungTwo.anchor.id}`);
	const b = (narrow.bounds ?? {}) as Record<string, number>;
	bundle.entryTooLittleStructure = {
		_why: 'The entry read on an anchor with nothing eligible — the ONLY branch that reaches rung 2.',
		// Declared IN the fixture rather than in a note, because a reader of this scenario would
		// otherwise take it for the case the clause is about and it is not.
		_does_not_witness:
			b.in_scope === 0
				? 'Rung 2 with MATERIAL. `in_scope` is 0 here, so this renders the degenerate sentence ' +
					'("you have nothing") rather than the one rung 2 exists for ("you have N, and too ' +
					'little structure to draw them"). No readable anchor on this corpus has eligible === 0 ' +
					'AND in_scope > 0 — measured at capture time across every anchor, not assumed.'
				: null,
		entry: narrow,
	};
	note(`  entryTooLittleStructure: ${rungTwo.anchor.ref} bounds=${JSON.stringify(b)}`);
} else {
	warn('entryTooLittleStructure: no anchor reaches eligible === 0 — rung 2 is NOT in this bundle');
}

// The traversal branch (`?from=`). Seeded from the entry read's best-connected node, discovered.
const seed = [...eNodes].sort((a, b) => (b.degree ?? 0) - (a.degree ?? 0))[0];
if (seed) {
	const walk = await get<Record<string, unknown>>(`/api/graph/traverse?from=${seed.id}&depth=1`);
	bundle.traversal = {
		_why: 'The `?from=` handoff — a walk that runs no composition. Had no caller until this bundle.',
		seeds: [seed.id],
		depth: 1,
		subgraph: walk,
	};
	note(`  traversal: from ${seed.id} (degree ${seed.degree}) → ${((walk.nodes ?? []) as unknown[]).length} nodes`);
} else {
	warn('traversal: the entry read returned no node to seed from — skipped');
}

await Bun.write(OUT, JSON.stringify(bundle, null, '\t'));
note(`\nwrote ${OUT}`);
note(`scenarios: ${Object.keys(bundle).filter((k) => k !== '_captured').join(', ')}`);
