import type { ResourceView } from '$lib/types/generated/resource_view';

/**
 * Single authority for `/vault/...` route URLs. Every nav link, back link,
 * row-click, and the Atlas rail's "View full resource" button routes through
 * these builders so addressing can never drift per call site.
 *
 * `ownerRef` is already sigil'd (`@<handle>` / `+<team-slug>`) and is NOT
 * percent-encoded — the sigils are valid path chars the `[owner]` route matches
 * literally. `slug` and `docType` are encoded defensively.
 */

export function contextHref(ownerRef: string, slug: string): string {
	return `/vault/${ownerRef}/${encodeURIComponent(slug)}`;
}

/**
 * The graph door for one context. Both the left-nav "Graph" link and the Home build
 * circle resolve here, so there is exactly one context-graph URL in the app.
 *
 * **It emits an `in` anchor, not a `?context=` scope** `[2026-08-20]`. The graph surface reads
 * `q`/`in`/`from`/`sel` and knows nothing about `context`, so this had to move in the same change
 * that rebuilt the route rather than in a later one. The failure in that window would not have been
 * the loud kind: an absent `in` means *"every readable anchor, bounded and declared"*, so a
 * context-scoped nav click would have **silently widened to the whole corpus** while the bound line
 * truthfully reported `12 of 12 places` — plausible, well-formed and wrong.
 *
 * There is deliberately **no backward compatibility for `?context=`** `[decided — 2026-08-20, Pete]`.
 * Nobody holds a bookmark to this surface yet, so the only consumer of the old spelling is in-app
 * nav, which this change updates. Accepting it would also have to resolve a bare slug against the
 * route's `[owner]` — the owner-scoping the grammar rejects precisely because it makes a team
 * context inexpressible — and would leave a compatibility path someone has to remember to delete.
 *
 * Delegates to {@link graphHref} rather than formatting a URL of its own: the anchor grammar and its
 * encoding have exactly one author, and a second spelling here is how the two would drift.
 */
export function contextGraphHref(ownerRef: string, slug: string): string {
	return graphHref(ownerRef, { anchors: [{ kind: 'context', ref: `${ownerRef}/${slug}` }] });
}

/** SvelteKit's `page.params` for the routes that can address a context. */
export interface ContextLocationParams {
	owner?: string;
	context?: string;
}

/**
 * Inverse of the two builders above: does `url` address `ownerRef`'s `slug` context?
 *
 * The two doors carry it differently, so this answers in two ways rather than one.
 * `contextHref` carries the context as the `[context]` path segment, matched by the router — so
 * there the route's `[owner]` is the owner, and both must agree.
 *
 * `contextGraphHref` carries it as an **`in` anchor holding a whole ref**, which is read back
 * through {@link parseGraphAddress} — the same parser the route itself loads from, so nav can never
 * disagree with the page about which places are being asked. And on that door the route's `[owner]`
 * is deliberately **not** consulted: `/graph/@me` may legitimately ask about `+team/ops`, which is
 * the reachability the anchor grammar exists to provide. Checking it would mark a team context
 * inactive on the very screen that is showing it.
 */
export function isContextLocation(
	params: ContextLocationParams,
	url: URL,
	ownerRef: string,
	slug: string,
): boolean {
	if (params.context !== undefined) {
		return params.owner === ownerRef && params.context === slug;
	}
	const ref = `${ownerRef}/${slug}`;
	return parseGraphAddress(url).anchors.some((a) => a.kind === 'context' && a.ref === ref);
}

/** True only on the Atlas door for `ownerRef`'s `slug` context. */
export function isContextGraphLocation(
	params: ContextLocationParams,
	url: URL,
	ownerRef: string,
	slug: string,
): boolean {
	return isContextLocation(params, url, ownerRef, slug) && url.pathname.startsWith('/graph/');
}

/**
 * Path to a resource, for any home. Resolution is trailing-UUID-only, so the
 * route needs nothing but the id — home is a rendered fact, not a routing
 * precondition (spec D1).
 *
 * This used to return `null` for a cogmap-homed resource (context_* are null),
 * which stranded 533 of 2330 active resources: VaultGrid listed them and
 * no-opped on click. It cannot return null now.
 *
 * It takes only the `id` because only the id is used. A caller holding a resource id and no row —
 * the analysis door's charter and regulation links — should not have to fake a `ResourceView` to
 * build the one link this repo has a single source for.
 */
export function resourceHref(row: Pick<ResourceView, 'id'>): string {
	return `/vault/r/${row.id}`;
}

