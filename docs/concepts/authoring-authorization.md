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
| `can_modify_resource` | May this profile modify this *existing* resource? — owner/originator, explicit grant, or container-write cascade |

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
