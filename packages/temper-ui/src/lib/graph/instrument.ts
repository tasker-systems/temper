/**
 * How much room the canvas is entitled to, and what gives way when it does not have it.
 *
 * `[ruled — 2026-08-22, Pete]` **Nothing in this layout declared what the canvas needs.**
 * `min-width: 0` appears three times in `GraphPage.svelte` — a legitimate grid idiom — and its
 * effect is that *the canvas is the only thing that ever yields, and it yields without limit*,
 * while the rail is a fixed `22rem` and the readout track a fixed `20rem`. So this was never
 * "two panels is one too many"; it is that the fixed things are fixed and the meaningful thing
 * is not.
 *
 * **Priority: the canvas yields last, the readout yields first.** The reader has just clicked a
 * specific node, so reasoning about the *whole answer* is the least urgent of the three at that
 * moment — and it is the thing still there when they close the rail. Merging the rail and the
 * readout into one slot was ruled OUT: they are different kinds, and `GraphPage.svelte`'s own
 * docstring is the argument — *"everything in the canvas is their own material, and everything a
 * machine decided is in the panel beside it."*
 */

/**
 * **The floor. Nothing measured it.**
 *
 * Same standing as `GIVE_UP_AFTER_MS`: a build decision with no instrument behind it, in **one
 * named constant**, so a number that turns out wrong is changed here and nowhere else. It is
 * deliberately not spread across breakpoints.
 *
 * Where it came from, stated so a later reader can argue with it: the ruling reports the canvas at
 * ~610px reading as *"a smudge"* for a 130-node force layout, and at ~770px without complaint.
 * This sits between them. That is a report of what a screen looked like, not a measurement.
 */
export const CANVAS_FLOOR_PX = 704;

/**
 * The rail's own width — `NodeRail.svelte`'s `.node-rail { width: 22rem }`, at a 16px root.
 *
 * Read here as a number because the decision is arithmetic and CSS cannot do it: a container query
 * condition may not read a custom property, so a rule expressed in CSS would have had to hard-code
 * the SUM, and the floor would then live in two places. Doing the arithmetic here is what keeps it
 * in one.
 *
 * The px conversion assumes the default root font size. A reader who has enlarged it gets a
 * threshold that fires slightly late, which is the safe direction: late means *nothing changes*.
 */
export const RAIL_PX = 352;

/** The readout track's width — `.instrument.with-panel`'s second column, `20rem` at a 16px root. */
export const READOUT_PX = 320;

/**
 * Whether *Why these* must give up its column so the canvas can stay above {@link CANVAS_FLOOR_PX}.
 *
 * Pure, and separate from the component, because it is a decision expressible as a value: the
 * component test can only witness which branch rendered, and jsdom computes no layout at all.
 *
 * Three things it deliberately does **not** do:
 *
 * - **It never yields when there is nothing else beside the canvas.** With no rail open the canvas
 *   already has everything but the readout, and the ruling's priority only orders three things
 *   competing for one row.
 * - **It never fires on an unmeasured surface.** `surfaceWidth` is `0` before the first observation
 *   and during SSR, and a zero would otherwise read as *infinitely narrow* and collapse the panel
 *   on the server for every reader.
 * - **It is not the reader's answer, only the layout's.** A reader who opens the strip anyway is
 *   overriding this, and the caller owns that state — a floor is not a lock.
 */
export function readoutMustYield(o: {
	/** The measured inline size of the surface the three share. */
	surfaceWidth: number;
	/** Is the node rail open? It lives inside the canvas's own track and takes width from it. */
	railOpen: boolean;
	/** Is there a readout (or a traversal's provenance) in the second track at all? */
	readoutPresent: boolean;
}): boolean {
	if (!o.railOpen || !o.readoutPresent) return false;
	if (o.surfaceWidth <= 0) return false;
	return o.surfaceWidth - RAIL_PX - READOUT_PX < CANVAS_FLOOR_PX;
}

/**
 * The width at which {@link readoutMustYield} starts saying yes — **derived, never written down.**
 *
 * Exported for the test that pins the arithmetic, and for anyone who wants the number without
 * recomputing it. Deriving it is the point: change {@link CANVAS_FLOOR_PX} and this moves with it.
 */
export const YIELD_BELOW_PX = CANVAS_FLOOR_PX + RAIL_PX + READOUT_PX;