export function searchHref(query: string): string {
	return `/vault/search?q=${encodeURIComponent(query)}`;
}

/**
 * Is this row homed in a cognitive map rather than a context?
 *
 * A resource is homed by exactly one anchor (`kb_resource_homes.anchor_table`), so the unused
 * half is ABSENT from the wire, not null: `ResourceView` carries
 * `skip_serializing_if = "Option::is_none"` on both `cogmap_*` and `context_*`. A context-homed
 * row therefore has no `cogmap_id` KEY at all.
 *
 * Hence `!=` rather than `!==`, which is the whole reason this is a named function instead of an
 * inline comparison. `row.cogmap_id !== null` is `true` for EVERY context-homed row, and the
 * ts-rs binding cannot catch it: it declares `cogmap_id: string | null` — a *required* key that
 * can never be `undefined` — so a strict comparison type-checks as exhaustive while being
 * always-true. `svelte-check` is structurally unable to see this class, so it needs a test.
 */
export function isCogmapHomed(row: ResourceView): boolean {
	return row.cogmap_id != null;
}

/**
 * One anchor as a URL carries it: a KIND and a WHOLE ref.
 *
 * `/graph/[owner]` names the reader whose graph this is, but the anchors they may read are not all
 * theirs — a team context is `+team/<slug>` and is routinely in reach. Scoping `ctx:` against the
 * route's `[owner]` would make a team context inexpressible at the very door whose purpose is
 * spanning every readable anchor, so the ref travels whole. A cogmap needs no owner, being addressed
 * by uuid.
 *
 * **No region id ever enters this grammar.** Region identity is not durable — `assert_region`
 * (`temper-substrate/src/write.rs:673`) mints a new id whenever a member set changes — so an
 * address holding one would go stale by derivation rather than by the reader deleting anything.
 * Anchors are `kb_contexts`/`kb_cogmaps` rows and seeds are `kb_resources` rows; all three are
 * durable, and a seed that no longer resolves is an honest 404 about the reader's own material.
 */
export interface GraphAnchorRef {
	kind: 'context' | 'cogmap';
	/** `@owner/slug` or `+team/slug` for a context; the uuid for a cogmap. */
	ref: string;
}

/** The graph surface's whole addressable state. Every part is a part of the composition. */
export interface GraphAddress {
	question: string | null;
	anchors: GraphAnchorRef[];
	seeds: string[];
	selection: string | null;
	/**
	 * How many hops a traversal walks from {@link GraphAddress.seeds}.
	 *
	 * **Grammar only** `[ruled — 2026-08-21, Pete]`. It is parsed, emitted and clamped, and every
	 * hop this surface emits writes `1`. No control ships: §10.2 ruled an address, not a widget, and
	 * a depth the reader cannot set is still worth addressing — it is what lets a hand-written or
	 * shared URL say exactly which read produced the screen.
	 *
	 * `null` when absent or unreadable, never a defaulted number. The default lives in the read
	 * (`/api/graph/traverse` takes `depth: Option<i32>`, `unwrap_or(1)`), and duplicating it here
	 * would put two copies of one rule on either side of the wire.
	 */
	depth: number | null;
}

/**
 * The hop range the service will actually walk — `LEAST(p_depth, 3)` in the SQL, `depth.clamp(1, 3)`
 * in `graph_service::traversal_slice`.
 *
 * Clamped rather than rejected, and clamped **here** rather than trusted to the server, so the
 * address and the screen cannot disagree: an out-of-range `depth` in a hand-written URL draws the
 * graph it will actually get, instead of one the reader can read a different number for.
 */
const clampDepth = (n: number): number => Math.min(3, Math.max(1, Math.trunc(n)));

const ANCHOR_PREFIX: Record<string, GraphAnchorRef['kind']> = {
	ctx: 'context',
	map: 'cogmap',
};

/**
 * Undo form-encoding's one collision with the team sigil.
 *
 * A query string is `application/x-www-form-urlencoded`, where a literal `+` MEANS a space. So a
 * hand-written `?in=ctx:+acme/ops` — the shape a reader gets by pasting a ref straight out of
 * `temper context list` — decodes to `ctx: acme/ops`. {@link graphHref} has no such problem; it
 * emits `%2B` and round-trips. This is only the hand-written path.
 *
 * **This is an inverse, not a guess.** No ref may begin with whitespace, so a leading space can only
 * have come from a `+`. The alternative was trimming, and trimming is the dangerous option rather
 * than the conservative one: it turns `ctx: acme/ops` into `acme/ops`, silently rewriting a team
 * anchor into a DIFFERENT — possibly real — owner's, which is precisely the silent widening this
 * parser refuses elsewhere by dropping what it cannot read.
 */
