# Admin-authz enclosure — a sealed `SystemAdmin` proof as the ladder's Level 3

**Status:** design, 2026-07-22. Scoped detour off goal `019f7cdb` (task `019f8951`).
**Character:** pure refactor — no DB migration, no wire-contract change (the admin surface is
OpenAPI-excluded), same authorization *outcome*, enforced structurally instead of by discipline.

## Problem

Admin authorization holds today, but by **discipline, not structure**. `require_system_admin(pool,
actor).await?` is an opt-in call at the top of each admin action. Forget it on a new action and the
action is open to anyone who can authenticate. Worse, two postures coexist:

- **Handler-gated** (`handlers/access.rs:184,202,228,244,262`, `embed.rs:195`) — `list_pending`,
  `review_request`, `get_admin_settings`, `update_settings`, `promote_admin`, `reembed`. The
  `is_system_admin` check sits in the axum handler. **A future MCP tool calling the same service fn
  would bypass it.**
- **Service-gated** (`access_service.rs:716–802`) — `admin_approve/revoke/deactivate/reactivate`,
  `demote_admin`, via `require_system_admin`. Both surfaces inherit (the F-3 posture).

The goal: an **enclosure** where admin-authz applies *by default* — registering an admin action
*without* the gate is the deliberate act, not the forgotten one — and where the gate lives at the
**shared api/mcp service layer** (F-3), because MCP does not go through axum, so a router-only
enclosure cannot cover it.

## The reframe: finish the ladder that already exists

`temper-services/src/auth/mod.rs` already builds a type-state ladder — we are adding its missing top
rung, not inventing a mechanism.

