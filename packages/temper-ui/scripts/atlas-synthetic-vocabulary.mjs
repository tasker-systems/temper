// atlas-synthetic-vocabulary.mjs — the word bank the Atlas fixture sanitizer builds every
// synthetic title, label, handle and excerpt from.
//
// It lives in its own module, rather than in `sanitize-atlas-fixtures.mjs`, so that
// `fixtures.test.ts` can import it without dragging the sanitizer into the app's
// `checkJs` type-checking (and without importing a module whose job is to read and write
// files). Two consumers, one list:
//
//   • the sanitizer, which draws from it
//   • `fixtures.test.ts`, which asserts the committed bundle's free text contains NOTHING
//     else — the positive leak guard that catches an un-sanitized value a denylist of
//     known-real strings cannot, since you cannot enumerate real prose
//
// Neutral, concrete, personal-data-free, and length-varied so replacements can track the
// originals' rendered width.

/** @type {readonly string[]} */
export const WORDS = [
	'meadow', 'signal', 'harbor', 'lattice', 'ember', 'orbit', 'cedar', 'ripple',
	'quartz', 'marble', 'thicket', 'summit', 'delta', 'beacon', 'willow', 'cobalt',
	'prairie', 'anchor', 'lucid', 'verdant', 'cascade', 'atlas', 'meridian', 'kestrel'
];