const restoreTeamSigil = (ref: string): string =>
	ref.startsWith(' ') ? `+${ref.slice(1).trim()}` : ref.trim();

/**
 * Read the composition's parts out of a graph URL.
 *
 * **The URL is a projection of the composition** — no state on screen that the URL does not
 * describe, and no param that does not name a part of the plan. So this is the inverse of
 * {@link graphHref} and the two are tested as a round trip.
 *
 * An unreadable `in` value is DROPPED rather than guessed at. A bare slug is not the grammar: it
 * would have to be resolved against the route's `[owner]`, which is exactly the owner-scoping this
 * grammar exists to avoid, and a silently-widened anchor is a worse answer than a missing one.
 */
export function parseGraphAddress(url: URL): GraphAddress {
	const params = url.searchParams;

	const question = params.get('q')?.trim();
	const selection = params.get('sel')?.trim();

	const anchors: GraphAnchorRef[] = [];
	for (const raw of params.getAll('in')) {
		const at = raw.indexOf(':');
		if (at < 0) continue;
		const kind = ANCHOR_PREFIX[raw.slice(0, at)];
		const ref = restoreTeamSigil(raw.slice(at + 1));
		if (kind && ref) anchors.push({ kind, ref });
	}

	// Unreadable is DROPPED, exactly as an unreadable `in` is — a depth that cannot be read is not
	// evidence for any particular number of hops, and defaulting one here would silently answer a
	// different question than the address asks.
	const rawDepth = Number(params.get('depth'));
	const depth = Number.isFinite(rawDepth) && rawDepth !== 0 ? clampDepth(rawDepth) : null;

	return {
		question: question ? question : null,
		anchors,
		seeds: params.getAll('from').filter((s) => s.length > 0),
		selection: selection ? selection : null,
		depth,
	};
}

/**
 * Build a graph URL from the composition's parts — the inverse of {@link parseGraphAddress}.
 *
 * `ownerRef` keeps its sigils, as every other builder in this file does: they are valid path chars
 * the `[owner]` route matches literally. Everything else rides in the query string and is encoded by
 * `URLSearchParams`, which is what lets a context slug hold a space or an ampersand and survive.
 */
export function graphHref(ownerRef: string, address: Partial<GraphAddress>): string {
	const params = new URLSearchParams();

	if (address.question?.trim()) params.set('q', address.question);
	for (const a of address.anchors ?? []) {
		params.append('in', `${a.kind === 'cogmap' ? 'map' : 'ctx'}:${a.ref}`);
	}
	for (const seed of address.seeds ?? []) params.append('from', seed);
	if (address.depth != null) params.set('depth', String(clampDepth(address.depth)));
	if (address.selection) params.set('sel', address.selection);

	const query = params.toString();
	return query ? `/graph/${ownerRef}?${query}` : `/graph/${ownerRef}`;
}

/**
 * The same graph URL with a different selection.
 *
 * **`sel` carries a bare resource uuid, with no kind prefix** — deliberately unlike the
 * predecessor's `node:`/`edge:` grammar. The successor's marks are nodes and edges, and an edge is
 * a `ViaEntry`, which carries `seed_id`, `source_id`, `target_id`, `edge_kind`, `label` and
 * `polarity` and **no id at all**. So an edge has no durable address to put in a URL, only one kind
 * is selectable, and a prefix would name a distinction that does not exist.
 *
 * This rewrites the URL in place rather than rebuilding it from an address, so `q`, `in` and `from`
 * travel untouched — a selection must never silently change what was asked.
 */
export function withGraphSelection(url: URL, selection: string | null): string {
	const next = new URL(url);
	if (selection) next.searchParams.set('sel', selection);
	else next.searchParams.delete('sel');
	return `${next.pathname}${next.search}`;
}

/**
 * The same graph URL asking a different question — **the walk ends, and the selection goes.**
 *
 * The selection goes because it names a node in the previous answer: keeping it would open a rail
 * onto something the new answer may not contain, which is a panel describing a resource that is
 * not on screen.
 *
 * `[amended — 2026-08-21]` **The walk goes for a sharper reason: without this, the Ask box does
 * nothing on a traversed screen.** §10.3 splits the two acts — grounding sets the space, navigation
 * moves inside it — and D2 routes any address carrying `from` to the traversal read, which never
 * looks at `q`. So a reader who typed a new question while standing on a hop would watch the URL
 * change and the graph stay exactly as it was: a control that appears to work and does not, which
 * is `no-reader-is-left-to-blame-themselves` in its purest form and the same shape as the click
 * that opened no rail.
 *
 * Asking is therefore a **re-grounding**: it ends the walk rather than filtering it. That is the
 * split read in the other direction, and it is why this deletion belongs here rather than in a
 * guard on the routing.
 */
