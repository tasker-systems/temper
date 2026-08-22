// graph-synthetic-vocabulary.mjs — the word bank the graph fixture sanitizer builds every
// synthetic title, region label, handle, slug and excerpt from.
//
// It lives in its own module, rather than inside `sanitize-graph-fixtures.mjs`, so that the
// fixture guard can import it without dragging the sanitizer into the app's `checkJs`
// type-checking (and without importing a module whose job is to read and write files). Two
// consumers, one list:
//
//   • the sanitizer, which draws from it
//   • `fixtures.test.ts`, which asserts the committed bundles' free text contains NOTHING
//     else — the POSITIVE leak guard. A denylist of known-real strings cannot work here,
//     because you cannot enumerate real prose; only a whitelist can fail on the value
//     nobody thought to list.
//
// Neutral, concrete, personal-data-free, and length-varied so replacements can track the
// originals' rendered width.

/** @type {readonly string[]} */
export const WORDS = [
	'meadow', 'signal', 'harbor', 'lattice', 'ember', 'orbit', 'cedar', 'ripple',
	'quartz', 'marble', 'thicket', 'summit', 'delta', 'beacon', 'willow', 'cobalt',
	'prairie', 'anchor', 'lucid', 'verdant', 'cascade', 'atlas', 'meridian', 'kestrel',
	'alder', 'basalt', 'current', 'drift', 'estuary', 'fathom', 'granite', 'hollow',
	'inlet', 'juniper', 'kelp', 'lantern', 'mistral', 'nimbus', 'onyx', 'plateau'
];
