import { error, fail } from '@sveltejs/kit';
import { mayChangeResource } from '$lib/authority';
import { type EditableKind, revisedValue } from '$lib/descriptions';
import { parseRef } from '$lib/ref';
import { ApiError, apiGet, apiPatch } from '$lib/server/api';
import { bounded } from '$lib/server/bounded';
import { readResourceEdges, readTrail } from '$lib/server/graph-reads';
import type { ContentResponse, DocTypeDescription, ResourceView } from '$lib/types';
import type { Actions, PageServerLoad } from './$types';

/**
 * Which values each of this kind of work's states may take — read, never restated.
 *
 * `GET /api/schema/doc-types/{name}` derives `enum_fields` from the doc-type's own embedded
 * schema, which is the only reason this surface can satisfy *the states offered are the states
 * the work carries* without holding a copy of a vocabulary the system owns.
 *
 * Three outcomes, and the third is why this is not a bare `.catch(() => null)`:
 *
 * - the kind has a vocabulary → its `enum_fields`
 * - **404** → the kind is out of vocabulary (no embedded schema). It carries no states, and
 *   `{}` says so definitively. Live resources sit on such types.
 * - anything else → `null`, meaning *nobody asked and nobody knows*. Offering nothing is the
 *   response to both `{}` and `null`, but they are not the same claim, and a table that
 *   silently offered nothing after a failed read would read as complete when it is not.
 */
async function readStateVocabulary(
	docType: string,
	accessToken: string,
): Promise<Readonly<Record<string, readonly string[]>> | null> {
	try {
		const described = await apiGet<DocTypeDescription>(
			`/api/schema/doc-types/${encodeURIComponent(docType)}`,
			accessToken,
		);
		return described.enum_fields as Readonly<Record<string, readonly string[]>>;
	} catch (err) {
		if (err instanceof ApiError && err.status === 404) return {};
		return null;
	}
}

export const load: PageServerLoad = async ({ locals, params, parent }) => {
	const accessToken = locals.accessToken!;
	const id = parseRef(params.ident);

	// The three fill reads are started HERE, above the one remaining `await`, so none of them
	// waits on the scaffold. `id` comes from `params`, not from `resource`, so nothing about them
	// needs the resource row first.
	//
	// **None of the three is caught into a value any more, and that is the change.** The file used
	// to apply the right principle to one read of three — "an API error must surface as an error,
	// not render as an empty document" — while degrading the other two to `null` and `[]`. Those
	// degradations asserted *there is nothing here*, which is a claim about the reader's material
	// that nothing verified: a failed trail read was indistinguishable from a resource with no
	// history, and a failed edges read from a resource with no connections. Failure now travels to
	// the template, where `{:catch}` names which region failed.
	//
	// `bounded` supplies the OTHER catch (spec §5.3) on each of these: an unawaited promise that
	// rejects with nothing subscribed is an unhandled rejection and takes the server down. It is
	// not the `{:catch}` that renders the failure, and having one does not give you the other.
	// The gap it closes is real in two places here — between these lines and SvelteKit subscribing
	// in order to serialize, and on the 404 path below, which abandons all three.
	const content = bounded(
		apiGet<ContentResponse>(`/api/resources/${id}/content`, accessToken).then((r) => r.markdown),
		'document',
	);
	const trail = bounded(readTrail(accessToken, 'node', id), 'history');
	const edges = bounded(readResourceEdges(accessToken, id), 'connections');

	// GET /api/resources/{id} returns a ResourceView — the one shape, with both meta
	// tiers filled. Do NOT read the tiers off /content: get_content_select hardcodes
	// both to None (substrate_read.rs). They are dead fields.
	//
	// This is the only `await` left, and it is the scaffold: a resource that is not there is a
	// real 404, not a page frame around three failed regions.
	let resource: ResourceView;
	try {
		resource = await apiGet<ResourceView>(`/api/resources/${id}`, accessToken);
	} catch (err) {
		if (err instanceof ApiError && err.status === 404) throw error(404, 'Resource not found');
		throw err;
	}

	// Whether the reader may change this is known BEFORE anything is offered — never
	// discovered by attempting. `contexts` and `profile` are already on the layout's data, so
	// the union costs no read: see `mayChangeResource` for why it is a union the surface
	// computes rather than a field it reads.
	const { contexts, profile } = await parent();
	const mayChange = mayChangeResource(resource, contexts, profile.id);

	// Only asked when there is something to offer. A reader who may not change this resource
	// is shown exactly the table they were shown before this arm existed.
	const stateVocabulary = mayChange
		? await readStateVocabulary(resource.doc_type_name, accessToken)
		: null;

	return { resource, content, trail, edges, mayChange, stateVocabulary };
};

