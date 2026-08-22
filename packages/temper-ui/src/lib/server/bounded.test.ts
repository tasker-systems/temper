import { describe, expect, it, vi } from 'vitest';
import { bounded, derive, describeFailure, GaveUp } from './bounded';

describe('a read the system stops waiting for', () => {
	it('resolves normally when the read answers in time', async () => {
		await expect(bounded(Promise.resolve('ok'), 'history', 50)).resolves.toBe('ok');
	});

	it('rejects with a NAMED give-up, so the region can say which read stopped', async () => {
		vi.useFakeTimers();
		const never = new Promise<string>(() => {});
		const p = bounded(never, 'history', 1000);
		vi.advanceTimersByTime(1001);
		await expect(p).rejects.toBeInstanceOf(GaveUp);
		await expect(p).rejects.toMatchObject({ label: 'history' });
		vi.useRealTimers();
	});

	it('a real failure still surfaces as itself, not as a give-up', async () => {
		const boom = Promise.reject(new Error('503'));
		await expect(bounded(boom, 'history', 50)).rejects.not.toBeInstanceOf(GaveUp);
	});

	// The bound must not outlive the read it bounds: a pending timer keeps the event loop
	// alive, so a read that answers first would still hold the process open.
	it('clears the bound when the read answers first, so nothing is left holding the loop', async () => {
		vi.useFakeTimers();
		await expect(bounded(Promise.resolve('ok'), 'history', 1000)).resolves.toBe('ok');
		expect(vi.getTimerCount()).toBe(0);
		vi.useRealTimers();
	});

	// `[corrected — 2026-08-21]` This test was written believing it witnessed the unhandled-rejection
	// trap. **It does not, and cannot.** `Promise.race` subscribes to every input immediately, so a
	// loser's late rejection is already handled — verified by executing a real `unhandledRejection`
	// listener against both shapes, where only an *unsubscribed* promise fires it. Any implementation
	// that passes "a real failure still surfaces as itself" must subscribe to `p`, and any that does
	// makes this trap unreachable.
	//
	// Kept, with its claim corrected, because it still pins the observable behaviour: the bound wins
	// and a late rejection changes nothing. It would bite a rewrite that stopped racing. The
	// constraint it does *not* witness is witnessed by the test below, which was the thing missing.
	it('the bound wins, and a late rejection changes nothing', async () => {
		vi.useFakeTimers();
		let reject!: (e: Error) => void;
		const late = new Promise<string>((_, r) => {
			reject = r;
		});
		const p = bounded(late, 'history', 10);
		vi.advanceTimersByTime(11);
		await expect(p).rejects.toBeInstanceOf(GaveUp);
		expect(() => reject(new Error('too late'))).not.toThrow();
		vi.useRealTimers();
	});

	/**
	 * The OTHER catch (spec §5.3), and **the version of it that can actually fail** — which is why
	 * this test exists at all. `[found — 2026-08-21]` The catch used to be attached to `bounded`'s
	 * *input*, where it is unwitnessable, and that was read as satisfying the constraint. It did not:
	 * a load hands `{#await}` the promise `bounded` **returns**, and on the server nothing subscribes
	 * to that one until SvelteKit serializes it.
	 *
	 * That is spec §5.3's own warning — *"having one does not give you the other"* — recurring one
	 * level down, inside the mechanism written to honour it.
	 */
	it('a returned promise nobody subscribed to does not crash the process', async () => {
		const fired: unknown[] = [];
		const onUnhandled = (reason: unknown) => fired.push(reason);
		process.on('unhandledRejection', onUnhandled);
		try {
			// Exactly the production shape: created, handed out, never consumed. Consuming it would
			// attach the very handler under test.
			void bounded(Promise.reject(new Error('503')), 'history', 50);

			// Node reports at the end of the turn in which the rejection went unhandled, not
			// synchronously — so drain a macrotask turn before looking.
			await new Promise((r) => setTimeout(r, 20));
			await new Promise((r) => setImmediate(r));

			expect(fired).toEqual([]);
		} finally {
			process.off('unhandledRejection', onUnhandled);
		}
	});

	// The catch above must MARK the rejection handled without CONSUMING it: a consumer's `{:catch}`
	// still has to see the failure, or the region renders nothing and the server merely survives.
	it('and every consumer still observes that rejection', async () => {
		await expect(bounded(Promise.reject(new Error('503')), 'history', 50)).rejects.toThrow('503');
	});
});

/**
 * `derive` carries the same invariant as `bounded` and had **no test of its own** — coverage was
 * incidental, through two loads that happen to leave a derived rejection unconsumed.
 *
 * Incidental coverage is the shape this whole change keeps finding: it holds until someone edits the
 * thing that was accidentally exercising it, and then it reports green while the guarantee is gone.
 * `derive` guards eleven streamed fields across the two graph loads, so its own failure mode is a
 * server that crashes on a read nobody was subscribed to.
 */
describe('a promise derived from a bounded one', () => {
	it('does not crash the process when nobody subscribed to it', async () => {
		const fired: unknown[] = [];
		const onUnhandled = (reason: unknown) => fired.push(reason);
		process.on('unhandledRejection', onUnhandled);
		try {
			// The production shape: derived off a read, handed to `{#await}`, unconsumed until
			// SvelteKit serializes it.
			void derive(bounded(Promise.reject(new Error('503')), 'history', 50), (v) => v);

			await new Promise((r) => setTimeout(r, 20));
			await new Promise((r) => setImmediate(r));

			expect(fired).toEqual([]);
		} finally {
			process.off('unhandledRejection', onUnhandled);
		}
	});

	// Marked handled, not consumed — the template's `{:catch}` still has to see it.
	it('and every consumer still observes the rejection', async () => {
		await expect(derive(Promise.reject(new Error('503')), (v) => v)).rejects.toThrow('503');
	});

	it('passes a value through the transform', async () => {
		await expect(derive(Promise.resolve(2), (v) => v * 21)).resolves.toBe(42);
	});
});

/**
 * What a give-up becomes when it leaves the server — the half `GaveUp` could not reach on its own.
 *
 * `[found — 2026-08-21]` `GaveUp` carried a `label` and **nothing read it**, because nothing could:
 * SvelteKit hands every rejected streamed promise to `handleError` and serialises whatever that
 * returns, so `error instanceof GaveUp` in a client `{:catch}` is unreachable by construction. The
 * fix is a *value* the client can branch on, and this is where the value is minted.
 */
describe('the give-up, described for a reader who cannot see the class', () => {
	it('names which read the system stopped waiting for', () => {
		expect(describeFailure(new GaveUp('history', 8000), 'Internal Error')).toEqual({
			message: 'gave up waiting for history after 8000ms',
			gaveUp: 'history',
		});
	});

	// A real failure is not a give-up, and must not borrow its words. The sanitised message is
	// SvelteKit's, and stays SvelteKit's — this hook adds a field, it does not open a leak.
	it('leaves a real failure exactly as SvelteKit sanitised it', () => {
		expect(describeFailure(new Error('503: postgres said no'), 'Internal Error')).toEqual({
			message: 'Internal Error',
		});
	});

	it('survives a rejection that is not an Error at all', () => {
		expect(describeFailure('nope', 'Internal Error')).toEqual({ message: 'Internal Error' });
	});
});
