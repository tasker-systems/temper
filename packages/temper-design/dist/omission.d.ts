/**
 * The omission sentence every bounded view ends with, composed from its numbers so no author words
 * it out of step with them. A view that is a page of something larger says which rows it shows and
 * what lies either side: `more` is the answer's own word that rows follow, never a guess from how
 * many rows arrived.
 *
 * A graph view composes its omissions in the read's own terms: a walk says how deep it went and
 * that deeper was not reported — there is no denominator to count, and none is invented; an entry
 * read says drawn of eligible and how many in-scope nodes it did not draw for having no
 * connections; a cut to the drawing's bounds is counted, never silent.
 */
export type Page = {
    offset: number;
    size: number;
    more: boolean;
};
/** Where a graph view was read from, as its read answers it. */
export type GraphArm = {
    read: 'walk';
    from: string[];
    depth: number;
} | {
    read: 'entry';
    in: string[];
    k: number;
};
/** What an entry read says it drew. */
export type GraphBounds = {
    drawn: number;
    eligible: number;
    inScope: number;
    truncated: boolean;
};
/** What the drawing's bounds clipped off the answer, each part present only when some were. */
export type GraphCut = {
    nodes?: number;
    edges?: number;
};
/** The read behind a graph view, as its omission clauses are composed from it. */
export type GraphOmissions = {
    arm: GraphArm | null;
    bounds: GraphBounds | null;
    cut: GraphCut | null;
};
export declare function omissionSentence(total: number, shown: number, scope: string, page?: Page | null): string;
/** The clause naming where a graph figure was read from, for its description. */
export declare function graphStanding(arm: GraphArm, nameOf: (id: string) => string): string;
/** The omission sentence for a graph view, composed from the read's own numbers. */
export declare function graphSentence(total: number, shown: number, scope: string, { arm, bounds, cut }: GraphOmissions): string;
