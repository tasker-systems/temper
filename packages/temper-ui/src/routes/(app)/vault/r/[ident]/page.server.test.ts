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
const apiPatch = vi.fn();

/**
 * The `ApiError` the module under test does `instanceof` against — the SAME class object, so a
 * rejection minted here is recognized there. A hand-rolled `{status}` object is not: the load
 * distinguishes a 404 (this kind carries no states) from any other failure (nobody knows), and
 * that branch is only reachable through the real prototype.
 */
class ApiErrorStub extends Error {
	status = 500;
}

vi.mock('$lib/server/api', () => ({
	apiGet: (...a: unknown[]) => apiGet(...a),
	apiPatch: (...a: unknown[]) => apiPatch(...a),
	ApiError: ApiErrorStub,
}));
vi.mock('$lib/server/graph-reads', () => ({
	readTrail: (...a: unknown[]) => readTrail(...a),
	readResourceEdges: (...a: unknown[]) => readResourceEdges(...a),
}));

const { load, actions } = await import('./+page.server');

/**
 * The `(app)` layout's data, which this load reads through `parent()` for the authority union.
 * `contexts` and `profile` are already loaded there, which is why offerability costs no read.
 */
const VIEWER = 'p-viewer';
const CTX = 'ctx-1';

const run = (parentData: Record<string, unknown> = {}) =>
	(load as (e: unknown) => Promise<Record<string, unknown>>)({
		locals: { accessToken: 'tok' },
		params: { ident: 'r1' },
		parent: async () => ({
			contexts: [{ id: CTX, can_write: false }],
			profile: { id: VIEWER },
			...parentData,
		}),
	});

beforeEach(() => {
	vi.clearAllMocks();
	// Dispatch on the PATH rather than on call order. Streaming moved the three fill reads above
	// the scaffold's `await` — that ordering is the change — so an order-keyed mock would hand the
	// never-settling promise to the resource read and time out for the wrong reason.
	apiGet.mockImplementation((path: string) => {
		if (path.endsWith('/content')) return new Promise(() => {});
		// The artifacts fill never settles by default, like the other three: the load's contract
		// is that it hands the template promises, and a settled default would let a load that
		// awaits this read pass C1 by accident.
		if (path.includes('/artifacts')) return new Promise(() => {});
		if (path.includes('/api/schema/doc-types/'))
			return Promise.resolve({ enum_fields: { 'temper-stage': ['backlog', 'done'] } });
		return Promise.resolve({
			id: 'r1',
			title: 'A resource',
			doc_type_name: 'task',
			kb_context_id: CTX,
			owner_profile_id: 'p-someone-else',
			is_active: true,
		});
	});
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
		expect(data.artifacts).toBeInstanceOf(Promise);
	});
});

