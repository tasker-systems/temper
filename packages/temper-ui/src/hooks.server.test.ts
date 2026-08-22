import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { runInThisContext } from 'node:vm';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { regionStateFor } from '$lib/region';
import { GaveUp } from '$lib/server/bounded';

/**
 * `$lib/server/oidc` resolves its config **at module scope** (`oidc.ts:47`), so importing
 * `hooks.server.ts` at all throws `OIDC issuer not configured` wherever that env is absent — which
 * is every CI run on a pull request, since a fork-visible workflow gets no secrets. `[found in CI —
 * 2026-08-22]` It passed locally only because a `.env` happened to be on disk; reproduced by moving
 * that file aside.
 *
 * The mock stands in for a **module-load side effect**, not for anything under test. `handleError`
 * does not touch OIDC — it reads an error and returns a shape — so importing the app's real hook,
 * which is the whole point of this file, is preserved exactly.
 */
vi.mock('$lib/server/oidc', () => ({
	REFRESH_THRESHOLD_SECONDS: 60,
	refreshAccessToken: () => Promise.reject(new Error('not used by handleError')),
}));

/**
 * The same thing one module over: `session.ts:49` derives its key from `SESSION_SECRET` at module
 * scope, via `$env/static/private`. Also unrelated to `handleError`, and also fatal at import.
 *
 * Both were found by moving `.env` aside and running this file — not by reading the import graph.
 * Mocking one and shipping would have failed CI a second time on the next module down.
 */
vi.mock('$lib/server/session', () => ({
	readSession: () => Promise.resolve(null),
	writeSession: () => Promise.resolve(),
	clearSession: () => {},
}));

import { handleError } from './hooks.server';

/**
 * **The probe that decides whether any of this is real**: the discriminator crossing the wire.
 *
 * A component test constructs its error in the same process it renders in, so it witnesses the
 * template branching and nothing else — spec §6.1's *"69 green tests and zero callers"* wearing a
 * different costume. This runs SvelteKit's own promise serialiser, the exact code path a **streamed**
 * rejection takes, with **this app's real `handleError`** installed, then executes the emitted chunk
 * the way a browser executes the inline `<script>` and reads what a `{:catch}` would be handed.
 *
 * It imports `hooks.server.ts` on purpose rather than calling `describeFailure` directly. The hook
 * being exported at all is half the mechanism: `[found — 2026-08-21]` the give-up shipped with a
 * `label` and no reader because nothing joined the two ends, and a test that supplied its own hook
 * would leave exactly that join unwitnessed again.
 *
 * It reaches into `@sveltejs/kit`'s internals, deliberately and with the cost stated: the serialiser
 * is not on kit's `exports` map, so a kit upgrade that moves or renames it fails this file loudly.
 * That is the intended failure — the alternative is to assert nothing about the boundary.
 *
 * **What it still does not witness**, named rather than implied: the HTTP response actually being
 * chunked, and the client runtime reattaching to the deferred promise. Only `temper-e2e` could reach
 * those, and it does not yet.
 */