- **Level 1 — `AuthenticatedProfile`**: meant to be obtainable only via `gate_resolved_profile`
  ("takes the profile by value … so that constructing one without passing the gate requires going out
  of your way").
- **Level 2 — `SystemAuthorized`**: "Proof that a profile passed both levels … Only obtainable from
  `require_system_access`, which only accepts an `AuthenticatedProfile` — so the type makes it
  impossible to run Level 2 without having passed Level 1."

The intent *is* the enclosure: a proof obtainable only by passing the gate. **Admin is the natural
Level 3.** The current flaw (goal `019f7cdb` follow-up #1) is that the ladder is *documented, not
enforced*: `SystemAuthorized(pub AuthenticatedProfile)` and `AuthenticatedProfile`'s public fields let
a proof be forged by struct literal. So the new rung must be **sealed** to be worth anything.

L2 and L3 are **siblings off L1**, not a chain: `require_system_access` checks *has-access* (standing)
and `require_system_admin` checks *governance* — each consumes an `AuthenticatedProfile` independently
(the gate takes L1, not L2; see §3.1).

```
                         ┌─require_system_access──▶ SystemAuthorized   (L2: has access / standing)
AuthenticatedProfile ────┤
   (L1: authenticated)   └─require_system_admin───▶ SystemAdmin        (L3: admin / governance)
```

## Design

### 3.1 The `SystemAdmin` type and its gate

A new **sealed** rung in `temper-services/src/auth/mod.rs`, beside `SystemAuthorized`:

```rust
/// Proof the caller is a system admin (D10 governance). SEALED: the private field means the only way
/// to hold one is `require_system_admin`, which checks the DB. Forging one is a compile error outside
/// this module.
#[derive(Debug)]
pub struct SystemAdmin(ProfileId);   // private field

impl SystemAdmin {
    /// The acting admin — recorded as `actor` on every governance/standing/ledger write.
    pub fn actor(&self) -> ProfileId { self.0 }
}

/// Level 3 — governance check. Consumes proof of Level 1 (like `require_system_access`), but returns
/// `ApiResult` with a plain `Forbidden`: admin denial needs none of the CLI-presentation payload that
/// `AuthzError::SystemAccessDenied` carries, and `Forbidden` is exactly what the admin gate returns
/// today — so parity is trivial and no new `AuthzError` variant / surface mapping is needed.
pub async fn require_system_admin(
    pool: &PgPool,
    authed: &AuthenticatedProfile,
) -> ApiResult<SystemAdmin> {
    let actor = ProfileId::from(authed.profile.id);
    if access_service::is_system_admin(pool, actor).await? {
        Ok(SystemAdmin(actor))
    } else {
        Err(ApiError::Forbidden)
    }
}
```

Two deliberate choices:

- **Consumes L1, not L2.** One DB round-trip, and it matches D11's posture that `is_system_admin`
  reads governance *alone*, never ANDing standing. Requiring L2 first would impose a redundant
  standing check that the maintained invariant "admin ⇒ approved" (§9) already guarantees.
- **Carries the actor `ProfileId`** — the only thing admin fns need (for governance/standing/ledger
  writes). Not the whole profile: YAGNI; widen only if a later admin fn needs more.

**Ergonomics: borrow, not consume.** Admin fns take `&SystemAdmin`. In a stateless request/response
system there is no in-flight mutation window that one-proof-per-act would defend against, so consuming
the proof buys a linear-type property we have no threat to spend it on; one `require_system_admin` at
the top of a request authorizes every admin act in it.

**Also seal Level 2 in the same PR.** `SystemAuthorized`'s field becomes private + an accessor
(same-crate, trivial). It's newly trusted alongside L3, so leaving it forgeable would be a live
inconsistency.

### 3.2 Enclosure at the service boundary

The proof becomes a **required parameter** of every pure-admin service fn — its presence in the
signature *is* the authorization requirement (a capability parameter, unused-but-required where the
body doesn't need the actor):

```rust
// before: admin_revoke(pool, subject, actor: ProfileId, reason)   + internal require_system_admin(pool, actor)
// after:  admin_revoke(pool, admin: &SystemAdmin, subject, reason)   — reads admin.actor(), no internal check
```

This **collapses the two postures into one**: the service-gated set drops its internal
`require_system_admin` and takes `&SystemAdmin`; the handler-gated set gains `&SystemAdmin` on its
service fn and the inline `is_system_admin` leaves the handler entirely.

### 3.3 Surface minting

Every surface caller mints once, then calls the gated fn:

```rust
let admin = require_system_admin(&state.pool, &auth.0).await?;
access_service::admin_revoke(&state.pool, &admin, subject, reason).await?;
```

MCP inherits this for free at parity: it calls the same service fns, so it *must* mint the proof too.
The `audit-handler-authz-drift` tripwire (which today flags new handler-side `is_system_admin`)
evolves to assert every `/admin/*` route dispatches to a `&SystemAdmin`-taking fn.

### 3.4 The shared-reader nuance

Some handler-gated endpoints call a service fn **shared with non-admin or internal callers** —
`get_system_settings` is read by the admin endpoint *and* by `get_public_settings` (the public route)
*and* by `promote_admin`/`update_system_settings` internally. The proof **must not** go on the shared
reader (it would deny the public path). Instead the *admin act* — "read full settings as an operator" —
gets its own thin gated entry point: `admin_get_settings(pool, _admin: &SystemAdmin) -> SystemSettings`
wrapping the shared reader. Rule: **the proof goes on the admin-authority entry point, never on a
reader shared with non-admin or internal callers.**

## What gets the `SystemAdmin` proof, and what must never

This is the load-bearing distinction. Two kinds of authorization question, different *in kind*:

- **System-authority acts** operate on the *system itself* — no resource or team scope
  (approve/revoke/deactivate/reactivate/promote/demote a principal, system settings, join-request
  review, whole-index reembed). The only legitimate answer is "are you a system admin?" — a boolean
  about the principal's *system role*, one authority. **These take the proof.**
- **Scoped-capability gates** ask "may *this* principal do *this* to *this* scoped thing?" The answer
  is a *disjunction over roles scoped to the object* — system admin as a superuser override, **or**
  team owner/maintainer within team scope, **or** resource grantee within grant scope. Admin is **one
  branch, not the gate.** These compose the raw `is_system_admin` **bool** predicate; forcing them
  behind `SystemAdmin` would deny the very team/resource actors they exist to admit.

There is a **third bucket** the audit (below) surfaced: **conditional/parametric admin** — admin
required only for a *privileged option* of an otherwise-scoped op. `team_service::create` is the case:
anyone with the right parent-team role may create a team, but setting `auto_join_role` (an everyone-pool)
is admin-only (`:150`). It **cannot** take a required `&SystemAdmin` (non-admins legitimately create
teams without that param), so it stays a bool sub-check (`if req.auto_join_role.is_some() &&
!is_system_admin { forbid }`). Enclosing it would break the base scoped op.

The proof models **system authority**; it deliberately does not model **scoped capability** or a
**conditional sub-check**, and conflating them is the mistake this section exists to prevent (a future
"let's make it consistent" refactor forcing scoped gates behind the proof, silently denying scoped
actors).

**Worked example — `admin_ledger_service::visible_event_types` stays out.** Its read gate dispatches
*per act family*: `is_system_admin` reads everything (superuser short-circuit); grant acts →
`can_administer_grant` (`can_grant OR is_system_admin` — a scoped grantor reads them without being a
system admin); machine/connection acts → `machine_authz::authorize` (`is_system_admin OR owner of the
machine's owning team`); only the `admin_ledger_opened` epoch marker is admin-only. It is the canonical
"system admin OR properly-scoped team-owner/grantor" gate. It keeps composing the raw bool.
`machine_authz::authorize` and `can_administer_grant` likewise keep the raw predicate.

## Authz-site inventory (empirical audit, 2026-07-22)

Every production `is_system_admin` decision point, enumerated and classified — so **this PR** can be sure
it isn't missing an admin-only site *under the guise of assumed composition*, and so the deferred
`DomainAuthzAction` layer (task `019f8970`) inherits a documented action × sufficient-states trail. The
audit already earned its keep: it found `machine_registration::rebind`, a pure-admin site outside
`access_service` that the migration inventory had missed.

**Bucket 1 — system-authority → enclose with `&SystemAdmin`:**

| Site | Action | Sufficient states |
|---|---|---|
| `access_service::admin_approve/revoke/deactivate/reactivate` | transition a principal's standing | `is_system_admin` |
| `access_service::demote_admin` / `promote_admin` | revoke / grant governance | `is_system_admin` |
| `access_service::update_system_settings` + admin settings read (§3.4 wrapper) | write / full-read system settings | `is_system_admin` |
| `access_service::list_pending_requests` / `review_request` | list / decide join requests | `is_system_admin` |
| embed `reembed` | re-embed the whole index | `is_system_admin` |
| **`machine_registration::rebind`** (`:401`) | transplant a profile's identity + inherited reach onto a new `client_id` | `is_system_admin` — **team ownership is explicitly insufficient** (it would let an owner inherit reach they could never confer, defeating containment) |

**Bucket 2 — scoped-capability → keep raw bool, deferred to `DomainAuthzAction`:**

| Site | Action | Sufficient states |
|---|---|---|
| `machine_authz::authorize` (`:73`) | act on a machine row | `is_system_admin` OR owner of the machine's owning team |
| `can_administer_grant` / `require_admin_or_can_grant` (`:435`) | administer a grant on a subject | `is_system_admin` OR `can_grant(subject)` |
| `admin_ledger_service::visible_event_types` (`:80`) | read ledger acts (by subject) | per act: admin-all OR `can_administer_grant` (grant acts) OR `machine_authz` (machine acts) OR admin-only (epoch) |
| `admin_ledger_service` actor axis (`:181`) | read another principal's acts | `caller == actor` OR `is_system_admin` |
| `cogmap_service` (`:68`) / `context_service` (`:376`) | authorability / access | `is_system_admin` OR `is_gating_team` OR scoped grant |
| `team_service::get_members`-style read (`:283`) | read a team | `is_member` OR `is_system_admin` |
| `connection_service` (`:75`) | list connections | `is_system_admin` (all) OR own/scoped |
| `db_backend` (`:2030`) | cogmap-creation target selection | `is_system_admin` OR scoped |

**Bucket 3 — conditional/parametric admin → stays a bool sub-check:**

| Site | Action | Sufficient states |
|---|---|---|
| `team_service::create` `auto_join_role` (`:150`) | set an everyone-pool auto-join role *during* team creation | `is_system_admin` — **only when the `auto_join_role` param is present**; the base op (create team) is scoped |

Buckets 2 and 3 are deliberately **out** of the enclosure; Bucket 3 additionally cannot express the gate
as a required param. The `is_system_admin(pool, id) -> bool` predicate remains for all of them.

## Scope sealed now

- **L3 `SystemAdmin`** — new, sealed, in temper-services. Threaded through the pure-admin set.
- **L2 `SystemAuthorized`** — field made private + accessor.
- The raw `is_system_admin(pool, id) -> bool` predicate **stays** (the compositional gates and
  `require_system_admin` both use it).

## Honest limits

The proof guarantees "you cannot *call* a gated fn without being admin." It does **not** force a
*future* fn to be classified as admin — someone could still write `fn dangerous(pool, id)` with a
privileged write and no `&SystemAdmin`. Two mitigations, neither total:

1. The authorization contract now lives in the **signature** (`&SystemAdmin` present or absent) —
   reviewable at a glance, versus a buried `.await?` line.
2. The evolved `audit-handler-authz-drift` tripwire asserts `/admin/*` routes dispatch to
   proof-taking fns.

A deeper option — pushing the proof down to the privileged *write primitives* (e.g.
`principal_governance_set` requires `&SystemAdmin`, so the effect itself can't be produced without it)
— is **noted as a future**, not built here.

## Non-goals (deferred, filed)

- **L1 sealing.** A forged `AuthenticatedProfile` is a latent impersonation primitive (act as any
  supplied id) with an escalation path (supply a known admin's id → `require_system_admin` mints a real
  `SystemAdmin`). But it is **latent, not live**: the only construction site is the gate,
  `AuthenticatedProfile` is not `Deserialize`, and no wire surface reaches a forgery — so it is a
  structural guarantee against a *future* construction mistake (ours or a downstream crate-consumer's),
  not a production risk in any deployed system. Its correct fix is **provenance control** (the
  token→DB→gate path, no `Profile`-taking constructor), a medium refactor — not `#[non_exhaustive]`.
  Filed as goal `019f7cdb` follow-up #1; do it as a fast-follow.
- **The `DomainAuthzAction` policy layer** (task `019f8970`). The scoped-capability gates have the same
  "everything checks for itself" fragility one level up; modeling scoped authz as declarative actions ×
  sufficient-states is the generalization. `SystemAdmin` is its degenerate nullary base case — design
  the general layer *from* this concrete instance, after it lands. Not conjoined to this PR.
- **Write-primitive deepening** (above).

## Testing

- **Compile-time enforcement is the point — prove it with `trybuild`.** A negative fixture showing a
  pure-admin fn *cannot* be called without a `SystemAdmin`, and that `SystemAdmin` cannot be constructed
  outside `auth`. This PR's whole thesis is "we were technically doing it right but it took discipline →
  now it is typed + a tripwire"; a compile-fail proof is the natural completion of that — it is the
  guarantee, demonstrated, not asserted.
- **Behavioral parity** — the existing admin suites must stay green unchanged: temper-services
  `admin_demotion_test` and access tests; e2e `admin_surface_e2e` (`non_admin_is_forbidden_on_all_admin_
  endpoints` in particular — the 403s must be identical, just sourced from the service now). Reuse the
  born-Denied fixtures (`test_support::approved_admin`, e2e `common::approved_admin`).
- **No new migration, no sqlx schema change**; `cargo make check` + the security tripwires green.

## Migration inventory (the plan expands per-site)

**Service-gated → drop internal gate, take `&SystemAdmin`:** `admin_approve`, `admin_revoke`,
`admin_deactivate`, `admin_reactivate`, `demote_admin` (`access_service.rs:716–802`).

**Handler-gated → add `&SystemAdmin` to the service entry, remove the inline `is_system_admin`:**
`promote_admin`; `list_pending_requests`; `review_request` (today takes `reviewer_profile_id:
ProfileId` — after enclosure the reviewer id comes from `admin.actor()`, dropping the separate param);
the settings read (via a new `admin_get_settings` wrapper — §3.4); `update_system_settings`; `reembed`.

**Inline-gated in a non-`access_service` service → take `&SystemAdmin`, drop the bare check:**
`machine_registration::rebind` (`:401`) — surfaced by the audit; its handler
(`handlers/machine_clients.rs`) mints the proof. Note its rationale (reach containment: team ownership
is deliberately *not* sufficient) so a future reviewer doesn't "helpfully" widen it to `machine_authz`.

**Gate + type home:** `require_system_admin` and `SystemAdmin` in `temper-services/src/auth/mod.rs`
(must be co-located — the private field means only that module can construct the proof, so the gate that
mints it lives there). It returns `ApiResult<SystemAdmin>` with `Err(ApiError::Forbidden)` (§3.1). The
old private `access_service::require_system_admin(-> ApiResult<()>)` helper is removed.

**Untouched (compositional):** `machine_authz::authorize`, `can_administer_grant` /
`require_admin_or_can_grant`, `admin_ledger_service`.

## Rollout

Additive code refactor — no schema migration, no wire-contract change (admin surface is
OpenAPI-excluded), same authorization outcomes. Independent PR off `main`, alongside the remaining
Phase-1 beats (Tasks 16, 17). Rides auto-deploy.
