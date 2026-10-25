import { beforeEach, describe, expect, it, vi } from 'vitest';

const apiGet = vi.fn();

// The `$lib/server/*` mock idiom of `routes/(app)/vault/r/[ident]/page.server.test.ts`.
vi.mock('$lib/server/api', () => ({
	apiGet: (...a: unknown[]) => apiGet(...a),
}));

const { resolveBodyLinks } = await import('./body-links');

const A = '019d4add-f49d-7c43-a87d-dda470e5dd9c';
const B = '01a16e2a-570d-77fa-9ca5-a905fe467e9a';
const C = '01a17073-2051-704b-bd59-dc75bd12b09b';
const BODY = `See [a](./${A}), [b](./${B}) and [c](./${C}), and [a again](./${A}).`;

const page = (rows: { id: string; title: string }[], truncated = false) => ({
	rows,
	total: rows.length,
	returned: rows.length,
	truncated,
	limit: 3,
	offset: 0,
	facets: {},
});

beforeEach(() => vi.clearAllMocks());

describe('resolveBodyLinks', () => {
	it('asks once, for every distinct id, with a page that holds them all', async () => {
		apiGet.mockResolvedValue(page([]));
		await resolveBodyLinks(BODY, 'tok');
		expect(apiGet).toHaveBeenCalledTimes(1);
		const [path, token] = apiGet.mock.calls[0];
		const query = new URL(path, 'http://x').searchParams;
		expect(new URL(path, 'http://x').pathname).toBe('/api/resources');
		expect(query.get('ids')).toBe(`${A},${B},${C}`);
		expect(query.get('limit')).toBe('3');
		expect(token).toBe('tok');
	});

	it('answers a returned row with its held title, and an asked id that did not come back as absent', async () => {
		apiGet.mockResolvedValue(page([{ id: A, title: 'Held A' }]));
		expect(new Map(await resolveBodyLinks(BODY, 'tok'))).toEqual(
			new Map([
				[A, { state: 'resolved', title: 'Held A' }],
				[B, { state: 'absent' }],
				[C, { state: 'absent' }],
			]),
		);
	});

	it('a truncated page proves nothing about what it did not return: those ids stay unanswered', async () => {
		apiGet.mockResolvedValue(page([{ id: A, title: 'Held A' }], true));
		expect(await resolveBodyLinks(BODY, 'tok')).toEqual([
			[A, { state: 'resolved', title: 'Held A' }],
		]);
	});

	it('a failed read answers nothing, and never rejects', async () => {
		apiGet.mockRejectedValue(new Error('upstream down'));
		await expect(resolveBodyLinks(BODY, 'tok')).resolves.toEqual([]);
	});

	it('a body with no vault links makes no request', async () => {
		expect(await resolveBodyLinks('[web](https://example.com)', 'tok')).toEqual([]);
		expect(apiGet).not.toHaveBeenCalled();
	});
});
