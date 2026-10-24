import { type SimulationNodeDatum } from 'd3-force';
export type GraphLayout = 'force' | 'radial';
export interface LayoutInput {
    id: string;
    /** In a radial layout, a core node settles at the centre and the rest ring it. */
    core?: boolean;
}
export interface Placed extends SimulationNodeDatum {
    id: string;
    /** Distinct neighbours among the placed nodes. */
    degree: number;
    x: number;
    y: number;
}
/**
 * Every edge between one pair of nodes, drawn as one line. Edges are kept in the order they came,
 * oriented from `source` to `target`: an edge that runs the other way is `reversed`.
 */
export interface Joined<E> {
    edges: {
        edge: E;
        reversed: boolean;
    }[];
    source: Placed;
    target: Placed;
}
/** The space the simulation runs in. A surface sizes it to its canvas. */
export interface Plane {
    width: number;
    height: number;
}
export declare const DEFAULT_PLANE: Plane;
/** A mark's radius: more connected, a little larger, and never past a bound. */
export declare const nodeRadius: (degree: number) => number;
/**
 * How many other carried nodes each node is joined to. Neighbours, not edges: two relations
 * between the same pair are one line, and a node joined only to itself has none.
 */
export declare function degrees(nodes: {
    id: string;
}[], edges: {
    source: string;
    target: string;
}[]): Map<string, number>;
export interface Settled<E> {
    nodes: Placed[];
    edges: Joined<E>[];
    selfJoined: E[];
    unconnected: string[];
    /** Whether the radial force ran: asked for, and the placed nodes held both core and ring. */
    radial: boolean;
}
/** Settles the connected nodes; see the module header for the policy. */
export declare function settle<E extends {
    source: string;
    target: string;
}>(input: LayoutInput[], edges: E[], layout: GraphLayout, plane?: Plane): Settled<E>;
export interface Box {
    x1: number;
    y1: number;
    x2: number;
    y2: number;
}
/** Truncate to `max` characters (code points, so no character is split) with a trailing ellipsis. */
export declare const truncate: (text: string, max: number) => string;
export interface Caption {
    id: string;
    x: number;
    y: number;
    text: string;
}
export interface CaptionOptions {
    /** How many captions to try to draw at all, most connected first. A surface's room. */
    max: number;
    /** Approximate advance width of one character at the render font size. */
    charWidth: number;
    lineHeight: number;
    /** Longest caption, in characters, before the ellipsis. */
    cap: number;
    /** Clearance around each reserved box. */
    padding: number;
}
export declare const CAPTION: CaptionOptions;
/**
 * Which captions can be drawn without landing on another caption or on another node's mark.
 * Greedy, most connected first, ties broken by code-unit order on id. A caption that would collide
 * is dropped, not nudged: a label moved off its node reads as belonging to whatever it landed
 * near. Every node is still drawn and still named elsewhere (a tooltip, a hover card, a list).
 */
export declare function placeCaptions(nodes: {
    id: string;
    x: number;
    y: number;
    degree: number;
    label: string;
}[], options?: Partial<CaptionOptions>): Caption[];
/**
 * The drawing's frame: every mark and caption, with a margin. A surface that fits its viewBox to
 * the drawing reads it; one with a fixed canvas does not need it. Captions are measured with the
 * same options `placeCaptions` was given, so the frame fits what was placed.
 */
export declare function frame(nodes: Placed[], captions: Caption[], { margin, plane, caption, }?: {
    margin?: number;
    plane?: Plane;
    caption?: Partial<CaptionOptions>;
}): Box;
/** How far from its centre a line ends at a mark: just past the mark's own rim. */
export declare const rimOf: (degree: number) => number;
/**
 * A pair's line, trimmed to the rims of its two marks, and an arrowhead at each end an edge
 * points to. Heads are shapes, not SVG markers, so a drawing holds no ids and any number of
 * graphs share a page. A line run centre to centre beneath its marks would end every head under
 * the node it points at. `rims` overrides where each end stops, for a mark that is not a circle.
 */
export declare function edgeGeometry(source: Placed, target: Placed, ends: {
    atSource: boolean;
    atTarget: boolean;
}, rims?: {
    source: number;
    target: number;
}): {
    x1: number;
    y1: number;
    x2: number;
    y2: number;
    heads: string[];
};
export type EdgeKind = 'contains' | 'leads_to' | 'express' | 'near';
/**
 * What the grammar reads off an edge: the relation's label, kind, polarity and weight. Every
 * field is optional, because a read may not carry it; each absence draws a stated default.
 */
export interface Answered {
    /** The relation, as the corpus names it. */
    label?: string | null;
    edgeKind?: EdgeKind;
    polarity?: 'forward' | 'inverse';
    /**
     * The relation's strength, drawn as the line's width. Absent draws the unweighted width —
     * never a default of 1, which is a real weight.
     */
    weight?: number | null;
}
export type StrokeRole = 'structural' | 'derived' | 'contradicts';
/**
 * Stroke for an edge whose weight is absent. A width cannot say "not reported" by itself, so a
 * surface names the weight, or its absence, where it names the relation.
 */
export declare const UNWEIGHTED_WIDTH = 1.4;
/**
 * Weight → width over the range weights take: asserted at 1.0 by default and only lowered by
 * decay, so (0, 1] spans 1 to 2.5. A weight outside it is held to the bounds, so a stray value
 * cannot draw a line that dominates the drawing.
 */
export declare const widthOf: (weight: number) => number;
/** How one edge draws: its role (painted by `contract/graph.css`), dash, width, and heads. */
export interface Stroke {
    role: StrokeRole;
    dash: string | null;
    width: number;
    atSource: boolean;
    atTarget: boolean;
    /**
     * A cross at the midpoint: contradicts' channel beyond colour. Its role sits too close to the
     * structural one in luminance for a colour-blind reader, and dash (kind) and width (weight) are
     * taken, so it carries a shape of its own.
     */
    cross: boolean;
}
export declare function stroke(edge: Answered): Stroke;
/** Where a pair's arrowheads go: at each end some edge between them points to. */
export declare function pairEnds<E extends Answered>(edges: {
    edge: E;
    reversed: boolean;
}[]): {
    atSource: boolean;
    atTarget: boolean;
};
/**
 * How a pair's line is drawn when its edges differ: a contradiction is never hidden behind a
 * plainer relation, a line dashes only when every edge on it dashes — with the dash the first
 * dashing edge names — and it is never thinner than the heaviest relation on it.
 */
export declare function pairStroke<E extends Answered>(edges: {
    edge: E;
}[]): {
    role: StrokeRole;
    dash: string | null;
    width: number;
    cross: boolean;
};
