/**
 * Bound vocabularies: how a doc type reaches colour. The contract owns eight categorical slots and
 * a neutral; a vocabulary binds its doc types to slots, never to a colour or a chrome or state
 * role. Core's default binding lives here, so every surface tints core's doc types alike; a
 * plugin's binding is its own and goes through `bindingsFrom`, which refuses anything but a slot.
 * A doc type bound nowhere paints in the neutral.
 *
 * Hue never carries identity alone: where a slot marks a doc type, the doc type's word is shown
 * beside it, and doc types that share a slot are told apart by that word.
 */
export declare const SLOTS: readonly ["cat-1", "cat-2", "cat-3", "cat-4", "cat-5", "cat-6", "cat-7", "cat-8"];
export type Slot = (typeof SLOTS)[number];
/** What a doc type's mark paints with: its bound slot, or the neutral. */
export type Tint = Slot | 'cat-neutral';
export interface DocTypeBinding {
    doctype: string;
    tint: Slot;
}
export type Bindings = ReadonlyMap<string, Slot>;
/** Core's default binding, grouped by what the doc types are for. */
export declare const CORE_BINDINGS: readonly DocTypeBinding[];
/** A vocabulary's bindings, checked: each tint a slot, each doc type bound once. */
export declare function bindingsFrom(entries: readonly {
    doctype: string;
    tint: unknown;
}[]): Bindings;
export declare const CORE: Bindings;
/** The tint a doc type paints with under a vocabulary's bindings (core's by default). */
export declare function tintFor(docType: string | null | undefined, bindings?: Bindings): Tint;
/** The role a tint paints with, as a `var()` — for a style, never an SVG presentation attribute. */
export declare function tintVar(tint: Tint): `var(--tp-${Tint})`;
