// page.server.test.ts — the first view's load.
//
// Until this change `/vault` redirected to the all-resources grid, so a reader arriving here met
// everything they could see rather than their own work. The first block below fails against that
// load: it threw a redirect and read nothing.
//
// `vi.mock` over `$lib/server/api` follows `../graph/[owner]/page.server.test.ts`: the loader's
// whole job is which reads it makes, and the paths are the witness.
import { WAY_IN_BOUND, WAYS_IN, type WayIn } from '@tasker-systems/temper-design/ways-in';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { placesPath, recentPath, resumePath, wayInPath } from '$lib/first-view';

const apiGet = vi.fn();
vi.mock('$lib/server/api', () => ({ apiGet: (...a: unknown[]) => apiGet(...a) }));

const { load } = await import('./+page.server');

const page = (rows: unknown[], total = rows.length) => ({ rows, total });

/** What the load hands the page: every region a promise of its own. */
type Loaded = {
	resume: Promise<unknown>;
	opening: Promise<unknown>;
	recent: Promise<unknown>;
	places: Promise<unknown>;
	ways: { way: WayIn; shown: number; list: Promise<unknown> }[];
};

const CONTEXT = {
	id: '01a0d34e-032b-72b3-8c5d-da7bc3da6727',
	name: 'core',
	slug: 'core',
	owner_ref: '+temper-dev',
};

async function run(search = '', contexts: unknown[] | null = [CONTEXT]): Promise<Loaded> {
	const event = {
		locals: { accessToken: 'tok' },
		parent: async () => ({ contexts }),
		url: new URL(`https://temper.test/vault${search}`),
	} as unknown as Parameters<typeof load>[0];
	return (await load(event)) as unknown as Loaded;
}

/** Answer each path; anything unlisted is a read the load must not make. */
function answer(by: Record<string, unknown>): void {
	apiGet.mockImplementation((path: string) =>
		path in by
			? by[path] instanceof Error
				? Promise.reject(by[path])
				: Promise.resolve(by[path])
			: Promise.reject(new Error(`unexpected read: ${path}`)),
	);
}

const everyListEmpty = (): Record<string, unknown> => ({
	[recentPath()]: page([]),
	[resumePath()]: page([]),
	[placesPath()]: [],
	...Object.fromEntries(WAYS_IN.map((w) => [wayInPath(w, WAY_IN_BOUND), page([])])),
});

beforeEach(() => {
	apiGet.mockReset();
});

describe('arrival starts from the work', () => {
	it('renders a first view instead of sending the reader to the grid', async () => {
		answer(everyListEmpty());
		const data = await run();
		expect(Object.keys(data).sort()).toEqual(['opening', 'places', 'recent', 'resume', 'ways']);
		await Promise.all([
			data.resume,
			data.opening,
			data.recent,
			data.places,
			...data.ways.map((w) => w.list),
		]);
	});

	// The latest handoff told a browsing reader nothing (handoffs are written by agents for the next
	// agent), so the first view no longer reads one. Every read the load makes is a region's.
	it('reads no session for a handoff', async () => {
		answer(everyListEmpty());
		await run();
		const reads = apiGet.mock.calls.map(([path]) => path as string);
		expect(reads.sort()).toEqual(Object.keys(everyListEmpty()).sort());
	});
});

describe('the same ways in on both surfaces', () => {
	it('reads every shared way in, and only those, under its own filter and bound', async () => {
		answer(everyListEmpty());
		const data = await run();
		expect(data.ways.map((w) => w.way)).toEqual(WAYS_IN);
		await Promise.all(data.ways.map((w) => w.list));
		for (const way of WAYS_IN) {
			const params = new URL(wayInPath(way, WAY_IN_BOUND), 'https://x').searchParams;
			expect(params.get('doc_type_name')).toBe(way.filter.docType);
			expect(params.get('owner')).toBe('@me');
			expect(params.get('status')).toBe(way.filter.status ?? null);
			expect(params.get('stage')).toBe(way.filter.stage ?? null);
			expect(params.get('limit')).toBe(String(WAY_IN_BOUND));
			expect([params.get('sort'), params.get('order')]).toEqual(['updated', 'desc']);
		}
	});

	it('grows one way in by its URL parameter and leaves the others at the bound', async () => {
		const goals = WAYS_IN[0];
		answer({ ...everyListEmpty(), [wayInPath(goals, 16)]: page([]) });
		const data = await run(`?${goals.id}=16`);
		expect(data.ways.map((w) => w.shown)).toEqual([16, WAY_IN_BOUND, WAY_IN_BOUND]);
		// `answer` rejects any read it was not given, so this resolves only if the read was limit 16.
		expect(await data.ways[0].list).toEqual(page([]));
	});

	it('fails one way in without touching the others', async () => {
		const [goals, tasks] = WAYS_IN;
		answer({ ...everyListEmpty(), [wayInPath(goals, WAY_IN_BOUND)]: new Error('503') });
		const data = await run();
		await expect(data.ways[0].list).rejects.toThrow('503');
		expect(data.ways[1].way).toBe(tasks);
		expect(await data.ways[1].list).toEqual(page([]));
	});
});

