// region.ts
/**
 * The give-up, read on the side of the boundary that has no classes.
 *
 * `$lib/server/bounded` rejects with a `GaveUp` **on the server**, and that instance never reaches a
 * browser. SvelteKit hands every rejected streamed promise to `handleError` and serialises what the
 * hook returns — a prototype does not survive that, so `error instanceof GaveUp` inside a client
 * `{:catch}` is unreachable by construction, not merely unlikely. `[found — 2026-08-21]` That is why
 * `GaveUp.label` had a writer and no reader: nothing on the far side could ever have seen it.
 *
 * So the discriminator is a **value**, `App.Error['gaveUp']`, minted by `describeFailure` in
 * `$lib/server/bounded` and read here. The two halves are held together by one declaration —
 * `App.Error` in `src/app.d.ts` — rather than by two spellings of the same string.
 *
 * This module is deliberately **not** under `$lib/server`: it is imported by components, and the
 * whole point is that it never needs the class.
 */

/** The two states a rejected read can present as. `arriving` and `empty` are not failures. */
export type RegionFailure = 'gave-up' | 'failed';

/**
 * Which of the two a `{:catch}` is holding.
 *
 * Anything without the field is a plain failure, and that includes an actual `GaveUp` instance: an
 * `Error` is what a test is tempted to construct and what production never delivers here, so
 * recognising one would make this function agree with a test that the browser disagrees with.
 */
export function regionStateFor(error: unknown): RegionFailure {
	const failure = error as Partial<App.Error> | null | undefined;
	return typeof failure?.gaveUp === 'string' ? 'gave-up' : 'failed';
}
