// The callback re-checks the destination it reads from the PKCE cookie rather than trusting it: a
// cookie written by an older build may carry a value that was never checked.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ARRIVAL } from '$lib/first-view';
import { HOSTILE_RETURN_TO } from '$lib/server/hostile-return-to.fixture';

vi.mock('$lib/server/oidc', () => ({
	exchangeCode: async () => ({ access_token: 'at', refresh_token: 'rt', expires_in: 3600 }),
	identityClaimsFromTokens: () => ({ sub: 'me' }),
}));
let stashed: { state: string; verifier: string; returnTo: string } | null = null;
const writeSession = vi.fn(async (..._a: unknown[]) => {});
vi.mock('$lib/server/session', () => ({
	readPkce: async () => stashed,
	clearPkce: () => {},
	writeSession: (...a: unknown[]) => writeSession(...a),
}));

const { GET } = await import('./+server');

async function landing(returnTo: string): Promise<string | null> {
	stashed = { state: 's', verifier: 'v', returnTo };
	const event = {
		url: new URL('https://temper.test/auth/callback?code=c&state=s'),
		cookies: {},
	} as unknown as Parameters<typeof GET>[0];
	try {
		await GET(event);
	} catch (thrown) {
		return (thrown as { location?: string }).location ?? null;
	}
	return null;
}

describe('the sign-in callback lands a reader only on this site', () => {
	beforeEach(() => writeSession.mockClear());

	it('honours the stashed path the reader was headed for', async () => {
		expect(await landing('/graph/@me')).toBe('/graph/@me');
		expect(await landing('/vault?tasks=24')).toBe('/vault?tasks=24');
		expect(writeSession).toHaveBeenCalled();
	});

	it.each(HOSTILE_RETURN_TO)(
		'lands on the first view when the cookie holds %s',
		async (_label, value) => {
			expect(await landing(value)).toBe(ARRIVAL);
			expect(writeSession).toHaveBeenCalled();
		},
	);
});
