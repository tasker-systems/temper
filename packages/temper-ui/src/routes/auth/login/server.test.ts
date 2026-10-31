// The sign-in door's default destination. Before the first view existed it sent a reader to the
// all-resources grid; arrival now lands where their own work is.
import { describe, expect, it, vi } from 'vitest';
import { ARRIVAL } from '$lib/first-view';
import { HOSTILE_RETURN_TO } from '$lib/server/hostile-return-to.fixture';

vi.mock('$lib/server/oidc', () => ({ authorizeUrl: async () => 'https://idp.test/authorize' }));
const writePkce = vi.fn(async (..._a: unknown[]) => {});
vi.mock('$lib/server/session', () => ({
	writePkce: (...a: unknown[]) => writePkce(...a),
}));

const { GET } = await import('./+server');

async function landing(search: string, signedIn: boolean): Promise<string | null> {
	const event = {
		url: new URL(`https://temper.test/auth/login${search}`),
		cookies: {},
		locals: { user: signedIn ? { id: 'me' } : null },
	} as unknown as Parameters<typeof GET>[0];
	try {
		await GET(event);
	} catch (thrown) {
		return (thrown as { location?: string }).location ?? null;
	}
	return null;
}

const returnTo = (value: string) => `?returnTo=${encodeURIComponent(value)}`;

describe('signing in lands on the first view', () => {
	it('sends a signed-in reader with nowhere named to the first view', async () => {
		expect(ARRIVAL).toBe('/vault');
		expect(await landing('', true)).toBe(ARRIVAL);
	});

	it('carries the first view through the provider round-trip for a reader signing in', async () => {
		expect(await landing('', false)).toBe('https://idp.test/authorize');
		expect(writePkce.mock.calls.at(-1)?.[1]).toMatchObject({ returnTo: ARRIVAL });
	});

	it('still honours a destination the reader was headed for', async () => {
		expect(await landing('?returnTo=/graph/@me', true)).toBe('/graph/@me');
	});

	it('honours a path with its query, as the app layout writes it', async () => {
		const headed = '/vault?tasks=24&goals=16';
		expect(await landing(returnTo(headed), true)).toBe(headed);
		expect(await landing(returnTo(headed), false)).toBe('https://idp.test/authorize');
		expect(writePkce.mock.calls.at(-1)?.[1]).toMatchObject({ returnTo: headed });
	});
});

describe('signing in never sends a reader off this site', () => {
	it.each(HOSTILE_RETURN_TO)(
		'sends a signed-in reader given %s to the first view',
		async (_label, value) => {
			expect(await landing(returnTo(value), true)).toBe(ARRIVAL);
		},
	);

	it.each(HOSTILE_RETURN_TO)(
		'stashes the first view, not %s, for a reader signing in',
		async (_label, value) => {
			writePkce.mockClear();
			expect(await landing(returnTo(value), false)).toBe('https://idp.test/authorize');
			expect(writePkce.mock.calls.at(-1)?.[1]).toMatchObject({ returnTo: ARRIVAL });
		},
	);
});
