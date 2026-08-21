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
 * The `.catch()` attached to `p` is the *other* catch (spec §5.3), the one that keeps a rejection
 * arriving after the bound has fired from becoming an unhandled rejection and crashing the server.
 * It is not the `{:catch}` that renders the failure, and having one does not give you the other.
 * Stated plainly, because it is easy to bank wrongly: **`Promise.race` already subscribes to `p`,
 * so the losing side's late rejection is absorbed by the race whether this line is here or not** —
 * no test can witness its absence. It stays as the explicit statement of the invariant, so a
 * rewrite that stops racing does not silently take the guarantee with it.
 */
export function bounded<T>(
	p: Promise<T>,
	label: string,
	ms: number = GIVE_UP_AFTER_MS,
): Promise<T> {
	p.catch(() => {});

	let timer: ReturnType<typeof setTimeout>;
	const bound = new Promise<never>((_, reject) => {
		timer = setTimeout(() => reject(new GaveUp(label, ms)), ms);
	});

	// `finally` and not `then`: the timer is cleared however the race ends, so a read that answers
	// first does not leave a pending timer holding the process open.
	return Promise.race([p, bound]).finally(() => clearTimeout(timer));
}
