import { describe, expect, it, vi } from 'vitest';
import { bounded, GaveUp } from './bounded';

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
	// and a late rejection changes nothing. It would bite a rewrite that stopped racing. **It does
	// not witness the global `.catch()`-at-creation constraint** — that constraint is real for a
	// promise handed to `{#await}` from a load, which genuinely is unsubscribed on the server, and
	// its probe lands there rather than here.
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
});
