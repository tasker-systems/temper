// graph.ts — the graph's geometry and its edge grammar: where nodes settle, which captions can be
// drawn, how a line ends at its marks, and how a relation is stroked. One policy on every surface;
// a surface supplies only its plane (the space the simulation runs in) and how many captions it
// has room for.
//
// Nothing here knows a doc type, a home or a colour. A node is an id and whether it holds the
// core; an edge is two ids and what the answer holds of its relation. A stroke names a ROLE
// (`structural`, `derived`, `contradicts`), which the contract's `graph.css` recipe paints.
//
// The policy, each rule for a reason a reader would otherwise pay for:
//
// - **Deterministic.** The simulation starts from a ring in arrival order and runs a fixed number
//   of ticks; captions are ranked by degree, ties broken by code-unit order, never by
//   `localeCompare`, which differs between machines. A view the reader returns to draws the same
//   picture, wherever it is drawn.
// - **What is unconnected is not placed.** A position claims a relation, so a node with no edge
//   among those carried is returned in `unconnected` for the surface to list beneath the drawing.
// - **One line per pair.** Every relation between two nodes is joined onto one line, which
//   carries all of them; lines drawn per relation overlay exactly and hide all but the top one.
//   An edge joining a node to itself has no line, and is returned in `selfJoined` to be named.
// - **Degree counts neighbours**, the lines drawn, not relations. It sizes the mark and its
//   collision radius. A surface that says how many relations a node has keeps its own count.
// - **Radial only on a contrast.** A radial layout settles the core at the centre and rings the
//   rest round it, and runs only when the input holds both. Asked of an input that is all core or
//   all ring it settles as force: pulling every node toward one radius packs the drawing into a
//   disc and tells the reader nothing.
import { forceCenter, forceCollide, forceLink, forceManyBody, forceRadial, forceSimulation, forceX, forceY, } from 'd3-force';
export const DEFAULT_PLANE = { width: 640, height: 400 };
const TICKS = 300;
/** A mark's radius: more connected, a little larger, and never past a bound. */
export const nodeRadius = (degree) => 7 + Math.min(9, degree * 0.6);
/**
 * How many other carried nodes each node is joined to. Neighbours, not edges: two relations
 * between the same pair are one line, and a node joined only to itself has none.
 */
export function degrees(nodes, edges) {
    const neighbours = new Map(nodes.map((n) => [n.id, new Set()]));
    for (const e of edges) {
        if (e.source === e.target)
            continue;
        neighbours.get(e.source)?.add(e.target);
        neighbours.get(e.target)?.add(e.source);
    }
    return new Map([...neighbours].map(([id, set]) => [
        id,
        [...set].filter((other) => neighbours.has(other)).length,
    ]));
}
/** Settles the connected nodes; see the module header for the policy. */
export function settle(input, edges, layout, plane = DEFAULT_PLANE) {
    const { width: W, height: H } = plane;
    const degree = degrees(input, edges);
    const connected = input.filter((n) => (degree.get(n.id) ?? 0) > 0);
    const count = Math.max(1, connected.length);
    const nodes = connected.map((n, i) => ({
        id: n.id,
        degree: degree.get(n.id) ?? 0,
        x: W / 2 + Math.cos((i / count) * 2 * Math.PI) * 120,
        y: H / 2 + Math.sin((i / count) * 2 * Math.PI) * 120,
    }));
    const byId = new Map(nodes.map((n) => [n.id, n]));
    const pairs = new Map();
    for (const edge of edges) {
        const source = byId.get(edge.source);
        const target = byId.get(edge.target);
        if (!source || !target || source === target)
            continue;
        const key = JSON.stringify([edge.source, edge.target].sort());
        const pair = pairs.get(key);
        if (pair)
            pair.edges.push({ edge, reversed: pair.source !== source });
        else
            pairs.set(key, { edges: [{ edge, reversed: false }], source, target });
    }
    const joined = [...pairs.values()];
    const core = new Set(connected.filter((n) => n.core).map((n) => n.id));
    const radial = layout === 'radial' && core.size > 0 && core.size < connected.length;
    // A link between the core and the ring runs looser and weaker, so the radial force can hold the
    // two apart; links within either keep their structure.
    const crosses = (j) => radial && core.has(j.source.id) !== core.has(j.target.id);
    const sim = forceSimulation(nodes)
        .force('link', forceLink(joined.map((j) => ({ source: j.source, target: j.target })))
        .distance((_l, i) => (crosses(joined[i]) ? 150 : 80))
        .strength((_l, i) => (crosses(joined[i]) ? 0.15 : 0.6)))
        .force('charge', forceManyBody().strength(-260).distanceMax(240))
        .force('center', forceCenter(W / 2, H / 2))
        // A weak pull to the middle keeps separate clusters near one another, so a drawing fitted to
        // where nodes land does not shrink every mark, and a fixed canvas does not lose a cluster.
        .force('x', forceX(W / 2).strength(0.08))
        .force('y', forceY(H / 2).strength(0.08))
        .force('collide', forceCollide().radius((n) => nodeRadius(n.degree) + 6))
        .stop();
    if (radial) {
        const minDim = Math.min(W, H);
        sim.force('radial', forceRadial((n) => (core.has(n.id) ? minDim * 0.06 : minDim * 0.44), W / 2, H / 2).strength(0.6));
    }
    for (let i = 0; i < TICKS; i++)
        sim.tick();
    return {
        nodes,
        edges: joined,
        selfJoined: edges.filter((e) => e.source === e.target && degree.has(e.source)),
        unconnected: input.filter((n) => (degree.get(n.id) ?? 0) === 0).map((n) => n.id),
        radial,
    };
}
const overlaps = (a, b) => a.x1 < b.x2 && b.x1 < a.x2 && a.y1 < b.y2 && b.y1 < a.y2;
/** Truncate to `max` characters (code points, so no character is split) with a trailing ellipsis. */
export const truncate = (text, max) => [...text].length <= max ? text : `${[...text].slice(0, max - 1).join('')}…`;
/** Code-unit order: the same on every machine, unlike `localeCompare`. */
const byCodeUnit = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
export const CAPTION = {
    max: 24,
    charWidth: 5.4,
    lineHeight: 12,
    cap: 28,
    padding: 2,
};
/**
 * Which captions can be drawn without landing on another caption or on another node's mark.
 * Greedy, most connected first, ties broken by code-unit order on id. A caption that would collide
 * is dropped, not nudged: a label moved off its node reads as belonging to whatever it landed
 * near. Every node is still drawn and still named elsewhere (a tooltip, a hover card, a list).
 */
