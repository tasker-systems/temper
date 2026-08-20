import type { CogmapRow } from '$lib/types/generated/cognitive_maps';
import type { ContextRowWithCounts } from '$lib/types/generated/context';
import type { GraphAnchorRef } from '$lib/vault-url';
import type { Anchor } from './composition';

/**
 * Which places the reader is asking about, and what question the surface asks on their behalf.
 *
 * Everything here is pure: it takes the two list reads and the parsed URL and decides. The route
 * does the I/O. That split is what lets the entries of spec §2 be tested without a browser or a
 * server, which is the same reason Beat A's builder is pure.
 *
 * @see internal/superpowers/specs/2026-08-20-graph-successor-surface-design.md §1, §2
 */

/**
 * Every anchor the reader can read, as the builder's `Anchor`.
 *
 * **A cogmap's `ref` is its uuid**, which is not a cosmetic choice: it is the grammar
 * `GraphAnchorRef` publishes (*"the uuid for a cogmap"*), so this `ref` and the one in the URL are
 * the same string, and matching a named anchor to a readable one is an equality rather than a
 * resolution step that could disagree with the address bar.
 *
 * `resource_count` arrives as a JSON number even though the generated type says `bigint` — ts-rs
 * maps Rust's `u64` that way and `res.json()` has no bigint to give it. `Number()` is correct for
 * both and is not a cast away from a real bigint.
 */
export function readableAnchors(src: {
	contexts: ContextRowWithCounts[];
	cogmaps: CogmapRow[];
}): Anchor[] {
	return [
		...src.contexts.map(
			(c): Anchor => ({
				kind: 'context',
				id: String(c.id),
				ref: `${c.owner_ref}/${c.slug}`,
				resourceCount: Number(c.resource_count),
			}),
		),
		...src.cogmaps.map(
			(m): Anchor => ({
				kind: 'cogmap',
				id: m.id,
				ref: m.id,
				resourceCount: Number(m.resource_count),
			}),
		),
	];
}

export type AnchorResolution =
	/** No `in` at all: every readable anchor, which the ceiling then bounds and the line declares. */
	| { entry: 'unaddressed'; anchors: Anchor[]; available: number }
	/** At least one named anchor resolved. `available` is what the reader NAMED, not what resolved. */
	| { entry: 'named'; anchors: Anchor[]; available: number }
	/** Named places, none of them readable. A refusal — never a widened answer. */
	| { entry: 'none-resolved'; named: number };

/**
 * Match the URL's anchors against what the reader can read.
 *
 * **Nothing resolving is a refusal, not a widening**, and that is the whole reason this returns a
 * third case instead of an empty array. An empty anchor list with a question in hand makes the
 * builder emit `find-about-anywhere` — *search everything I can see* — so a link naming one
 * deleted context would silently answer across the entire corpus, and the bound line would say
 * `12 of 12 places` truthfully while answering a question nobody asked. That is the exact shape the
 * `?context=` nav flip had to be moved into this beat to avoid, one layer further in.
 */
export function resolveAnchors(readable: Anchor[], named: GraphAnchorRef[]): AnchorResolution {
	if (named.length === 0) {
		return { entry: 'unaddressed', anchors: readable, available: readable.length };
	}
	const byRef = new Map(readable.map((a) => [`${a.kind}:${a.ref}`, a]));
	const anchors = named.flatMap((n) => {
		const hit = byRef.get(`${n.kind}:${n.ref}`);
		return hit ? [hit] : [];
	});
	return anchors.length === 0
		? { entry: 'none-resolved', named: named.length }
		: { entry: 'named', anchors, available: named.length };
}

/** What the surface is asking, and whether it borrowed the question from a map's charter. */
export interface Question {
	text: string | null;
	/** The map whose charter supplied the question, so the surface can say so and link to it. */
	borrowedFrom: { id: string; name: string; telosResourceId: string } | null;
}

/**
 * The question, which the reader may not have supplied.
 *
 * §2.2 — **one cogmap, no question** — reads the map's telos and surveys under it: *"surveying under
 * this map's charter"*. The charter is already in hand: `CogmapRow.charter_statement` is *"the
 * charter's statement-of-purpose (block-0 of the telos)"*, carried on the same list row that named
 * the anchor. So this costs **no read at all**, where the spec assumed one — the list row is the
 * incumbent and reaching for `/api/resources/{telos_resource_id}` would be a second source for a
 * string already on the first.
 *
 * A charter can be absent (*"`None` when the charter has no authored statement yet"*), and then this
 * returns no question rather than an empty one. That is not a degraded §2.2: it is §2.3 for a map —
 * a place with no question shows everything in it — and the builder already answers that shape for
 * either anchor kind.
 */
export function questionFor(
	asked: string | null,
	anchors: Anchor[],
	cogmaps: CogmapRow[],
): Question {
	if (asked) return { text: asked, borrowedFrom: null };
	if (anchors.length !== 1 || anchors[0].kind !== 'cogmap')
		return { text: null, borrowedFrom: null };

	const map = cogmaps.find((m) => m.id === anchors[0].id);
	const charter = map?.charter_statement?.trim();
	if (!map || !charter) return { text: null, borrowedFrom: null };

	return {
		text: charter,
		borrowedFrom: { id: map.id, name: map.name, telosResourceId: map.telos_resource_id },
	};
}

/** A place, as a surface names it to a reader. */
export interface NamedPlace {
	kind: 'context' | 'cogmap';
	/** The same string the URL grammar carries, so a link is built without a second resolution. */
	ref: string;
	title: string;
}

/**
 * Name a place in words a reader recognises.
 *
 * A context's `ref` is already `@owner/slug` and is what the reader calls it. **A cogmap's `ref` is
 * its uuid** — the grammar `GraphAnchorRef` publishes — so showing the ref would put a uuid on
 * screen where a name belongs. The name is already in hand on the list row that produced the
 * anchor, so this costs no read.
 *
 * A map that is not on the list falls back to its ref rather than inventing a name: an anchor the
 * reader can address but whose row is not here is a state worth showing honestly, not papering over.
 */
export function describeAnchor(anchor: Anchor, cogmaps: CogmapRow[]): NamedPlace {
	if (anchor.kind === 'context') {
		return { kind: 'context', ref: anchor.ref, title: anchor.ref };
	}
	const map = cogmaps.find((m) => m.id === anchor.id);
	return { kind: 'cogmap', ref: anchor.ref, title: map?.name ?? anchor.ref };
}
