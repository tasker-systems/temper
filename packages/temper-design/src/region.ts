// region.ts — the region vocabulary: the one place arriving, empty, gave-up and failed are given
// their words, their marker and their ARIA role, on every surface.
//
// `region-states-read-alike`: the four states are distinct signals, carried on more than colour,
// and the same signal on both surfaces. Each surface renders a state through its own thin
// component; what the component says comes from here, and how it looks comes from the contract's
// `region.css` recipe on the `--tp-region-*` roles. Neither is spelled per surface.
//
// **The states differ by more than one channel**, deliberately: a test asserting two renders
// differ is satisfied by a single character, while the clause is about what a reader can resolve.
// So each state carries its own sentence, its own marker glyph, its own colour and its own border
// treatment (the recipe's). Removing any one still leaves them resolvable.
//
// Three rules the words obey:
//
// - **A pending region carries text**, never a bare shimmer, so it reaches the accessibility tree
//   rather than being a silent animation.
// - **A failed region names what failed** — "history unavailable", never "something went wrong" —
//   and says that nothing was read, which is exactly what separates it from `empty`. `empty`
//   asserts *there is nothing here*; a failed read has verified no such thing.
// - **A give-up names what the system stopped waiting for**, never a bare "timeout", which
//   describes the mechanism and names nothing the reader was waiting for.
//
// `label` names the region ("history", "excerpt", "connections") and is composed into all four
// sentences, so a consumer cannot word its arrival, its refusal and its failure out of step.
//
// **ARIA role is the one channel that cannot be four-way distinct**: there are two live-region
// roles. `arriving` and `gave-up` share `status` because neither is a fault — one is not finished,
// the other is a decision the system made and owns. `failed` is `alert`. `empty` is a settled fact
// and announces nothing. The other channels carry the distinction between the two that share.
//
// **The fifth state, present, is deliberately not here.** A region that has its content renders
// its own markup; it needs no vocabulary to describe it.

export type RegionStateName = 'arriving' | 'empty' | 'gave-up' | 'failed';

/** Every state, in the order a region can pass through them. */
export const REGION_STATES: readonly RegionStateName[] = ['arriving', 'empty', 'gave-up', 'failed'];

/** What a component renders for one state. */
export interface RegionWords {
	/** The glyph that opens the line; decorative, so a component hides it from assistive tech. */
	marker: string;
	/** The live-region role, or `null` for a state that announces nothing. */
	role: 'status' | 'alert' | null;
	/** The sentence, composed from the region's label. */
	words: string;
	/** The state's own qualifier after the sentence, set apart by the recipe; `null` if none. */
	qualifier: string | null;
}

/** The words, marker and role for `state` of the region named `label`. */
export function regionWords(state: RegionStateName, label: string): RegionWords {
	switch (state) {
		case 'arriving':
			return { marker: '◌', role: 'status', words: `Loading ${label}…`, qualifier: null };
		case 'empty':
			return { marker: '—', role: null, words: `No ${label}.`, qualifier: null };
		case 'gave-up':
			return {
				marker: '⊘',
				role: 'status',
				words: `We stopped waiting for ${label}`,
				qualifier: ' — it did not answer in time.',
			};
		case 'failed':
			// Only the failure opens its sentence with the label, so only it needs the capital.
			return {
				marker: '⚠',
				role: 'alert',
				words: `${label.charAt(0).toUpperCase()}${label.slice(1)} unavailable`,
				qualifier: ' — nothing here was read.',
			};
	}
}