describe('the artifacts read', () => {
	it("asks for the folded ones too — whether an artifact is live is the reader's question", async () => {
		apiGet.mockImplementation((path: string) => {
			if (path.includes('/artifacts')) return Promise.resolve([]);
			if (path.endsWith('/content')) return Promise.resolve({ markdown: '# b' });
			if (path.includes('/api/schema/doc-types/')) return Promise.resolve({ enum_fields: {} });
			return Promise.resolve({
				id: 'r1',
				title: 'A resource',
				doc_type_name: 'task',
				kb_context_id: CTX,
				owner_profile_id: 'p-someone-else',
				is_active: true,
			});
		});
		const data = await run();
		await expect(data.artifacts).resolves.toEqual([]);
		const asked = apiGet.mock.calls.map(([path]) => String(path));
		expect(asked).toContain('/api/resources/r1/artifacts?include_folded=true');
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

/**
 * Offerability, decided in the load — before anything is offered, never by attempting.
 *
 * These sit here rather than in the component test because the component cannot see them: it
 * receives `mayChange`/`stateVocabulary` already decided. What is at stake is *how* they were
 * decided, and the load is where that happens.
 */
describe('a change is offered only where the reader holds authority to make it', () => {
	it('offers nothing to a reader who reaches the context but cannot author into it', async () => {
		// THE BITE. This reader can see the resource — the read succeeded — and `can_write` is
		// false. Reach is not change authority, and a surface deriving one from the other is the
		// fail-open shape this arm exists to prevent.
		const data = await run();
		expect(data.mayChange).toBe(false);
		expect(data.stateVocabulary).toBeNull();
	});

	it('does not even ASK for the vocabulary when nothing can be offered', async () => {
		await run();
		const asked = apiGet.mock.calls.map(([path]) => String(path));
		expect(asked.some((p) => p.includes('/api/schema/doc-types/'))).toBe(false);
	});

	it('offers the states the work carries to a reader who may author into the context', async () => {
		const data = await run({ contexts: [{ id: CTX, can_write: true }] });
		expect(data.mayChange).toBe(true);
		// Read, not restated: the vocabulary is whatever `enum_fields` answered for this kind.
		expect(data.stateVocabulary).toEqual({ 'temper-stage': ['backlog', 'done'] });
		const asked = apiGet.mock.calls.map(([path]) => String(path));
		expect(asked).toContain('/api/schema/doc-types/task');
	});

	it('treats an unreadable context list as unknown rather than as permission', async () => {
		const data = await run({ contexts: null });
		expect(data.mayChange).toBe(false);
	});

	it('offers nothing, definitively, for a kind with no schema', async () => {
		// A 404 is not a failure — it is the answer that this kind of work carries no states.
		// Live resources sit on out-of-vocabulary doc types.
		apiGet.mockImplementation((path: string) => {
			if (path.endsWith('/content')) return new Promise(() => {});
			if (path.includes('/api/schema/doc-types/')) {
				const err = new Error('no such doc type') as Error & { status: number };
				err.status = 404;
				Object.setPrototypeOf(err, ApiErrorStub.prototype);
				return Promise.reject(err);
			}
			return Promise.resolve({
				id: 'r1',
				title: 'A resource',
				doc_type_name: 'kernel_landmark',
				kb_context_id: CTX,
				owner_profile_id: 'p-someone-else',
				is_active: true,
			});
		});
		const data = await run({ contexts: [{ id: CTX, can_write: true }] });
		expect(data.mayChange).toBe(true);
		expect(data.stateVocabulary).toEqual({});
	});

	it('distinguishes a vocabulary that could not be read from one that is empty', async () => {
		// Both offer nothing. Only one of them is a degradation the reader should be able to
		// see, so `null` and `{}` must not collapse into each other.
		apiGet.mockImplementation((path: string) => {
			if (path.endsWith('/content')) return new Promise(() => {});
			if (path.includes('/api/schema/doc-types/')) return Promise.reject(new Error('503'));
			return Promise.resolve({
				id: 'r1',
				title: 'A resource',
				doc_type_name: 'task',
				kb_context_id: CTX,
				owner_profile_id: 'p-someone-else',
				is_active: true,
			});
		});
		const data = await run({ contexts: [{ id: CTX, can_write: true }] });
		expect(data.mayChange).toBe(true);
		expect(data.stateVocabulary).toBeNull();
	});
});

/**
 * The change itself.
 *
 * The API is the authority gate and the vocabulary gate — `can_modify_resource` answers 403
 * before anything lands, `ManagedMeta`'s `deny_unknown_fields` refuses a key that is not a
 * managed state, frontmatter validation refuses a value outside the field's enum, and the
 * shared applicability gate refuses a state this kind of work does not carry. What is asserted
 * here is what the SURFACE owes: that it sends only what was shown, and that a refusal reaches
 * the reader instead of the page.
 */
const submit = (field: unknown, value: unknown) =>
	(actions.changeState as (e: unknown) => Promise<unknown>)({
		locals: { accessToken: 'tok' },
		params: { ident: 'r1' },
		request: {
			formData: async () =>
				new Map([
					['field', field],
					['value', value],
				]) as unknown as FormData,
		},
	});

describe('changing a state the system defines', () => {
	it('sends exactly the one field asked for, and nothing else', async () => {
		// `no-write-lands-outside-what-was-shown`. `managed_meta` is a partial merge, so a
		// payload carrying a second key would silently restate it — and PATCH would accept that.
		apiPatch.mockResolvedValue({ id: 'r1' });
		await submit('temper-stage', 'done');
		expect(apiPatch).toHaveBeenCalledTimes(1);
		const [path, , body] = apiPatch.mock.calls[0];
		expect(path).toBe('/api/resources/r1');
		expect(body).toEqual({ managed_meta: { 'temper-stage': 'done' } });
	});

	it('hands a refusal back to the reader rather than throwing the page away', async () => {
		const err = new ApiErrorStub('this kind of work does not carry that state');
		err.status = 400;
		apiPatch.mockRejectedValue(err);
		const result = (await submit('temper-stage', 'nonsense')) as {
			status: number;
			data: { field: string; message: string };
		};
		expect(result.status).toBe(400);
		expect(result.data).toEqual({
			field: 'temper-stage',
			message: 'this kind of work does not carry that state',
		});
	});

	it('writes nothing when the form carries nothing to change', async () => {
		apiPatch.mockResolvedValue({ id: 'r1' });
		const result = (await submit('temper-stage', '')) as { status: number };
		expect(result.status).toBe(400);
		expect(apiPatch).not.toHaveBeenCalled();
	});

	it('no reading act becomes a changing one: the load writes nothing', async () => {
		// The clause fails silently when it fails, so it is worth an assertion rather than an
		// argument. Every load path here — including the one that reads the vocabulary — must
		// leave the write door untouched.
		apiPatch.mockResolvedValue({ id: 'r1' });
		await run({ contexts: [{ id: CTX, can_write: true }] });
		expect(apiPatch).not.toHaveBeenCalled();
	});
});

/**
 * Attaching and revising a description the system has no field for.
 *
 * The other arm of the act, and the register rejects the equivalence that would make it the
 * same one. These assertions are what make the two visibly different: this arm travels on the
 * open tier, keeps the type it had, and carries the one rule the surface applies itself.
 */
/**
 * Both description actions now read the resource at action time, because what the surface
 * offered is a property of the resource and the form's `name` is browser-supplied. `stored`
 * sets what that read answers.
 */
const storedTiers = (
	open: Record<string, unknown> = { owner: 'Pete', priority: 3 },
	managed: Record<string, unknown> = { 'temper-stage': 'design' },
) => {
	apiGet.mockImplementation((path: string) => {
		if (path.endsWith('/content')) return new Promise(() => {});
		if (path.includes('/api/schema/doc-types/')) return Promise.resolve({ enum_fields: {} });
		return Promise.resolve({
			id: 'r1',
			title: 'A resource',
			doc_type_name: 'task',
			kb_context_id: CTX,
			owner_profile_id: 'p-someone-else',
			is_active: true,
			open_meta: open,
			managed_meta: managed,
		});
	});
};

const describeIt = (action: string, fields: Record<string, unknown>) =>
	(actions[action] as (e: unknown) => Promise<unknown>)({
		locals: { accessToken: 'tok' },
		params: { ident: 'r1' },
		request: {
			formData: async () => new Map(Object.entries(fields)) as unknown as FormData,
		},
	});

describe('attaching and revising a description', () => {
	it('revises one description and lands outside nothing else', async () => {
		storedTiers();
		apiPatch.mockResolvedValue({ id: 'r1' });
		await describeIt('changeDescription', { name: 'owner', value: 'Pete' });
		const [path, , body] = apiPatch.mock.calls[0];
		expect(path).toBe('/api/resources/r1');
		// `open_meta` merges at the key level, so one key is one description. Nothing else moves.
		expect(body).toEqual({ open_meta: { owner: 'Pete' } });
	});

	it('keeps the type a description already had', async () => {
		// THE BITE, and it is invisible without the assertion: a form submits text, so a
		// revision of `priority: 3` to `4` would store `"4"` — a change nobody asked for that
		// renders identically in the table and that every downstream consumer sees.
		storedTiers();
		apiPatch.mockResolvedValue({ id: 'r1' });
		await describeIt('changeDescription', { name: 'priority', value: '4' });
		expect(apiPatch.mock.calls[0][2]).toEqual({ open_meta: { priority: 4 } });
	});

	it('attaches a new description as text', async () => {
		storedTiers();
		apiPatch.mockResolvedValue({ id: 'r1' });
		await describeIt('attachDescription', { name: 'reviewer', value: 'qa' });
		expect(apiPatch.mock.calls[0][2]).toEqual({ open_meta: { reviewer: 'qa' } });
	});

	it('declines to attach a description into the namespace the system owns', async () => {
		// THE BITE for the rejected equivalence arriving through the back door. The open tier is
		// carried verbatim into the same flat store the managed tier lands in, and the read path
		// sorts them apart BY NAME — so `temper-stage` attached as a description comes back as
		// the task's stage, set to a value that never met the vocabulary check the state arm
		// exists to enforce. No door refuses this today, at any surface.
		storedTiers();
		apiPatch.mockResolvedValue({ id: 'r1' });
		const result = (await describeIt('attachDescription', {
			name: 'temper-stage',
			value: 'whatever-i-like',
		})) as { status: number; data: { message: string } };
		expect(result.status).toBe(400);
		expect(result.data.message).toContain('temper-');
		expect(apiPatch).not.toHaveBeenCalled();
	});

	it('writes nothing when a description has no name or no value', async () => {
		storedTiers();
		apiPatch.mockResolvedValue({ id: 'r1' });
		expect(
			((await describeIt('attachDescription', { name: '  ', value: 'x' })) as { status: number })
				.status,
		).toBe(400);
		expect(
			((await describeIt('attachDescription', { name: 'x', value: '' })) as { status: number })
				.status,
		).toBe(400);
		expect(apiPatch).not.toHaveBeenCalled();
	});

	it('hands a refusal back rather than throwing the page away', async () => {
		storedTiers({ descriptor: 'a descriptor' });
		const err = new ApiErrorStub('invalid open_meta shape: descriptor: expected string');
		err.status = 400;
		apiPatch.mockRejectedValue(err);
		const result = (await describeIt('changeDescription', {
			name: 'descriptor',
			value: 'x',
		})) as { status: number; data: { field: string; message: string } };
		expect(result.status).toBe(400);
		expect(result.data.field).toBe('descriptor');
		expect(result.data.message).toContain('invalid open_meta shape');
	});
});

/**
 * What the description actions refuse — every one of these found by adversarial review of the
 * arm above, and every one reachable before the refusal existed.
 *
 * The shared shape of the class: `open_meta` is carried VERBATIM into a tierless property store
 * and the read path sorts the tiers apart BY KEY NAME, so which key a description is given
 * decides what it becomes. No door refuses any of this server-side, at any surface; these are
 * the half a surface can hold.
 */
describe('a description cannot be given a name that is not a description', () => {
	it('declines `doc_type`, which is the resource’s kind and not a description of it', async () => {
		// THE BITE, and it needed no forgery — a reader types `doc_type` into the rendered attach
		// box and presses Attach. Verified end to end against a live stack before the fix: the
		// PATCH returned 200, `doc_type_name` became the typed value, and because an unrecognized
		// kind has no schema, EVERY later managed write to that resource then skipped both the
		// enum check and the applicability gate — an out-of-vocabulary `temper-stage` was
		// accepted with 200. One form submission disarms the gate #769 exists to enforce.
		storedTiers();
		apiPatch.mockResolvedValue({ id: 'r1' });
		const result = (await describeIt('attachDescription', {
			name: 'doc_type',
			value: 'banana',
		})) as { status: number; data: { message: string } };
		expect(result.status).toBe(400);
		expect(result.data.message).toContain('doc_type');
		expect(apiPatch).not.toHaveBeenCalled();
	});

	it('declines `facet`, which the read path merges rather than stores', async () => {
		storedTiers();
		apiPatch.mockResolvedValue({ id: 'r1' });
		expect(
			(
				(await describeIt('attachDescription', { name: 'facet', value: 'x' })) as {
					status: number;
				}
			).status,
		).toBe(400);
		expect(apiPatch).not.toHaveBeenCalled();
	});

	it('declines a managed name on REVISE too, not only on attach', async () => {
		// The rendered page cannot send this — `name` is a hidden input from a row key, and a
		// stored `temper-stage` comes back in the managed tier, which gets no description
		// control. A hand-written same-origin POST is not the rendered page, and the origin CSRF
		// guard makes a submission same-origin, never trustworthy. Before the fix this wrote
		// `{"temper-stage": 3}`, which `ManagedMeta` cannot decode (`stage` is `Option<String>`)
		// — failing the BATCHED read for every list and search page carrying the resource, for
		// every principal who can see it, with no door able to retract it.
		storedTiers();
		apiPatch.mockResolvedValue({ id: 'r1' });
		const result = (await describeIt('changeDescription', {
			name: 'temper-stage',
			value: '3',
		})) as { status: number; data: { message: string } };
		expect(result.status).toBe(400);
		expect(apiPatch).not.toHaveBeenCalled();
	});

	it('revises only a key that IS an editable description right now', async () => {
		storedTiers({ owner: 'Pete', tags: ['a', 'b'] });
		apiPatch.mockResolvedValue({ id: 'r1' });
		// Absent from the resource entirely.
		expect(
			(
				(await describeIt('changeDescription', { name: 'invented', value: 'x' })) as {
					status: number;
				}
			).status,
		).toBe(400);
		// Present, but structured — the table shows it as uneditable, so the action agrees.
		expect(
			(
				(await describeIt('changeDescription', { name: 'tags', value: 'x' })) as {
					status: number;
				}
			).status,
		).toBe(400);
		expect(apiPatch).not.toHaveBeenCalled();
	});

	it('takes the stored type from the resource, not from the submission', async () => {
		// `kind` used to be a hidden field. A caller who picks the key AND its JSON type is the
		// mechanism above; the type now comes from what is stored, so a forged `kind` is inert.
		storedTiers({ priority: 3 });
		apiPatch.mockResolvedValue({ id: 'r1' });
		await describeIt('changeDescription', { name: 'priority', value: '4', kind: 'string' });
		expect(apiPatch.mock.calls[0][2]).toEqual({ open_meta: { priority: 4 } });
	});
});

describe('attach means attach — it never replaces what is already there', () => {
	it('refuses a name already in use rather than overwriting it', async () => {
		// THE BITE, and this codebase has the scar: `--tags docs` on a resource holding six tags
		// wrote a one-element list and destroyed the other five, under a flag whose help read
		// "Add tag". `open_meta` is a REPLACE channel — the write folds the key's whole live set
		// and inserts one value. So attaching `tags` over a list flattens it to a scalar, on
		// exactly the rows the table has just told the reader it cannot edit.
		storedTiers({ owner: 'Pete', tags: ['architecture', 'review', 'backlog'] });
		apiPatch.mockResolvedValue({ id: 'r1' });
		const result = (await describeIt('attachDescription', {
			name: 'tags',
			value: 'design',
		})) as { status: number; data: { message: string } };
		expect(result.status).toBe(409);
		expect(result.data.message).toContain('tags');
		expect(apiPatch).not.toHaveBeenCalled();
	});

	it('refuses a name in use by a STATE as well as by a description', async () => {
		storedTiers({ owner: 'Pete' }, { 'temper-branch': 'jct/x' });
		apiPatch.mockResolvedValue({ id: 'r1' });
		expect(
			(
				(await describeIt('attachDescription', { name: 'temper-branch', value: 'y' })) as {
					status: number;
				}
			).status,
		).toBe(400); // reserved rule fires first — the name never reaches the collision check
		expect(apiPatch).not.toHaveBeenCalled();
	});

	it('still attaches a genuinely new name', async () => {
		// The refusals must not be passing by refusing everything.
		storedTiers({ owner: 'Pete' });
		apiPatch.mockResolvedValue({ id: 'r1' });
		await describeIt('attachDescription', { name: 'reviewer', value: 'qa' });
		expect(apiPatch.mock.calls[0][2]).toEqual({ open_meta: { reviewer: 'qa' } });
	});
});

describe('an unverifiable description write is not attempted', () => {
	it('refuses rather than writing when the resource cannot be re-read', async () => {
		// Both actions check the submission against the resource. If that read fails, what the
		// surface offered is unknown — and an unknown offer is not a licence to write.
		apiGet.mockImplementation((path: string) => {
			if (path.endsWith('/content')) return new Promise(() => {});
			return Promise.reject(new Error('503'));
		});
		apiPatch.mockResolvedValue({ id: 'r1' });
		expect(
			(
				(await describeIt('changeDescription', { name: 'owner', value: 'x' })) as {
					status: number;
				}
			).status,
		).toBe(502);
		expect(
			(
				(await describeIt('attachDescription', { name: 'reviewer', value: 'x' })) as {
					status: number;
				}
			).status,
		).toBe(502);
		expect(apiPatch).not.toHaveBeenCalled();
	});
});
