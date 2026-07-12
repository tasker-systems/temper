# Cognitive-map & resource authoring authorization

This is the **per-resource** authorization axis: given an authenticated caller, *what
may they author?* It is distinct from the [authorization seam](authorization-seam.md),
which answers the prior question — *may they use Temper at all?* (authenticate → system
access). A caller passes the seam first; then every write is gated again, per target,
by the predicates below.

## The one thing to remember

**Authoring is an explicit capability, never obscurity.** A cognitive map has **no
owner column** — since the Q-A flip (`20260701000001_cogmap_write_tightening.sql`),
authorship is *wholly* an explicit write grant. Team membership grants **read**, not
write. Knowing a cogmap id gets you nothing: without a `kb_access_grants` write row (or
being a resource's own owner/originator) every authoring call is denied.

> Verified live 2026-07-06: a principal with **read** on the team map `019f2391` (via
> membership) but no write grant was denied `fold_relationship` ("cannot modify this
> resource") and could not self-grant (`cogmap_grant` → `granted:false`). The only
> write-holder is the steward's granted M2M principal.

## The three predicates

All three are SQL functions resolved against the connection search_path; surfaces and
`DbBackend` call them, never inlining the SQL.

| Predicate | Question | Definition |
|---|---|---|
| `cogmap_authorable_by_profile(profile, cogmap)` | May `profile` author *into* this map? | `profile_explicit_grant(profile, 'write', 'kb_cogmaps', cogmap)` — explicit write grant only (cogmaps have no owner) |
| `context_authorable_by_profile(profile, context)` | May `profile` author *into* this context? | personal-owner, reachable-member-of-owning-team, **or** an explicit `can_write` grant |
| `can_modify_resource(profile, resource)` | May `profile` modify this *existing* resource? | resource's home `owner`/`originator`, an explicit per-resource (`kb_access_grants`, `can_write`) grant, **or** write on the resource's home **container** (`cogmap_authorable_by_profile` / `context_authorable_by_profile`) — the **container-write cascade** |
| `anchor_readable_by_profile(profile, 'kb_cogmaps', cogmap)` | May `profile` *read* this map? | membership-broad read visibility |

`can_modify_resource` consults the resource's own ownership/grants **and** its home
container's write capability (the cascade, below). An existing cogmap node is therefore
modifiable by its originator, by any co-author who holds write on the map, or via an
explicit per-resource grant — but not by an arbitrary map *reader*.

## The container-write cascade

**Whoever may author a container may modify any node homed in it** — unix directory
semantics: directory-write ⇒ file-rwx. A cogmap co-author (holder of
`cogmap_authorable_by_profile`) or a context writer (`context_authorable_by_profile`)
can create nodes *and* `fold`/`facet`/`assert`-from/`update` nodes another principal
originated, without a per-resource grant. This is deliberate collaborative stewardship:
gating node-modify on node-ownership alone was illusory anyway, since a container-writer
could already supersede a node by fold-then-recreate. Provenance is unaffected — the
event ledger records the actual actor on every mutation, so "co-author B folded A's node"
reads truthfully.

`context_authorable_by_profile`'s **team-owner** arm (a member of the owning team may
author a team-owned context) is deliberate and is **not** the pre-Q-A "membership implies
write". Q-A removed write for teams merely *joined-for-read* to a cogmap; *owning* a
context is a strictly stronger relationship. (Spec:
`docs/superpowers/specs/2026-07-06-container-write-cascade-and-authz-hardening-design.md`.)

## The gate map — every authoring op

| Operation | Gate | Enforced in |
|---|---|---|
| `create_resource` into a cogmap | `cogmap_authorable_by_profile` (write grant) | **`DbBackend::create_resource`** (F1) + the surfaces (MCP `create_resource` tool + HTTP `ingest`) as fast-fail pre-checks, via the shared `cogmap_service::authorable_by_profile` seam |
| `assert_relationship` | `can_modify_resource(source)` — incl. container cascade | `DbBackend::check_can_modify_next` |
| `fold_relationship` / `retype` / `reweight` | `can_modify_resource(source)` — incl. container cascade | `DbBackend::check_can_modify_next` |
| `facet_set` | `can_modify_resource(resource)` — incl. container cascade | `DbBackend::check_can_modify_next` |
| `update_resource` / `delete_resource` | `can_modify_resource(resource)` — incl. container cascade | `DbBackend::check_can_modify_next` |
| `advance_steward_watermark` | `cogmap_authorable_by_profile` (write grant) | `DbBackend` |
| `materialize` / `materialize_delta` | `cogmap_authorable_by_profile` (write grant) | `DbBackend` |
| `invocation_open` (self-attributed, `parent` = None) | `cogmap_authorable_by_profile` — **WRITE** (F2) | `DbBackend::check_cogmap_authorable` |
| `invocation_open` (delegated, `parent` = Some) | `anchor_readable_by_profile` — **READ**; substrate enforces parent→originating lineage | `DbBackend::check_can_read_cogmap` |

Note the two homes: **content writes on existing resources** gate inside `DbBackend`
(`can_modify_resource`, which now cascades from container write); **create-into-cogmap**
also gates inside `DbBackend` (F1) — the surfaces keep a matching pre-check for fast-fail
and clearer error text, but the shared write path is the authoritative gate, so a new
caller cannot bypass it.

## Cross-surface uniformity

The create-into-cogmap gate is defined **once** in `cogmap_service::authorable_by_profile`
and called by both the MCP tool and the HTTP ingest handler — so MCP, CLI (which routes
`resource create` → `POST /api/ingest`), and API all enforce the same rule. `DbBackend`
is the single write path for every other op, so those gates are surface-uniform by
construction.

## Agent vs human principals

Agents are gated **identically** to humans. The steward is an M2M principal
(`client_credentials`); it authors `019f2391` because it holds an **explicit write
grant**, exactly the mechanism a granted human would use — *not* by virtue of any team
membership, since membership confers **read** only (the Q-A flip). There is no
agent-specific bypass and no ambient authority: every call resolves to one concrete
`profile_id`, and every gate evaluates that id.

An agent profile **can** hold team memberships — registration takes `--team <ref>[:role]`
(repeatable) and also enrolls the machine in the gating team as `watcher`, so it clears the
`system_access` gate. That is the point of the design: a machine's reach is ordinary teams
and ordinary grants, bounded to what its minter could confer on a human, so **machine RBAC
falls out of the same predicates as human RBAC** — there is no machine-specific
authorization path to keep in sync. What team membership still does *not* buy an agent, any
more than it buys a human, is cogmap write. See
[../guides/machine-credentials.md](../guides/machine-credentials.md) and the
[machine-token contract](machine-token-contract.md).

When a human drives an agent (e.g. an AI assistant over the human's authenticated MCP
session), the session authenticates as the **human's** principal — there is no separate
"assistant" principal. The assistant's tool calls carry the human's rights, no more.

## Hardening — resolved

The three findings once tracked here are shipped (none was ever a live open door; all were
defense-in-depth / clarity). Recorded for provenance.

### F1 — create-into-cogmap authz moved onto the shared write path ✅

`DbBackend::create_resource` now checks `cogmap_authorable_by_profile` on a `Cogmap` home
(`check_cogmap_authorable`), before any write — so the shared write path denies even a
caller that skipped a surface pre-check (the SAML `is_active` failure mode this directory
exists to prevent). The surfaces keep the pre-check for fast-fail + clearer error text.

### F2 — `invocation_open` is write-gated for self-attributed opens ✅

A self-attributed open (`parent` = None) now requires `cogmap_authorable_by_profile`
(WRITE) — claiming a ledger slot under one's own name is an authoring act, closing the
reader-posts-inert-envelopes noise vector. A delegated open (`parent` = Some) keeps the
READ gate, with the substrate's parent→originating lineage as the control for delegated
sub-agents. In production the only self-attributed opener is the steward, which holds
write, so no real caller regressed.

### F3 — stale "team-cogmap membership" comments corrected ✅

The three seam comments (`temper-mcp/.../resources.rs`, `temper-api/.../ingest.rs`,
`cogmap_service.rs`) now name `cogmap_authorable_by_profile` = an explicit *write grant*
(membership confers read only, per Q-A), and note the F1 backend re-enforcement.
