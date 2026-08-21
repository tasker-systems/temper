// shim.test.ts — the legacy `/vault/<owner>/<context>/graph` bookmark still lands somewhere.
//
// Beat D's acceptance says *"the 308 shim still resolves — verify, do not assume."* The
// destination changed twice underneath this route without the file being edited, which is the
// point of routing through `contextGraphHref` rather than spelling a path out; this test is what
// makes that indirection checkable instead of merely plausible.
import { describe, expect, it } from 'vitest';
import { load } from './+page.server';

/** Invoke the loader and return the redirect it threw, or fail if it returned normally. */
async function redirectFrom(owner: string, context: string) {
	try {
		// The loader reads only `params`; the rest of the event is never touched.
		await (load as (e: unknown) => Promise<unknown>)({ params: { owner, context } });
	} catch (e) {
		return e as { status: number; location: string };
	}
	throw new Error('the shim returned instead of redirecting');
}

describe('the legacy context-graph shim', () => {
	it('redirects 308 to the successor surface, with the context as an `in` anchor', async () => {
		const r = await redirectFrom('@me', 'temper');
		expect(r.status).toBe(308);
		expect(r.location).toBe('/graph/@me?in=ctx%3A%40me%2Ftemper');
	});

	it('is permanent, not a one-off — 308 preserves the method', async () => {
		// 303 would silently turn a POST into a GET. The distinction is why 308 was chosen.
		expect((await redirectFrom('+acme-team', 'ops')).status).toBe(308);
	});

	it('percent-encodes a slug with a space, so the bookmark survives it', async () => {
		const r = await redirectFrom('+acme-team', 'ops team');
		expect(new URL(r.location, 'https://temperkb.io').searchParams.get('in')).toBe(
			'ctx:+acme-team/ops team',
		);
	});
});
