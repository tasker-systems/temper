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
export const SLOTS = [
    'cat-1',
    'cat-2',
    'cat-3',
    'cat-4',
    'cat-5',
    'cat-6',
    'cat-7',
    'cat-8',
];
/** Core's default binding, grouped by what the doc types are for. */
export const CORE_BINDINGS = [
    { doctype: 'task', tint: 'cat-2' },
    // intended ends
    { doctype: 'goal', tint: 'cat-5' },
    { doctype: 'commitment', tint: 'cat-5' },
    // what happened, and what was kept
    { doctype: 'session', tint: 'cat-3' },
    { doctype: 'memory', tint: 'cat-3' },
    // inquiry
    { doctype: 'research', tint: 'cat-7' },
    { doctype: 'question', tint: 'cat-7' },
    // rulings and standing rules
    { doctype: 'decision', tint: 'cat-4' },
    { doctype: 'principle', tint: 'cat-4' },
    // ideas and territories
    { doctype: 'concept', tint: 'cat-6' },
    { doctype: 'domain', tint: 'cat-6' },
    { doctype: 'theme', tint: 'cat-6' },
    { doctype: 'fact', tint: 'cat-8' },
    { doctype: 'concern', tint: 'cat-1' },
];
function isSlot(value) {
    return SLOTS.includes(value);
}
/**
 * A vocabulary's bindings, checked: each doc type named and bound once, each tint a slot. The
 * result stands alone — `tintFor` under it does not fall back to core's binding, so a surface
 * that honours several vocabularies merges them before resolving.
 */
export function bindingsFrom(entries) {
    const out = new Map();
    for (const { doctype, tint } of entries) {
        if (typeof doctype !== 'string' || !doctype.trim()) {
            throw new Error(`a binding names no doc type: ${JSON.stringify(doctype)}`);
        }
        if (!isSlot(tint)) {
            throw new Error(`${doctype} is bound to ${String(tint)}, which is not a categorical slot`);
        }
        if (out.has(doctype))
            throw new Error(`${doctype} is bound twice`);
        out.set(doctype, tint);
    }
    return out;
}
export const CORE = bindingsFrom(CORE_BINDINGS);
/** The tint a doc type paints with under a vocabulary's bindings (core's by default). */
export function tintFor(docType, bindings = CORE) {
    return (docType && bindings.get(docType)) || 'cat-neutral';
}
/** The role a tint paints with, as a `var()` — for a style, never an SVG presentation attribute. */
export function tintVar(tint) {
    return `var(--tp-${tint})`;
}