export function placeCaptions(nodes, options = {}) {
    const o = { ...CAPTION, ...options };
    const marks = new Map(nodes.map((n) => {
        const r = nodeRadius(n.degree) + o.padding;
        return [n.id, { x1: n.x - r, y1: n.y - r, x2: n.x + r, y2: n.y + r }];
    }));
    const ranked = [...nodes].sort((a, b) => b.degree - a.degree || byCodeUnit(a.id, b.id));
    const taken = [];
    const placed = [];
    for (const n of ranked) {
        if (placed.length >= o.max)
            break;
        const text = truncate(n.label, o.cap);
        const half = ([...text].length * o.charWidth) / 2;
        const baseline = n.y + nodeRadius(n.degree) + o.lineHeight;
        const box = {
            x1: n.x - half - o.padding,
            y1: baseline - o.lineHeight * 0.75,
            x2: n.x + half + o.padding,
            y2: baseline + o.padding,
        };
        const hitsAMark = [...marks].some(([id, b]) => id !== n.id && overlaps(box, b));
        if (hitsAMark || taken.some((b) => overlaps(box, b)))
            continue;
        taken.push(box);
        placed.push({ id: n.id, x: n.x, y: baseline, text });
    }
    return placed;
}
/**
 * The drawing's frame: every mark and caption, with a margin. A surface that fits its viewBox to
 * the drawing reads it; one with a fixed canvas does not need it. Captions are measured with the
 * same options `placeCaptions` was given, so the frame fits what was placed.
 */
export function frame(nodes, captions, { margin = 12, plane = DEFAULT_PLANE, caption = {}, } = {}) {
    const o = { ...CAPTION, ...caption };
    if (nodes.length === 0)
        return { x1: 0, y1: 0, x2: plane.width, y2: plane.height };
    let box = { x1: Infinity, y1: Infinity, x2: -Infinity, y2: -Infinity };
    const grow = (b) => {
        box = {
            x1: Math.min(box.x1, b.x1),
            y1: Math.min(box.y1, b.y1),
            x2: Math.max(box.x2, b.x2),
            y2: Math.max(box.y2, b.y2),
        };
    };
    for (const n of nodes) {
        const r = nodeRadius(n.degree);
        grow({ x1: n.x - r, y1: n.y - r, x2: n.x + r, y2: n.y + r });
    }
    for (const c of captions) {
        const half = ([...c.text].length * o.charWidth) / 2;
        grow({ x1: c.x - half, y1: c.y - o.lineHeight, x2: c.x + half, y2: c.y + 3 });
    }
    return { x1: box.x1 - margin, y1: box.y1 - margin, x2: box.x2 + margin, y2: box.y2 + margin };
}
/** How far from its centre a line ends at a mark: just past the mark's own rim. */
export const rimOf = (degree) => nodeRadius(degree) + 1.5;
/**
 * A pair's line, trimmed to the rims of its two marks, and an arrowhead at each end an edge
 * points to. Heads are shapes, not SVG markers, so a drawing holds no ids and any number of
 * graphs share a page. A line run centre to centre beneath its marks would end every head under
 * the node it points at. `rims` overrides where each end stops, for a mark that is not a circle.
 */
