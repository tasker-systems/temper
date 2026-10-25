// page.server.test.ts — the first view's load.
//
// Until this change `/vault` redirected to the all-resources grid, so a reader arriving here met
// everything they could see rather than their own work. The first block below fails against that
// load: it threw a redirect and read nothing.
//
// `vi.mock` over `$lib/server/api` follows `../graph/[owner]/page.server.test.ts`: the loader's
// whole job is which reads it makes, and the paths are the witness.
import { HANDOFF_LABEL } from '@tasker-systems/temper-design/handoff';
import { WAY_IN_BOUND, WAYS_IN, type WayIn } from '@tasker-systems/temper-design/ways-in';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { handoffPath, recentPath, wayInPath } from '$lib/first-view';

const apiGet = vi.fn();
vi.mock('$lib/server/api', () => ({ apiGet: (...a: unknown[]) => apiGet(...a) }));

const { load } = await import('./+page.server');

const SESSION = { id: '01a17104-f5db-7782-b326-70ce7109c2d5', title: 'A session' };
const page = (rows: unknown[], total = rows.length) => ({ rows, total });

/** What the load hands the page: every region a promise of its own. */
type Loaded = {
	session: Promise<unknown>;
	next: Promise<unknown>;
	recent: Promise<unknown>;
	ways: { way: WayIn; shown: number; list: Promise<unknown> }[];
};

async function run(search = ''): Promise<Loaded> {
	const event = {
		locals: { accessToken: 'tok' },
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
	[handoffPath()]: page([]),
	[recentPath()]: page([]),
	...Object.fromEntries(WAYS_IN.map((w) => [wayInPath(w, WAY_IN_BOUND), page([])])),
});

beforeEach(() => {
	apiGet.mockReset();
});

describe('arrival starts from the work', () => {
	it('renders a first view instead of sending the reader to the grid', async () => {
		answer(everyListEmpty());
		const data = await run();
		expect(Object.keys(data).sort()).toEqual(['next', 'recent', 'session', 'ways']);
		await Promise.all([data.session, data.next, data.recent, ...data.ways.map((w) => w.list)]);
	});

	it("quotes the next steps of the reader's newest session, read from its body", async () => {
		answer({
			...everyListEmpty(),
			[handoffPath()]: page([SESSION]),
			[`/api/resources/${SESSION.id}/content`]: { markdown: '## Done\nx\n## Next Steps\nShip it.' },
		});
		const data = await run();
		expect(await data.session).toEqual(SESSION);
		expect(await data.next).toEqual({ heading: 'Next Steps', text: 'Ship it.', truncated: false });
	});

	it('quotes nothing, and reads no body, when the reader has no session', async () => {
		answer(everyListEmpty());
		const data = await run();
		expect(await data.session).toBeNull();
		expect(await data.next).toBeNull();
	});

	it('fails the next steps alone when the body cannot be read', async () => {
		answer({
			...everyListEmpty(),
			[handoffPath()]: page([SESSION]),
			[`/api/resources/${SESSION.id}/content`]: new Error('503'),
		});
		const data = await run();
		expect(await data.session).toEqual(SESSION);
		await expect(data.next).rejects.toThrow('503');
		expect(await data.recent).toEqual(page([]));
	});

	it('reads the handoff from the reader’s own sessions, newest update first', () => {
		const params = new URL(handoffPath(), 'https://x').searchParams;
		expect(Object.fromEntries(params)).toEqual({
			doc_type_name: 'session',
			owner: '@me',
			limit: '1',
			sort: 'updated',
			order: 'desc',
		});
	});

	it(`labels the handoff "${HANDOFF_LABEL}", as the desktop does`, () => {
		expect(HANDOFF_LABEL).toBe('latest handoff');
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
