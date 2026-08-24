import { error, fail } from '@sveltejs/kit';
import { mayChangeResource } from '$lib/authority';
import { editableKind, revisedValue } from '$lib/descriptions';
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

/**
 * Property keys the read path does **not** return as a description, and which this surface must
 * therefore never write as one.
 *
 * `open_meta` is carried verbatim into a tierless `kb_properties` store and the read path sorts
 * the tiers apart **by key name** (`temper-substrate/src/readback/mod.rs`). Three names are peeled
 * out of that stream before anything becomes a description:
 *
 * - **`doc_type`** — the resource's authoritative KIND, surfaced as `ResourceView.doc_type_name`.
 *   Its only intended writer is a type move (`MoveSpec.type_to`). Written as a description it
 *   retypes the resource with no validation, and every later managed write to it then skips the
 *   enum check and the applicability gate, because an unrecognized kind has no schema to enforce.
 * - **`facet`** — merged per inner key into the facet slot, not stored as a plain value.
 * - **`temper-*`** — sorted into the managed tier by `is_managed_property_key`, so a description
 *   named `temper-stage` comes back as the task's *stage*, holding a value that never met the
 *   vocabulary check the state arm exists to enforce.
 *
 * **No door refuses any of this**, at any surface — `properties_from_meta` filters the managed
 * tier through `key_fate` and the open tier not at all. That is a server-side hole, filed
 * separately. This is the half a surface can hold without restating a vocabulary it does not own:
 * a description is a thing the read path gives back as a description, and these are not.
 *
 * The `temper-` rule over-declines — `temper-invented` is an ordinary open key the system accepts.
 * That is the safe direction, and it is a named limit rather than a silent one.
 */
function reservedName(name: string): string | null {
	if (name.startsWith('temper-')) {
		return `"${name}" is a name the system owns — descriptions cannot start with "temper-".`;
	}
	if (name === 'doc_type') {
		return '"doc_type" is this resource\'s kind, not a description of it.';
	}
	if (name === 'facet') {
		return '"facet" is a name the system owns.';
	}
	return null;
}

/**
 * The resource's descriptions as they stand **right now**, read at action time.
 *
 * Both description actions need this and neither can take it from the form. The `name` a
 * submission carries is browser-supplied — a hidden input on the revise control, a text box on
 * attach — so trusting it means trusting the client about which key it is allowed to write. What
 * the surface actually offered is a property of the resource, and only the resource can answer it.
 *
 * Returns `null` when the read fails, which both callers turn into a refusal: an unverifiable
 * write is not attempted.
 */
async function currentDescriptions(
	id: string,
	accessToken: string,
): Promise<{ open: Record<string, unknown>; managed: Record<string, unknown> } | null> {
	try {
		const view = await apiGet<ResourceView>(`/api/resources/${id}`, accessToken);
		return {
			open: (view.open_meta as Record<string, unknown> | null) ?? {},
			managed: (view.managed_meta as unknown as Record<string, unknown> | null) ?? {},
		};
	} catch {
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
	 * **It revises only a key that IS currently an editable description**, checked against the
	 * resource rather than against the form. `name` arrives in a hidden input, so the rendered
	 * page can only ever send a key it rendered — but a hand-written same-origin POST is not the
	 * rendered page, and the same-origin CSRF guard does not make a submission trustworthy, only
	 * same-origin. Without this check that POST could write any key at all, including a managed
	 * one at a type its field cannot decode, which fails the batched read for **every** page that
	 * lists the resource, for every principal who can see it, with no door able to retract it.
	 *
	 * **The stored type comes from the stored value, not from the browser.** It used to travel as
	 * a hidden `kind` field; a caller who picks both the key and its JSON type is the mechanism
	 * above. `editableKind` re-derives it here, so the type a revision keeps is the type the
	 * resource actually holds.
	 */
	changeDescription: async ({ request, locals, params }) => {
		const id = parseRef(params.ident);
		const form = await request.formData();
		const name = form.get('name');
		const value = form.get('value');

		if (typeof name !== 'string' || !name || typeof value !== 'string') {
			return fail(400, { field: null, message: 'Nothing to change.' });
		}

		const stored = await currentDescriptions(id, locals.accessToken!);
		if (!stored) {
			return fail(502, { field: name, message: 'Could not read this resource to change it.' });
		}
		const kind = editableKind(stored.open[name]);
		if (kind === null) {
			// Covers every not-a-revisable-description case in one refusal: the key is absent, it
			// is a state rather than a description, or its value is structured — which the table
			// already shows as uneditable.
			return fail(400, {
				field: name,
				message: `"${name}" is not a description that can be changed here.`,
			});
		}

		try {
			await apiPatch<ResourceView>(`/api/resources/${id}`, locals.accessToken!, {
				open_meta: { [name]: revisedValue(value, kind) },
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
	 * Two rules, both applied against the resource rather than the form — see `reservedName` and
	 * `currentDescriptions`.
	 *
	 * **A reserved name is declined**: `doc_type`, `facet`, and anything `temper-`prefixed are
	 * keys the read path does not give back as descriptions.
	 *
	 * **A name already in use is declined, and this one is about data.** `open_meta` is a
	 * REPLACE-shaped channel: the write folds the key's whole live set and inserts one value. So
	 * attaching a name that already exists is not an attach at all — it overwrites. On a
	 * list-valued description that means an N-member list becomes one scalar, silently, on the
	 * very rows this table has just told the reader it cannot edit. This codebase has the scar
	 * already: `--tags docs` on a resource holding six tags wrote a one-element list and
	 * destroyed the other five, under a flag whose help read *"Add tag"*. Attach means attach;
	 * changing what is there is the revise control, beside the row.
	 *
	 * Both refusals name the offending key, because "that name is taken" is only actionable if
	 * the reader can see which one.
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

		const reserved = reservedName(trimmed);
		if (reserved) return fail(400, { field: '', message: reserved });

		const stored = await currentDescriptions(id, locals.accessToken!);
		if (!stored) {
			return fail(502, { field: '', message: 'Could not read this resource to add to it.' });
		}
		if (trimmed in stored.open || trimmed in stored.managed) {
			return fail(409, {
				field: '',
				message: `"${trimmed}" is already on this resource. Change it where it is shown above rather than attaching it again — attaching would replace what is there.`,
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
