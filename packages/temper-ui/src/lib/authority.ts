import type { ContextRowWithCounts, ResourceView } from '$lib/types';

/**
 * Whether this reader may change this resource — computed **before** anything is offered,
 * never discovered by attempting.
 *
 * ### Why this is a union the surface computes rather than a field it reads
 *
 * `ResourceView` carries no `can_write`. Authority lives on the **container** read:
 * `ContextView.can_write` (`crates/temper-core/src/types/context.rs`) answers
 * `context_authorable_by_profile`, which is the container-write cascade arm of
 * `can_modify_resource`. Because that arm keys on the *home* rather than the resource, one
 * boolean answers the cascade for every resource homed there — which is why it rides the
 * context read instead of every response that carries a resource.
 *
 * That field's own doc comment says plainly what it is not:
 *
 * > **Not the whole write gate.** `can_modify_resource` also admits the resource's home owner
 * > and explicit per-resource grants. A surface deriving offerability from this must union it
 * > with the owner check (`ResourceView.owner_profile_id`) under the `is_active` floor, and
 * > accepts that a reader whose only authority is a per-resource grant is not covered.
 *
 * So: the `is_active` floor, then owner OR container. Nothing here derives authority from
 * *reach*. Reading a resource is strictly broader than authoring it — a watcher and a
 * read-only grant both reach a context they cannot write — and deriving the second from the
 * first is the fail-open shape this whole arm exists to prevent.
 *
 * ### The accepted false negative
 *
 * A reader whose **only** authority is an explicit per-resource grant answers `false` here and
 * is offered nothing, though the write gate would admit them. That is a deliberate,
 * already-recorded limit of the container-shaped answer, not a hole to route around: erring
 * toward offering less is the safe direction, and a reader in that position can still change
 * the resource through another door.
 *
 * ### Unknown is not permitted
 *
 * `contexts` is `null` when the context read failed — which is a claim the layout deliberately
 * keeps distinct from `[]`. A failed read tells us nothing about the container arm, so the
 * container arm contributes nothing. The owner arm is independent and still decides on its
 * own; that is the point of a union.
 */
export function mayChangeResource(
	resource: Pick<ResourceView, 'kb_context_id' | 'owner_profile_id' | 'is_active'>,
	contexts: readonly ContextRowWithCounts[] | null,
	viewerProfileId: string,
): boolean {
	// The floor. A soft-deleted resource is changed by nobody, whatever else holds.
	if (!resource.is_active) return false;

	// Owner arm — independent of the container, and answerable with no second read.
	if (resource.owner_profile_id === viewerProfileId) return true;

	// Container arm. A resource homed in a cognitive map has no `kb_context_id`; this arm
	// cannot speak to it and does not pretend to.
	if (resource.kb_context_id === null) return false;
	const home = contexts?.find((c) => c.id === resource.kb_context_id);
	return home?.can_write === true;
}