describe('the places nearest are the ones in use', () => {
	it("reads the reader's own latest change in each place", async () => {
		const activity = [
			{
				context_id: '01a0d34e-032b-72b3-8c5d-da7bc3da6727',
				last_changed_at: '2026-10-25T00:00:00Z',
			},
		];
		answer({ ...everyListEmpty(), [placesPath()]: activity });
		const data = await run();
		expect(await data.places).toEqual(activity);
	});

	it('fails the places alone when the activity cannot be read', async () => {
		answer({ ...everyListEmpty(), [placesPath()]: new Error('503') });
		const data = await run();
		await expect(data.places).rejects.toThrow('503');
		expect(await data.recent).toEqual(page([]));
	});
});

describe('work resumes where it was left', () => {
	const CHANGED = {
		id: '01a17626-b8cf-74e0-9f36-d0ec473d7128',
		title: 'A task',
		updated: '2026-10-26T12:00:00Z',
		changed_by_me_at: '2026-10-20T08:00:00Z',
	};

	it('offers the resource the reader last changed, with the time of their change', async () => {
		answer({ ...everyListEmpty(), [resumePath()]: page([CHANGED]) });
		const data = await run();
		expect(await data.resume).toEqual({ resource: CHANGED, changedAt: CHANGED.changed_by_me_at });
	});

	it('offers no return when the reader has changed nothing', async () => {
		answer(everyListEmpty());
		const data = await run();
		expect(await data.resume).toBeNull();
	});

	it('fails resume alone when its read fails', async () => {
		answer({ ...everyListEmpty(), [resumePath()]: new Error('503') });
		const data = await run();
		await expect(data.resume).rejects.toThrow('503');
		expect(await data.recent).toEqual(page([]));
	});

	it('reads the opening of that work from its body', async () => {
		answer({
			...everyListEmpty(),
			[resumePath()]: page([CHANGED]),
			[`/api/resources/${CHANGED.id}/content`]: {
				markdown: '# A task\n\nShip the return.\n\n## Done\nx',
			},
		});
		const data = await run();
		expect(await data.opening).toEqual({ markdown: 'Ship the return.', truncated: true });
	});

	it('reads no body when there is no return', async () => {
		answer(everyListEmpty());
		const data = await run();
		expect(await data.opening).toBeNull();
	});

	it('fails the opening alone when the body cannot be read', async () => {
		answer({
			...everyListEmpty(),
			[resumePath()]: page([CHANGED]),
			[`/api/resources/${CHANGED.id}/content`]: new Error('503'),
		});
		const data = await run();
		expect(await data.resume).toEqual({ resource: CHANGED, changedAt: CHANGED.changed_by_me_at });
		await expect(data.opening).rejects.toThrow('503');
	});

	it('reports a failed resume once, as resume’s failure, not again as the opening', async () => {
		answer({ ...everyListEmpty(), [resumePath()]: new Error('503') });
		const data = await run();
		await expect(data.resume).rejects.toThrow('503');
		expect(await data.opening).toBeNull();
	});

	it('gives the body read its own bound, starting when the return arrives', async () => {
		vi.useFakeTimers();
		try {
			const answersAfter = <T>(ms: number, v: T) =>
				new Promise<T>((r) => setTimeout(() => r(v), ms));
			const content = `/api/resources/${CHANGED.id}/content`;
			const reads: Record<string, unknown> = { ...everyListEmpty() };
			apiGet.mockImplementation((path: string) =>
				path === resumePath()
					? answersAfter(7_500, page([CHANGED]))
					: path === content
						? answersAfter(600, { markdown: 'Go.' })
						: Promise.resolve(reads[path]),
			);
			const data = await run();
			const opening = data.opening.then(
				(v) => ({ ok: v }),
				(e) => ({ failed: e }),
			);
			await vi.advanceTimersByTimeAsync(8_200);
			expect(await opening).toEqual({ ok: { markdown: 'Go.', truncated: false } });
		} finally {
			vi.useRealTimers();
		}
	});
});
