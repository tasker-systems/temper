// +page.server.test.ts — the load that decides "no request on file" versus "the read failed".
//
// `/api/access/requests/me` answers the no-request state as a `null` ON A 200 (`body =
// Option<JoinRequest>`, `handlers/access.rs`) — this door has no 404 to conflate with a 5xx.
// The distinction the tests below hold: a `null` reaches the page as null, and every failure
// rejects, so a backend outage never renders the fresh-submission form.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const apiGet = vi.fn();

class ApiError extends Error {
	status: number;
	constructor(status: number, message: string) {
		super(message);
		this.status = status;
	}
}

vi.mock('$lib/server/api', () => ({
	ApiError,
	apiGet: (...a: unknown[]) => apiGet(...a),
	apiPost: vi.fn(),
	apiDelete: vi.fn(),
}));

const { load } = await import('./+page.server');

const SETTINGS = {
	terms_version: 'v1',
	terms_resource_uri: null,
	instance_name: 'Steward Archive',
};

const run = () =>
	(load as (e: unknown) => Promise<Record<string, unknown>>)({
		locals: {
			user: { sub: 'user-1' },
			accessToken: 'tok',
			entitlements: {},
		},
		url: new URL('https://temperkb.io/request-access'),
	});

beforeEach(() => {
	vi.clearAllMocks();
	apiGet.mockImplementation((path: string) => {
		if (path === '/api/access/requests/me') return Promise.resolve(null);
		if (path === '/api/access/settings') return Promise.resolve(SETTINGS);
		return Promise.reject(new ApiError(404, 'not under test'));
	});
});

describe('the own-request read — the null is the wire’s, a failure is a failure', () => {
	it('a failed own-request read rejects the load — it never reads as "no request on file"', async () => {
		apiGet.mockImplementation((path: string) => {
			if (path === '/api/access/requests/me') {
				return Promise.reject(new ApiError(500, 'requests door down'));
			}
			if (path === '/api/access/settings') return Promise.resolve(SETTINGS);
			return Promise.reject(new ApiError(404, 'not under test'));
		});

		await expect(run()).rejects.toThrow('requests door down');
	});

	it('the null the wire carries reaches the page as null, with the settings beside it', async () => {
		const data = await run();

		expect(data.ownRequest).toBeNull();
		expect(data.settings).toEqual(SETTINGS);
	});
});

describe('the settings read — the endpoint declares 200 and 401, so any failure is a failure', () => {
	it('a failed settings read rejects the load — it never arrives as absent settings', async () => {
		apiGet.mockImplementation((path: string) => {
			if (path === '/api/access/requests/me') return Promise.resolve(null);
			if (path === '/api/access/settings') {
				return Promise.reject(new ApiError(500, 'settings door down'));
			}
			return Promise.reject(new ApiError(404, 'not under test'));
		});

		await expect(run()).rejects.toThrow('settings door down');
	});
});
