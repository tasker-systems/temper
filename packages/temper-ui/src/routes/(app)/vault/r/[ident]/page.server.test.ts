// page.server.test.ts — the two things about this load that no component test can see.
//
// The component tests render the template's branches. Neither of the properties below is visible
// from there: whether the load handed the template a promise or a settled value, and whether a
// streamed promise that rejects while nothing is subscribed takes the server down.
//
// `vi.mock` over `$lib/server/*` follows the idiom established by
// `src/routes/(app)/graph/[owner]/page.server.test.ts` — module-scope `vi.fn()`s, `vi.mock`
// forwarding to them, then a dynamic `import` of the module under test so the mocks are installed
// before it is evaluated.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const apiGet = vi.fn();
const readTrail = vi.fn();
const readResourceEdges = vi.fn();

vi.mock('$lib/server/api', () => ({
	apiGet: (...a: unknown[]) => apiGet(...a),
	ApiError: class extends Error {
		status = 500;
	},
}));
vi.mock('$lib/server/graph-reads', () => ({
	readTrail: (...a: unknown[]) => readTrail(...a),
	readResourceEdges: (...a: unknown[]) => readResourceEdges(...a),
}));

const { load } = await import('./+page.server');

const run = () =>
	(load as (e: unknown) => Promise<Record<string, unknown>>)({
		locals: { accessToken: 'tok' },
		params: { ident: 'r1' },
	});

beforeEach(() => {
	vi.clearAllMocks();
	// Dispatch on the PATH rather than on call order. Streaming moved the three fill reads above
	// the scaffold's `await` — that ordering is the change — so an order-keyed mock would hand the
	// never-settling promise to the resource read and time out for the wrong reason.
	apiGet.mockImplementation((path: string) =>
		path.endsWith('/content')
			? new Promise(() => {})
			: Promise.resolve({ id: 'r1', title: 'A resource' }),
	);
	readTrail.mockReturnValue(new Promise(() => {}));
	readResourceEdges.mockReturnValue(new Promise(() => {}));
});

describe('the resource page does not block on its fill', () => {
	it('C1: returns the scaffold with the fill still unsettled', async () => {
		const data = await run();

		// The scaffold is a value; the fill is still a promise. If someone adds an `await`,
		// this load never returns and the test times out — which is the regression to catch.
		expect(data.resource).toMatchObject({ title: 'A resource' });
		expect(data.trail).toBeInstanceOf(Promise);
		expect(data.edges).toBeInstanceOf(Promise);
		expect(data.content).toBeInstanceOf(Promise);
	});
});

/**
 * The OTHER catch (spec §5.3) — the one that keeps a rejection from crashing the process, as
 * opposed to the `{:catch}` that renders the failure. Having one does not give you the other.
 *
 * **This is where that constraint becomes witnessable, at the load level.** A catch on `bounded`'s
 * *input* cannot be witnessed — `Promise.race` already subscribes to it — but the promise `bounded`
 * *returns* is a further derivation that nothing inside it subscribes to, and on the server that one
 * stays genuinely unsubscribed until SvelteKit serializes it. `bounded` catches on what it hands
 * out, which is what this test pins from the outside: delete that line in `bounded.ts` and this
 * fails with the read's own error. `bounded.test.ts` pins the same guarantee at the unit.
 *
 * **One thing this test cannot see, recorded so it is not banked as coverage.** A load that handed
 * out the mocked read's return value *directly*, with no `.catch()` anywhere, still passes here —
 * because `vi.fn()` subscribes to every promise it returns in order to record `settledResults`
 * (tinyspy `dist/index.js:52`: `w(o) && o.then(ok, err)`). The mock handles the rejection the
 * production code failed to. So this witnesses the catch on the promise `bounded` *returns* — the
 * one actually handed to the template — and not the read's own promise.
 */
describe('a streamed read that fails does not take the server down with it', () => {
	it('rejects into nothing, with nobody subscribed, without an unhandled rejection', async () => {
		readTrail.mockReturnValue(Promise.reject(new Error('503 from the trail read')));

		const fired: unknown[] = [];
		const onUnhandled = (reason: unknown) => fired.push(reason);
		process.on('unhandledRejection', onUnhandled);
		try {
			// Deliberately never consumed — consuming it would attach the very handler under test.
			const data = await run();
			expect(data.trail).toBeInstanceOf(Promise);

			// Node reports an unhandled rejection at the end of the turn in which it went
			// unhandled, not synchronously — so drain a macrotask turn before looking.
			await new Promise((r) => setTimeout(r, 20));
			await new Promise((r) => setImmediate(r));

			expect(fired).toEqual([]);
		} finally {
			process.off('unhandledRejection', onUnhandled);
		}
	});
});
