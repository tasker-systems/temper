import { describe, expect, it } from 'vitest';
import { mayChangeResource } from './authority';

const VIEWER = 'profile-viewer';
const OTHER = 'profile-other';
const CTX = 'ctx-1';

function resource(over: Partial<Parameters<typeof mayChangeResource>[0]> = {}) {
	return {
		kb_context_id: CTX,
		owner_profile_id: OTHER,
		is_active: true,
		...over,
	} as Parameters<typeof mayChangeResource>[0];
}

type ContextArg = NonNullable<Parameters<typeof mayChangeResource>[1]>[number];

function context(canWrite: boolean, id = CTX): ContextArg {
	return { id, can_write: canWrite } as unknown as ContextArg;
}

describe('mayChangeResource', () => {
	it('answers yes on the container arm', () => {
		expect(mayChangeResource(resource(), [context(true)], VIEWER)).toBe(true);
	});

	it('answers yes on the owner arm even where the container says no', () => {
		// The two arms are a UNION, not a conjunction: `can_write` is the container cascade,
		// and `can_modify_resource` admits the home owner separately.
		expect(
			mayChangeResource(resource({ owner_profile_id: VIEWER }), [context(false)], VIEWER),
		).toBe(true);
	});

	it('answers no where the reader only reaches the context', () => {
		// THE BITE the whole arm exists for: reach does not imply change authority. A watcher
		// and a read-only grant both see this resource. Deriving offerability from visibility
		// would answer true here, and that is the fail-open shape.
		expect(mayChangeResource(resource(), [context(false)], VIEWER)).toBe(false);
	});

	it('treats an unreadable context list as unknown, never as permitted', () => {
		// `contexts` is null only when the read FAILED — the layout keeps that distinct from
		// `[]` on purpose. Unknown must not become an offer.
		expect(mayChangeResource(resource(), null, VIEWER)).toBe(false);
	});

	it('still answers yes from the owner arm when the context list is unreadable', () => {
		// The arms are independent. A failed container read must not suppress an answer that
		// never needed it.
		expect(mayChangeResource(resource({ owner_profile_id: VIEWER }), null, VIEWER)).toBe(true);
	});

	it('answers no when the resource is homed in a context the list does not carry', () => {
		expect(mayChangeResource(resource(), [context(true, 'ctx-elsewhere')], VIEWER)).toBe(false);
	});

	it('refuses everyone on an inactive resource, owner included', () => {
		// The floor sits UNDER the union — an owner is not an exception to it.
		expect(
			mayChangeResource(
				resource({ owner_profile_id: VIEWER, is_active: false }),
				[context(true)],
				VIEWER,
			),
		).toBe(false);
	});

	it('answers no for a cogmap-homed resource the owner arm does not cover', () => {
		// `kb_context_id` is null for a cognitive-map home. The container arm cannot speak to
		// it; answering from a context row that happens to be in the list would be answering
		// about the wrong container.
		expect(mayChangeResource(resource({ kb_context_id: null }), [context(true)], VIEWER)).toBe(
			false,
		);
	});
});