describe('the give-up, across the boundary a class cannot cross', () => {
	/** Kit's serialiser, reached by path because its `exports` map does not publish it. */
	const kitSerializer = async () => {
		const kitRoot = dirname(createRequire(import.meta.url).resolve('@sveltejs/kit/package.json'));
		const mod = await import(
			pathToFileURL(join(kitRoot, 'src/runtime/server/page/data_serializer.js')).href
		);
		return mod.server_data_serializer as (
			event: unknown,
			state: unknown,
			options: unknown,
		) => {
			add_node: (i: number, node: unknown) => void;
			get_data: (csp: { script_needs_nonce: boolean }) => {
				data: string;
				chunks: AsyncIterable<string> | null;
			};
		};
	};

	/**
	 * A rejection with spec §5.3's *other* catch already attached.
	 *
	 * The serialiser does not subscribe until a dynamic import has resolved, which is several turns
	 * after these are created, and node reports an unhandled rejection at the end of the turn.
	 */
	const rejected = (e: unknown): Promise<never> => {
		const p = Promise.reject(e);
		p.catch(() => {});
		return p;
	};

	/** One give-up and one ordinary failure, streamed side by side so they can be compared. */
	const twoRejections = () => ({
		history: rejected(new GaveUp('history', 8000)),
		other: rejected(new Error('503')),
	});

	/**
	 * Serialise those rejections as a streamed load does, then execute the chunks as the browser
	 * does, and return what each deferred promise is rejected with — keyed by the promise id kit
	 * assigns in order, so `1` is the give-up and `2` the failure.
	 *
	 * `hook` is a parameter because the whole finding is the difference between having one and not.
	 */
	const asTheClientReceivesThem = async (
		hook: unknown = handleError,
	): Promise<Record<number, unknown>> => {
		const serializer = await kitSerializer();
		const s = serializer(
			{
				route: { id: '/graph/[owner]' },
				isDataRequest: false,
				url: new URL('http://localhost/graph/@me'),
				request: { method: 'GET' },
			},
			{},
			{ hooks: { handleError: hook, transport: {} }, version_hash: 'probe' },
		);
		s.add_node(0, { type: 'data', data: twoRejections() });
		const { chunks } = s.get_data({ script_needs_nonce: false });

		const received: Record<number, unknown> = {};
		// `resolve(id, fn)` is what the browser calls; `fn()` yields `[data, error]` and kit's client
		// runtime rejects the deferred promise with `error`. Both global names are bound because kit
		// chooses between them on its own `DEV` flag.
		const collect = {
			resolve: (id: number, fn: () => [unknown, unknown]) => {
				received[id] = fn()[1];
			},
		};
		Object.assign(globalThis, { __sveltekit_dev: collect, __sveltekit_probe: collect });

		// Kit wraps each chunk in a bare `<script>` (bare because `script_needs_nonce: false` above).
		// Unwrapped by exact string bounds rather than by regex, deliberately: `[found by CodeQL —
		// 2026-08-22]` a tag-shaped regex here trips `js/bad-tag-filter` at high severity, and the
		// scanner is right to be suspicious of the *shape* even though this is not sanitisation —
		// the input is a chunk this test just asked kit to emit. Exact bounds say that plainly, and
		// the assertion below means a kit change to the wrapper fails loudly rather than executing
		// tag text as source.
		const OPEN = '<script>';
		const CLOSE = '</script>';
		for await (const chunk of chunks ?? []) {
			const trimmed = chunk.trim();
			expect(trimmed.startsWith(OPEN) && trimmed.endsWith(CLOSE)).toBe(true);
			// Executed, not pattern-matched: a chunk whose text contains "gaveUp" is not evidence that
			// a browser would decode one. `runInThisContext` is the nearest node has to an inline
			// `<script>` — same globals, no module scope.
			runInThisContext(trimmed.slice(OPEN.length, -CLOSE.length));
		}
		return received;
	};

	beforeEach(() => {
		// The hook logs every error it describes, which is its job and not this file's output.
		vi.spyOn(console, 'error').mockImplementation(() => {});
	});
	afterEach(() => {
		vi.restoreAllMocks();
	});

	/**
	 * The finding, executable. With no hook, SvelteKit sanitises both to the SAME object — so
	 * `error instanceof GaveUp` in a client `{:catch}` is not merely false, there is nothing left to
	 * ask the question of. This is what the six `{:catch}` blocks were reading before this task.
	 */
	it('without a hook, a give-up and a 503 arrive identical', async () => {
		const received = await asTheClientReceivesThem(() => {});

		expect(received[1]).toEqual({ message: 'Internal Error' });
		expect(received[1]).toEqual(received[2]);
	});

	it("with this app's hook, the give-up arrives naming the read it stopped waiting for", async () => {
		const received = await asTheClientReceivesThem();

		expect(received[1]).toEqual({
			message: 'gave up waiting for history after 8000ms',
			gaveUp: 'history',
		});
	});

	// The addition is one field on one kind of error. A real failure keeps SvelteKit's sanitised
	// message, or this hook is a route for internals to escape rather than a discriminator.
	it('and leaves an ordinary failure exactly as SvelteKit sanitised it', async () => {
		const received = await asTheClientReceivesThem();

		expect(received[2]).toEqual({ message: 'Internal Error' });
	});

	// The field is worth nothing unless the client's reader agrees with it, so the two halves are
	// joined here rather than left to line up by inspection.
	it('and the client reader calls it a give-up on the far side, and the other a failure', async () => {
		const received = await asTheClientReceivesThem();

		expect(regionStateFor(received[1])).toBe('gave-up');
		expect(regionStateFor(received[2])).toBe('failed');
	});
});