export function withGraphQuestion(url: URL, question: string | null): string {
	const next = new URL(url);
	const q = question?.trim();
	if (q) next.searchParams.set('q', q);
	else next.searchParams.delete('q');
	next.searchParams.delete('sel');
	next.searchParams.delete('from');
	next.searchParams.delete('depth');
	return `${next.pathname}${next.search}`;
}

/**
 * The same graph URL walking from one named seed — **the question survives, the selection does not.**
 *
 * `from` *replaces* the upstream stage as what the walk grows from: the reader has named where to
 * walk from, and the pipe is no longer the answer to that.
 *
 * `[amended — 2026-08-21]` **`q` used to be deleted here, and the reason given was right about the
 * fact and wrong about the response.** It said *"carrying a stale `q` alongside would leave a
 * question on screen that no longer decides anything about the answer"* — which is true, and is
 * exactly why §10.2 rules that it stays: *"`q` survives the handoff as provenance, and that is what
 * makes §7.2 work."* A question that no longer decides anything is not stale, it is **where the
 * reader started**, and dropping it is what left them with no route back to it. What must not
 * happen is a screen that reads as though the question were still narrowing — and that is the
 * panel's job to say, not this function's to prevent by deletion.
 *
 * The selection still goes, unchanged: it names a node in the previous answer, and the walk may not
 * contain it.
 *
 * **Every hop this surface emits walks one hop.** `depth` is grammar, not a control (see
 * {@link GraphAddress.depth}), so it is written rather than carried forward — a second hop from a
 * `depth=3` screen is still one hop from where the reader now stands.
 */
export function withGraphSeed(url: URL, seed: string): string {
	const next = new URL(url);
	next.searchParams.delete('sel');
	next.searchParams.delete('from');
	// Deleted before it is written, not `set` in place: `set` keeps an existing key's ORIGINAL
	// position, so hopping from a `?from=x&depth=3` screen would emit `depth` before `from` while
	// hopping from a screen without one emits it after. Same address, two spellings, decided by
	// where the reader happened to come from.
	next.searchParams.delete('depth');
	next.searchParams.append('from', seed);
	next.searchParams.set('depth', '1');
	return `${next.pathname}${next.search}`;
}

/**
 * The grounding a traversed view descends from — {@link withGraphSeed} undone.
 *
 * §7.2 rejected letting *Why these* simply disappear on a hop precisely because that *"loses the
 * reader's route back to how they got here."* This is that route: the same address with the walk
 * taken off it, so `q` and `in` land the reader back on the screen they started from.
 *
 * The selection goes too. It names a node in the TRAVERSAL's answer, and the grounding may not
 * contain it — the same rule {@link withGraphQuestion} follows, for the same reason.
 */
export function withoutGraphWalk(url: URL): string {
	const next = new URL(url);
	next.searchParams.delete('from');
	next.searchParams.delete('depth');
	next.searchParams.delete('sel');
	return `${next.pathname}${next.search}`;
}

/**
 * The analysis door for one place — the machine's measurements of it, declared as such.
 *
 * **Exactly one anchor, and that is the design rather than a simplification.** Measured on the
 * deployed substrate `[2026-08-20]`, the same quantity spans wildly different ranges per place:
 * `centrality` maxes at 2342.2 on the self-cognition map and 276 on `@me/temper`, `salience` at
 * 497.65 against 69.54. One ranked list over two places would be arithmetic on incommensurable
 * quantities, and the resulting order would look exactly as authoritative as a real one.
 *
 * It reuses `in=`'s grammar rather than inventing a second, so a place is addressed identically on
 * both doors and {@link parseGraphAddress} reads this URL too.
 */
export function graphAnalysisHref(ownerRef: string, anchor: GraphAnchorRef | null): string {
	if (!anchor) return `/graph/${ownerRef}/analysis`;
	const params = new URLSearchParams();
	params.set('in', `${anchor.kind === 'cogmap' ? 'map' : 'ctx'}:${anchor.ref}`);
	return `/graph/${ownerRef}/analysis?${params.toString()}`;
}