export const actions: Actions = {
	/**
	 * Change one state the system defines, on the reader's own behalf, with no elevation.
	 *
	 * **A POST, and only a POST.** Moving around, opening or looking at a resource must never
	 * change it, so the change has its own named action reached by submitting a form — never a
	 * link, a query parameter, or a select that submits as the reader arrows through it.
	 *
	 * **The authority check is the API's, not this action's.** `can_modify_resource` runs
	 * server-side before anything lands and answers 403; re-deciding it here would be a second
	 * gate that can only disagree with the first. What the surface owes is not offering the
	 * change to a reader who lacks authority — which is `load`'s job, above.
	 *
	 * Likewise the vocabulary: an unknown managed key is refused by `ManagedMeta`'s
	 * `deny_unknown_fields`, a value outside the field's enum by frontmatter validation, and a
	 * state this kind of work does not carry by the shared applicability gate every door
	 * passes through. Restating any of those here is the drift this arm exists to avoid.
	 *
	 * On success nothing is returned: SvelteKit re-runs `load`, so what the reader is shown
	 * afterwards is read back from storage rather than echoed from what they asked for.
	 */
	changeState: async ({ request, locals, params }) => {
		const id = parseRef(params.ident);
		const form = await request.formData();
		const field = form.get('field');
		const value = form.get('value');

		// Shape only — that these are strings at all. What they MEAN is the server's to judge.
		if (typeof field !== 'string' || !field || typeof value !== 'string' || !value) {
			return fail(400, { field: null, message: 'Nothing to change.' });
		}

		try {
			await apiPatch<ResourceView>(`/api/resources/${id}`, locals.accessToken!, {
				managed_meta: { [field]: value },
			});
		} catch (err) {
			if (err instanceof ApiError) {
				return fail(err.status, { field, message: err.message });
			}
			throw err;
		}
		return { field };
	},

	/**
	 * Revise a description the reader attached — free text, and deliberately so.
	 *
	 * This is the **other** act, and the register it answers to rejects the equivalence that
	 * would make it the same one: *a state the system defines and a description the reader
	 * invented are not the same act because they are the same storage.* There is no vocabulary
	 * to check this against, so it is sent and the server's answer is what the reader sees —
	 * which is why it is a separate action rather than a mode on the one above.
	 *
	 * `open_meta` merges at the KEY level: unsupplied keys are untouched, so this revises one
	 * description and lands outside nothing that was shown. (The additive `open_meta_add`
	 * channel is for list union and is not what a single-value revision means.)
	 *
	 * `kind` keeps the stored type — see `revisedValue`. It comes from the browser, which can
	 * only use it to give the reader's own description a type they chose.
	 */
	changeDescription: async ({ request, locals, params }) => {
		const id = parseRef(params.ident);
		const form = await request.formData();
		const name = form.get('name');
		const value = form.get('value');
		const kind = form.get('kind');

		if (typeof name !== 'string' || !name || typeof value !== 'string') {
			return fail(400, { field: null, message: 'Nothing to change.' });
		}
		const asKind: EditableKind =
			kind === 'number' || kind === 'boolean' || kind === 'string' ? kind : 'string';

		try {
			await apiPatch<ResourceView>(`/api/resources/${id}`, locals.accessToken!, {
				open_meta: { [name]: revisedValue(value, asKind) },
			});
		} catch (err) {
			if (err instanceof ApiError) return fail(err.status, { field: name, message: err.message });
			throw err;
		}
		return { field: name };
	},

	/**
	 * Attach a description the system has no field for.
	 *
	 * A new description is text: nothing on this surface lets the reader say otherwise, so
	 * nothing here guesses a type for them.
	 *
	 * **A `temper-`prefixed name is declined, and this is the one rule the surface applies
	 * itself.** The open tier is carried verbatim into the same flat property store the managed
	 * tier lands in, and the read path sorts them apart by name — so a description called
	 * `temper-stage` would come back as the task's STAGE, set to a value that never passed the
	 * vocabulary check the state arm exists to enforce. That is the rejected equivalence
	 * arriving through the back door. No door refuses it today, at any surface; declining to
	 * author into a namespace the system owns is the conservative half this one can do without
	 * restating which names are managed — which it deliberately no longer knows.
	 *
	 * It over-declines: `temper-invented` is an ordinary open key the system would accept. That
	 * is the safe direction, and it is a named limit rather than a silent one.
	 */
	attachDescription: async ({ request, locals, params }) => {
		const id = parseRef(params.ident);
		const form = await request.formData();
		const name = form.get('name');
		const value = form.get('value');

		// `field: ''` addresses the attach form rather than any row — `null` is the whole table.
		if (typeof name !== 'string' || !name.trim() || typeof value !== 'string' || !value) {
			return fail(400, { field: '', message: 'A description needs a name and a value.' });
		}
		const trimmed = name.trim();
		if (trimmed.startsWith('temper-')) {
			return fail(400, {
				field: '',
				message: `"${trimmed}" is a name the system owns. Descriptions you attach cannot start with "temper-".`,
			});
		}

		try {
			await apiPatch<ResourceView>(`/api/resources/${id}`, locals.accessToken!, {
				open_meta: { [trimmed]: value },
			});
		} catch (err) {
			if (err instanceof ApiError) return fail(err.status, { field: '', message: err.message });
			throw err;
		}
		return { field: trimmed };
	},
};
