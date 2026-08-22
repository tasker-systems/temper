// bounded.ts
/**
 * The refusal: a read the system may stop waiting for.
 *
 * A read that *fails* is distinguishable from one that is working — the region renders the
 * failure. A read that **never answers** is stopped without failing, and presents as arriving
 * forever, which is the exact failure mode of `working-and-stopped-are-distinguishable`. A bound
 * closes that hole: past it the wait is converted into a named `GaveUp`, so the region can say
 * *which* read the system declined to keep waiting for.
 *
 * **The duration is settled; the clause is not about a number.** `[ruled — 2026-08-21, Pete]`
 * *"we have to bound it somewhere."* The register still specifies no duration and deliberately
 * never will — a budget is a build decision, not an invariant — so this is a ruling on the
 * mechanism rather than an amendment to any clause. `ms` stays a parameter: a call site with a
 * reason may name its own.
 */

/**
 * How long a streamed read is waited for before the system declines to keep waiting.
 *
 * `[ruled — 2026-08-21, Pete]` **8 seconds**, on the reasoning that *"we have to bound it
 * somewhere"* — the alternative to a chosen number is not "no number", it is an unbounded wait
 * that presents as arriving forever, which is the exact failure this exists to close.
 *
 * Chosen so the bound fires while a reader is still watching the region rather than after they
 * have concluded the page is broken, and well inside the serverless function's own lifetime so
 * the give-up is *ours* to render rather than the platform's to truncate. **No measurement backs
 * it** — nothing has been instrumented, which is Phase 2's subject. A number that turns out wrong
 * is changed here, in one place, without touching a clause.
 */
export const GIVE_UP_AFTER_MS = 8_000;

/** A wait the system ended on purpose. Carries the region's label so the failure can be named. */
export class GaveUp extends Error {
	label: string;

	constructor(label: string, ms: number) {
		super(`gave up waiting for ${label} after ${ms}ms`);
		this.name = 'GaveUp';
		this.label = label;
	}
}

/**
 * Resolve as `p` does, or reject with `GaveUp` if `ms` elapses first.
 *
 * A real failure surfaces as itself — `bounded` never converts one rejection into another.
 *
 * **The catch goes on what this function HANDS OUT, not on what it is given** (spec §5.3's other
 * catch — the one that keeps an unhandled rejection from crashing the server, never the `{:catch}`
 * that renders the failure).
 *
 * `[corrected — 2026-08-21]` It used to go on `p`, and that was the same trap spec §5.3 warns
 * about, one level down: **having a catch on the input does not give you one on the output.**
 * `Promise.race([p, bound])` subscribes to `p`, so `p` is safe either way and no test could witness
 * a catch on it — but the promise this function *returns* is a further derivation
 * (`.race(...).finally(...)`) that **nothing inside here subscribes to**, and that is the one a load
 * hands to `{#await}`. Verified by executing a real `unhandledRejection` listener against an
 * unconsumed return value: with the catch on `p` it fires; with the catch here it does not, and a
 * consumer's own `{:catch}` still sees the rejection unchanged.
 *
 * Centralised here rather than left as a duty at each call site, deliberately. A rule every caller
 * must remember is a rule the next caller forgets — the same argument `GraphRead`'s subtraction
 * type makes on the graph load. A promise that does **not** go through `bounded` still needs its
 * own `.catch()` at creation.
 */
export function bounded<T>(
	p: Promise<T>,
	label: string,
	ms: number = GIVE_UP_AFTER_MS,
): Promise<T> {
	let timer: ReturnType<typeof setTimeout>;
	const bound = new Promise<never>((_, reject) => {
		timer = setTimeout(() => reject(new GaveUp(label, ms)), ms);
	});

	// `finally` and not `then`: the timer is cleared however the race ends, so a read that answers
	// first does not leave a pending timer holding the process open.
	const raced = Promise.race([p, bound]).finally(() => clearTimeout(timer));

	// Attached to `raced` — the value returned below — and not to `p`. See the note above.
	// `.catch()` returns a NEW promise and does not consume this one, so every consumer still
	// observes the rejection; this only marks it handled.
	raced.catch(() => {});

	return raced;
}

/**
 * A promise derived from another, with spec §5.3's **other** catch attached at creation.
 *
 * Every `.then()` produces a NEW promise, and {@link bounded}'s own guard sits on the promise
 * `bounded` handed out — one link upstream of this one. `[found by Task 4 — 2026-08-21]` that is
 * exactly the trap: a catch on the input does not give you one on the output, and the output is what
 * a load hands to `{#await}`, which on the server stays unsubscribed until SvelteKit serializes it.
 * So a promise that does not itself come through `bounded` still needs its own.
 *
 * `.catch()` returns a new promise and consumes nothing, so every consumer still observes the
 * rejection; this only marks it handled.
 *
 * It lives here beside `bounded` rather than in a load, for the reason that function's own note
 * gives: **a rule every caller must remember is a rule the next caller forgets**, and two copies of
 * a safety invariant drift. Both streaming loads derive through this one.
 */
export function derive<T, U>(p: Promise<T>, f: (v: T) => U | Promise<U>): Promise<U> {
	const next = p.then(f);
	next.catch(() => {});
	return next;
}
