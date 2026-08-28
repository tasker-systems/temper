# Authoring Authorization

**For integrators and users** — anyone who needs to know what decides whether a write to
Temper is permitted. This is the per-resource authorization axis; the prior question (may you
use Temper at all?) is answered in [The Trust Boundary](./trust-boundary.md).

## Authoring is an explicit capability, never obscurity

A cognitive map has **no owner column** — authorship is wholly an explicit write grant. Team
membership grants **read**, not write. Knowing a map's id gets you nothing: without an explicit
write grant, every authoring call is denied.

This is deliberate. Gating writes on obscurity (a private id, a team membership) is illusory
when container-write already lets a co-author supersede a node by fold-then-recreate. The
honest model gates on the explicit grant and records the actual actor on every mutation.

## The three predicates

| Predicate | Question |
|---|---|
| `cogmap_authorable_by_profile` | May this profile author *into* this map? — explicit write grant only |
| `context_authorable_by_profile` | May this profile author *into* this context? — owner, reachable team member, or explicit grant |
| `can_modify_resource` | May this profile modify this *existing* resource? — owner, explicit grant, or container-write cascade |

## Ownership is not a grant from a team

`kb_resource_homes.owner_profile_id` is one profile, and it is the only access-bearing profile
key — `originator_profile_id` is recorded provenance and confers nothing. Both gates read the
owner directly: neither team membership nor a context share enters that arm. Ownership is not a
grant, so nothing revokes it in place.

The consequence is deliberate. **Withdrawing a share withdraws only what the share conferred.**
Removing someone from a team, or unsharing a context, does not move ownership — so anything they
still own keeps them as its owner, and therefore as its reader and writer. Ownership moves only by
an act that names a successor: [`temper resource reassign`](../reference/cli/resource.md) for one
resource, [`temper team reassign`](../reference/cli/team.md) for everything one person owns in a
team's shared contexts.

So a departure **surfaces** what is still owned rather than sweeping it. `temper team remove-member`
reports the removed member's residual, computed by the same query the handoff moves, so the report
and the handoff cannot disagree. What actually ends a departing person's reach is the prior
question — admission — and it is answered before any predicate on this page runs: see
[The Trust Boundary](./trust-boundary.md), and
[Offboard a Departure](../playbooks/offboard-a-departure.md) for the sequence an operator runs.

## The container-write cascade

**Whoever may author a container may modify any node homed in it** — unix directory semantics:
directory-write implies file-rwx. A cogmap co-author can create nodes *and* fold, retype,
reweight, or update nodes another principal originated, without a per-resource grant. This is
deliberate collaborative stewardship. Provenance is unaffected — the event ledger records the
actual actor on every mutation, so "co-author B folded A's node" reads truthfully.

## Agent vs. human principals

Agents are gated **identically** to humans. An agent authors because it holds an explicit write
grant — the same mechanism a granted human would use — not by virtue of any team membership or
ambient authority. There is no agent-specific bypass. Every call resolves to one concrete
profile, and every gate evaluates that profile.

When a human drives an agent (an AI assistant over the human's authenticated session), the
session authenticates as the **human's** principal — there is no separate "assistant"
principal. The assistant's tool calls carry the human's rights, no more.

## Further reading

- **The trust boundary that gates admission before authorization:**
  [The Trust Boundary](./trust-boundary.md).
- **What a cognitive map is:**
  [temperkb.io/cognitive-maps/what-a-cognitive-map-is](https://temperkb.io/cognitive-maps/what-a-cognitive-map-is).
- **How maps grow (the collaborative model):**
  [temperkb.io/cognitive-maps/how-a-map-grows](https://temperkb.io/cognitive-maps/how-a-map-grows).
