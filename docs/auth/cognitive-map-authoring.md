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
| `cogmap_authorable_by_profile(profile, cogmap)` | May `profile` author *into* this map? | `profile_explicit_grant(profile, 'write', 'kb_cogmaps', cogmap)` — explicit write grant only |
| `can_modify_resource(profile, resource)` | May `profile` modify this *existing* resource? | resource's home `owner`/`originator` **or** an explicit per-resource (`kb_access_grants`, `can_write`) grant, profile- or reachable-team-anchored |
| `anchor_readable_by_profile(profile, 'kb_cogmaps', cogmap)` | May `profile` *read* this map? | membership-broad read visibility |

`can_modify_resource` does **not** consult cogmap-authorship: it gates on the *resource's*
own ownership/grants. This is why an existing cogmap node is modifiable by its
originator (the authoring agent) but not by an arbitrary map reader.

## The gate map — every authoring op

| Operation | Gate | Enforced in |
|---|---|---|
| `create_resource` into a cogmap | `cogmap_authorable_by_profile` (write grant) | **Surface** — MCP `create_resource` tool + HTTP `ingest` handler, via the shared `cogmap_service::authorable_by_profile` seam ⚠️ see F1 |
| `assert_relationship` | `can_modify_resource(source)` | `DbBackend::check_can_modify_next` |
| `fold_relationship` / `retype` / `reweight` | `can_modify_resource(source)` | `DbBackend::check_can_modify_next` |
| `facet_set` | `can_modify_resource(resource)` | `DbBackend::check_can_modify_next` |
| `update_resource` / `delete_resource` | `can_modify_resource(resource)` | `DbBackend::check_can_modify_next` |
| `advance_steward_watermark` | `cogmap_authorable_by_profile` (write grant) | `DbBackend` |
| `materialize` / `materialize_delta` | `cogmap_authorable_by_profile` (write grant) | `DbBackend` |
| `invocation_open` | `anchor_readable_by_profile` — **READ only** | `DbBackend::check_can_read_cogmap` ⚠️ see F2 |

Note the two homes: **content writes on existing resources** gate inside `DbBackend`
(`can_modify_resource`); **create-into-cogmap** gates at the *surface* (the resource does
not exist yet, so `can_modify_resource(new_id)` cannot apply — the home cogmap is checked
instead). That split is the source of finding F1.

## Cross-surface uniformity

The create-into-cogmap gate is defined **once** in `cogmap_service::authorable_by_profile`
and called by both the MCP tool and the HTTP ingest handler — so MCP, CLI (which routes
`resource create` → `POST /api/ingest`), and API all enforce the same rule. `DbBackend`
is the single write path for every other op, so those gates are surface-uniform by
construction.

## Agent vs human principals

Agents are gated **identically** to humans. The steward is an M2M principal
(`client_credentials`); it authors `019f2391` because it holds an **explicit write
grant**, exactly the mechanism a granted human would use — *not* via team membership
(agents belong to no team). There is no agent-specific bypass and no ambient authority:
every call resolves to one concrete `profile_id`, and every gate evaluates that id.

When a human drives an agent (e.g. an AI assistant over the human's authenticated MCP
session), the session authenticates as the **human's** principal — there is no separate
"assistant" principal. The assistant's tool calls carry the human's rights, no more.

## Known hardening gaps

Tracked as a single security-hardening task (see the goal/task index). None is a live
open door; all three are defense-in-depth / clarity.

### F1 — create-into-cogmap authz is surface-side, not backend-side

`DbBackend::create_resource` does **not** check `cogmap_authorable_by_profile` on its
`Cogmap` home — it trusts each surface to have pre-checked. Both current surfaces do, so
there is **no live bypass**. But this is precisely the failure mode this directory exists
to prevent (cf. the SAML `is_active` gate that lived on `temper-api` only and missed MCP
until review): a gate that lives on the surfaces rather than the shared write path is one
new caller away from a silent miss. **Fix:** add the `cogmap_authorable_by_profile` check
inside `DbBackend::create_resource` when `cmd.home` is a `Cogmap` (belt-and-suspenders;
keep the surface pre-checks for fast-fail + better error text).

### F2 — `invocation_open` is read-gated, not write-gated

Opening an invocation envelope requires only `anchor_readable_by_profile` (READ). A map
*reader* can therefore open (empty, inert) envelopes on a map they cannot author — no
content lands and the watermark cannot move, so it is low-severity, but it permits
self-attributed noise on the accountability ledger. **Decision needed:** is read-to-open
intentional (the delegated sub-agent model relies on the substrate's parent→originating
delegation gate), or should a non-delegated open require write? At minimum, document the
intent at the gate.

### F3 — stale comments assert a weaker, pre-Q-A model

Three sites describe the create gate as "team-cogmap membership":
`temper-mcp/.../resources.rs` (~L422), `temper-api/.../ingest.rs` (~L43), and
`cogmap_service.rs` (~L62). The gate is `cogmap_authorable_by_profile` = explicit *write
grant*; membership-implies-write was removed by Q-A. A comment asserting the weaker model
invites a future "simplification" back toward it. **Fix:** correct the comments to name
the explicit-grant predicate.
