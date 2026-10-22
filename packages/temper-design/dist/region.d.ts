export type RegionStateName = 'arriving' | 'empty' | 'gave-up' | 'failed';
/** Every state, in the order a region can pass through them. */
export declare const REGION_STATES: readonly RegionStateName[];
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
export declare function regionWords(state: RegionStateName, label: string): RegionWords;