export function edgeGeometry(source, target, ends, rims = {
    source: rimOf(source.degree),
    target: rimOf(target.degree),
}) {
    const dx = target.x - source.x;
    const dy = target.y - source.y;
    const len = Math.hypot(dx, dy) || 1;
    const ux = dx / len;
    const uy = dy / len;
    // Marks closer than their two rims would trim the line past itself and turn a head backwards;
    // the trims shrink together until they meet, so the line collapses instead.
    const fit = Math.min(1, len / (rims.source + rims.target || 1));
    const x1 = source.x + ux * rims.source * fit;
    const y1 = source.y + uy * rims.source * fit;
    const x2 = target.x - ux * rims.target * fit;
    const y2 = target.y - uy * rims.target * fit;
    const head = (tipX, tipY, dirX, dirY) => {
        const size = 6;
        const bx = tipX - dirX * size;
        const by = tipY - dirY * size;
        const px = -dirY * size * 0.5;
        const py = dirX * size * 0.5;
        const f = (v) => v.toFixed(2);
        return `M${f(tipX)},${f(tipY)} L${f(bx + px)},${f(by + py)} L${f(bx - px)},${f(by - py)} Z`;
    };
    const heads = [
        ...(ends.atTarget ? [head(x2, y2, ux, uy)] : []),
        ...(ends.atSource ? [head(x1, y1, -ux, -uy)] : []),
    ];
    return { x1, y1, x2, y2, heads };
}
/** An edge with no `edgeKind` draws as the structural default: solid, headed by its polarity. */
const kindOf = (edge) => edge.edgeKind ?? 'contains';
const KIND_DASH = {
    contains: null,
    leads_to: '7 4',
    express: '1 4',
    near: '4 4',
};
/**
 * Stroke for an edge whose weight is absent. A width cannot say "not reported" by itself, so a
 * surface names the weight, or its absence, where it names the relation.
 */
export const UNWEIGHTED_WIDTH = 1.4;
/**
 * Weight → width over the range weights take: asserted at 1.0 by default and only lowered by
 * decay, so (0, 1] spans 1 to 2.5. A weight outside it is held to the bounds, so a stray value
 * cannot draw a line that dominates the drawing.
 */
export const widthOf = (weight) => 1 + 1.5 * Math.min(1, Math.max(0, weight));
export function stroke(edge) {
    const role = edge.label === 'contradicts'
        ? 'contradicts'
        : edge.label === 'derived_from'
            ? 'derived'
            : 'structural';
    // The label dashes first: a `derived_from` line dashes whatever its kind.
    const dash = edge.label === 'derived_from' ? '7 4' : KIND_DASH[kindOf(edge)];
    const width = edge.weight == null ? UNWEIGHTED_WIDTH : widthOf(edge.weight);
    // A `near` relation points both ways at once, so it heads neither end.
    const headed = kindOf(edge) !== 'near';
    const forward = (edge.polarity ?? 'forward') === 'forward';
    return {
        role,
        dash,
        width,
        atSource: headed && !forward,
        atTarget: headed && forward,
        cross: role === 'contradicts',
    };
}
/** Where a pair's arrowheads go: at each end some edge between them points to. */
export function pairEnds(edges) {
    const ends = { atSource: false, atTarget: false };
    for (const { edge, reversed } of edges) {
        const s = stroke(edge);
        if (!s.atSource && !s.atTarget)
            continue;
        if (s.atTarget !== reversed)
            ends.atTarget = true;
        else
            ends.atSource = true;
    }
    return ends;
}
/**
 * How a pair's line is drawn when its edges differ: a contradiction is never hidden behind a
 * plainer relation, a line dashes only when every edge on it dashes — with the dash the first
 * dashing edge names — and it is never thinner than the heaviest relation on it.
 */
export function pairStroke(edges) {
    const strokes = edges.map(({ edge }) => stroke(edge));
    const first = strokes[0];
    if (first === undefined)
        throw new Error('pairStroke: a pair carries at least one edge');
    const role = strokes.some((s) => s.role === 'contradicts')
        ? 'contradicts'
        : strokes.every((s) => s.role === 'derived')
            ? 'derived'
            : 'structural';
    const dash = strokes.every((s) => s.dash) ? first.dash : null;
    const width = Math.max(...strokes.map((s) => s.width));
    return { role, dash, width, cross: role === 'contradicts' };
}
