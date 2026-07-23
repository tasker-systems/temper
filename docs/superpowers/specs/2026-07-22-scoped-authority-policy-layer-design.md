# `ScopedAuthority` — naming and finishing the scoped-authz pattern the codebase already grew

**Status:** design, 2026-07-22. Task `019f8970-5dd0-7402-bda2-abdf8a8feff2`.
**Character:** pure refactor — no DB migration, no wire-contract change, no OpenAPI/gem/`schema.ts`
regen. Same authorization outcomes, enforced structurally instead of by discipline and doc comment.
**Successor to:** the admin-authz enclosure (`2026-07-22-admin-authz-enclosure-design.md`), which
sealed the *system-authority* rung and deferred both the scoped layer and the write-primitive
deepening to here.

---

## 1. The reframe: the task's premise is half-stale, and the better half is on disk

Task `019f8970` was filed on the premise that scoped gates *"still hand-roll their own composition —
`is_system_admin OR can_grant(subject) OR team_owner(team)` — inline, per gate, re-derived for every
new scoped action."* Re-grounding on disk after the enclosure landed, that is **no longer an accurate
description of this codebase**, and the design changes accordingly.

Two things are true instead:

**(a) The abstraction already exists, three times, unnamed.** Each is "resolve the caller's authority
over a scoped subject into a typed enum, then act under that authority":

| Incumbent | Arms | Site |
|---|---|---|
| `GrantAuthority` | `SystemAdmin` / `Delegated` / `None` | `access_service.rs:70` |
| `MachineAuthority` | `SystemAdmin` / `TeamOwner` | `machine_authz.rs:57` |
| `ActorAuthority` | `Credential` / `SelfPrincipal` / `Admin` | `temper-principal/src/act.rs:23` |

**(b) The hard rung — the task's own stated crux — is already solved once, in production.**
`AuthorizedReach<'a>` (`machine_authz.rs:93`) has private fields, is constructible only by
`authorize_registration`, and `apply_reach` takes it, *"which makes the unchecked path
**unrepresentable** rather than merely discouraged."* The task called scoped proofs-about-a-value
"genuinely more intricate than the nullary rung" and it is — but there is a worked instance to
generalize from, not a blank page.

There is also substantial deliberate **anti-drift** work already in place, which the task's premise
does not credit:

- `can_administer_grant` (`access_service.rs:81`) *calls* the write gate rather than restating it:
  *"Restating the predicate there would be a second copy of the policy that drifts from the gate it
  exists to mirror."*
- `cogmap_write_requires_admin` was extracted specifically so the grant path and the write path
  consult **the same** condition.
- `authorize_capability_grant` (`access_service.rs:214`) exists *because* the human and machine grant
  sinks drifted once — *"laundering by proxy. Hence one helper."*

So this design is **not** an intervention against a sprawl. It names a pattern the codebase converged
on, gives it one contract, and finishes the two things the pattern has not yet reached: the duplicated
two-sided gates, and the ungated write primitives.

### 1.1 Corrections to the enclosure spec's Bucket-2 table

The enclosure spec instructed this task to *"start from that table rather than re-enumerating"* while
warning it would move. It moved:

- **`require_admin_or_can_grant` no longer exists.**
- **`can_administer_grant` is not the gate** — it is a bool projection (`:81`) of `grant_authority`
  (`:103`), which returns the typed `GrantAuthority`.
- **Its sufficient-states are richer than "`is_system_admin` OR `can_grant`"**: an L0/gating-map
  escalation guard returns `None` **even for a `can_grant` holder** (`:118`), and the `Delegated` arm
  carries an attenuation obligation discharged in `authorize_capability_grant`.
- **Two sites the audit missed**: `machine_client_service::list` (`:96`), and
  `slack_disconnect_service::admin_disconnect_slack_principal` (`:267`) — see §7.

---

## 2. The mechanism

New module `crates/temper-services/src/authz/`, beside `auth/`.

### 2.1 The trait

```rust
#[async_trait]
pub(crate) trait ScopedAuthority: Sized + Copy + Debug {
    /// What this authority is *about*. `Copy` so the proof hands it back without cloning.
    type Subject: Copy + Debug;

    /// Sequenced probes, short-circuiting. SQL predicates stay authoritative — this
    /// routes to them, it does not restate them.
    async fn resolve(pool: &PgPool, caller: ProfileId, subject: Self::Subject) -> ApiResult<Self>;

    /// Denial is an ARM every domain must name, never an absence.
    fn is_denial(&self) -> bool;

    /// How this domain renders denial.
    fn denial() -> ApiError;
}
```

`#[async_trait]`, not native AFIT — matching the incumbent `Backend` trait
(`temper-workflow/src/operations/backend.rs:54`). AFIT would force `Send`-bound gymnastics at the
axum handlers for no gain, since every use here is static dispatch.

**`denial()` is load-bearing, not generality.** Two in-scope gates refuse with `NotFound`, not
`Forbidden`, and say why: `team_detail` hides team existence because *"team slugs are globally unique
and used in share flows"* (`team_service.rs:277–279`), and the ledger's actor axis does the same
(`admin_ledger_service.rs:181`). A trait that hardcoded `Forbidden` would silently convert two
deliberate information-hiding decisions into existence leaks.

### 2.2 The sealed proof

```rust
pub(crate) struct Authorized<A: ScopedAuthority> {
    authority: A,          // private
    subject: A::Subject,   // private
}

impl<A: ScopedAuthority> Authorized<A> {
    pub(crate) fn authority(&self) -> A { self.authority }
    /// The ONLY subject an act may touch.
    pub(crate) fn subject(&self) -> A::Subject { self.subject }
}

/// The one gate. Resolve, refuse denials in the domain's own dialect, seal the pair.
pub(crate) async fn authorize<A: ScopedAuthority>(
    pool: &PgPool, caller: ProfileId, subject: A::Subject,
) -> ApiResult<Authorized<A>> {
    let authority = A::resolve(pool, caller, subject).await?;
    if authority.is_denial() { return Err(A::denial()); }
    Ok(Authorized { authority, subject })
}
```

**Binding the subject *into* the proof is the point.** Today's two-sided gates spell their scope
twice: `bind_team` authorizes `(cogmap_id, team_id)` and then writes `(cogmap_id, team_id)` from its
own arguments (`cogmap_service.rs:34,38`). Nothing links the two spellings — a transposition, or a
later edit that re-derives one of them, is invisible to the compiler. With `Subject = (CogmapId,
TeamId)` the act reads `proof.subject()` and there is no second spelling to get wrong.

**Arms are preserved, never unified.** `GrantAuthority` keeps `SystemAdmin | Delegated | None`;
`MachineAuthority` keeps `SystemAdmin | TeamOwner`. They share the trait and never the type — their
intents differ, and the compiler should keep that boundary rather than have it DRY'd away.
`MachineAuthority` gains an explicit `None` arm so denial stops being an `Err(Forbidden)` returned
from inside `resolve`, which would bypass `denial()`.

### 2.3 Sealing the write primitives

`insert_grant` (`access_service.rs:289`) is `pub` and documented *"**Performs no authorization** —
every caller must gate first."* It has five callers, under four different authority domains. None is
a hole today; each is gated; the enforcement is a doc comment and reviewer attention.

| Caller | Gated by |
|---|---|
| `access_service::grant_capability:365` | `authorize_capability_grant` — authority arm + attenuation |
| `machine_registration_service:134` | inside `apply_reach`, which takes `AuthorizedReach` |
| `connection_service:480,503` | `machine_authz::authorize` + `contain_target_team` |
| `db_backend:2225` | nothing — cogmap **creator seed**, self-grant on a freshly-minted id |

That the four gates differ is **correct, not sloppy**: the connection path documents why it must not
route through grant authority — *"the `can_grant` seam has no bootstrap holder for a connection
subject"* (`connection_service.rs:419`). So the design does not force one gate. It enumerates the
legitimate warrants:

```rust
/// The COMPLETE set of ways a `kb_access_grants` row may be born. There is no ungated path;
/// adding a fifth way means adding an arm, in a diff, under review.
pub(crate) enum GrantWarrant<'a> {
    /// Human/API grant administration — authority arm plus attenuation.
    Administered(&'a Authorized<GrantAuthority>),
    /// Machine-registration reach, contained against the registrar's own.
    MachineReach(&'a AuthorizedGrant),
    /// Connection read-reach: authority over the connection, plus manage on the receiving team.
    ConnectionReach(&'a Authorized<ConnectionAuthority>),
    /// Creator seed at cogmap genesis — the subject is born in this txn.
    Birth(&'a BornSubject<RefTarget>),
}

impl GrantWarrant<'_> {
    /// `insert_grant` reads the subject from here and takes no subject argument at all.
    pub(crate) fn subject(&self) -> RefTarget { .. }
}
```

`insert_grant(conn, warrant, principal, caps, emitter)` — the `subject_table`/`subject_id` parameters
**disappear**. Not "must match the gate"; there is nothing to match, because there is one spelling.
Both primitives narrow `pub` → `pub(crate)` (verified: no caller outside temper-services).

> **The table above is the INSERT axis only, and this section originally mistook that for all of it.**
> `delete_grant` has its own two callers under their own two gates, both deliberately weaker than
> their granting counterparts. `GrantWarrant` cannot seal it. See §2.5, added 2026-07-23 during PR 3
> grounding.

The `MachineReach` arm takes `&AuthorizedGrant`, not `&AuthorizedReach`, because a reach carries many
grants and `apply_reach` loops over them (`machine_registration_service.rs:133`). Sealing each item
individually — `AuthorizedReach::grants() -> &[AuthorizedGrant]`, each carrying its own cogmap id —
makes the per-row subject structural instead of a runtime "is this grant actually in that reach?"
membership check.

### 2.4 Genesis, and why it is not an escape hatch on `Authorized`

The cogmap creator seed is legitimately ungated: at genesis there is no prior subject to hold
authority *over*. The obvious accommodation is `Authorized::at_genesis(..)`. **Do not.**
`Authorized<A>` is generic, so a forge on it hands **every** domain a bypass in order to solve **one**
domain's problem. Confine it to its own narrow type instead:

```rust
/// Proof that a subject is being BORN in this transaction — no prior authority over it can
/// exist, because it did not exist. Deliberately narrow, deliberately greppable.
pub(crate) struct BornSubject<S: Copy> { subject: S }
```

**Honest limit, stated rather than implied:** `BornSubject` cannot *prove* freshness. A caller could
mint one for an existing id. What it buys is confinement and visibility — one narrow `pub(crate)`
type, a name that reads as a claim, and a call-site-count test (§8) so a new construction site fails
a test rather than passing review unnoticed. Bootstrapping a system is hard to model without
exceptions; the code-risk is owned *somewhere*, and this is the smallest blast radius available.

### 2.5 The revoke axis — a second warrant, because de-escalation is deliberately cheaper

**Added 2026-07-23 (PR 3 grounding). §2.3 modelled insertion and assumed revocation was the same
shape. It is not, on either of its two paths, and the difference is a stated principle rather than an
oversight.**

| Sink | Caller | Gate |
|---|---|---|
| `insert_grant` | `access_service::grant_capability` | `authorize_capability_grant` — authority arm **+ attenuation** |
| `delete_grant` | `access_service::revoke_capability` | `can_administer_grant` — the arm **only** |
| `insert_grant` | `connection_service::grant_reach` | `ConnectionAuthority` — the connection **and** the receiving team |
| `delete_grant` | `connection_service::revoke_reach` | `MachineAuthority` on the connection **alone** |

Both weakenings are load-bearing and both say so where they live:

- *"Revocation is deliberately NOT attenuated: de-escalation must never be harder than escalation, or
  a grant becomes unwithdrawable"* (`access_service.rs:343`).
- *"Were revoke gated on the target team the way grant is, this grant would now be permanently
  unrevokable — access stranded on a connection whose own owner can see the grant and cannot
  withdraw it"* — the doc on `revoke_reach`, and the test
  `revoke_reach_survives_losing_the_target_team_role`, which opens *"The grant/revoke asymmetry,
  asserted so nobody 'fixes' it into symmetry."*

So `revoke_reach` **cannot** produce an `Authorized<ConnectionAuthority>`: that proof requires the
target-team role revocation must not demand. Forcing one warrant across both axes would therefore
tighten `revoke_reach` — a behavior change an existing test forbids by name.

```rust
/// The COMPLETE set of ways a `kb_access_grants` row may be REMOVED. Parallel to `GrantWarrant`,
/// and deliberately not the same enum: the gates are weaker on purpose, and one type spanning both
/// axes would make an insertion arm mintable at a revocation site.
pub(crate) enum RevokeWarrant<'a> {
    /// Human/API grant administration. The arm WITHOUT attenuation — revocation is not attenuated.
    Administered(&'a Authorized<GrantAuthority>),
    /// Connection read-reach withdrawal — authority over the connection, and nothing about the
    /// team losing the reach.
    ConnectionControl(&'a Authorized<ConnectionControlAuthority>),
}
```

`ConnectionControlAuthority` is **not new policy**. It is `ConnectionAuthority`'s *first* question —
*"may you act on this connection?"*, `MachineAuthority` keyed on the connection's owning team —
factored out under its own name, with `Subject = ConnectionId`. `ConnectionAuthority` then composes
it rather than restating it, so the two cannot drift; that composition is the reason this factoring
is a simplification and not an extra type.

**Decision (Pete, 2026-07-23):** a separate `RevokeWarrant`, over sealing insertion alone. Sealing
one primitive would leave D4's guarantee — *the effect cannot be produced unauthorized* — true for
grants and false for revocations, and would leave the second spelling of the subject alive on the
delete path, which is the thing §2.3 exists to remove.

---

## 3. Migration inventory

**In scope:**

| Site | Becomes | `Subject` | Denial |
|---|---|---|---|
| `access_service::grant_authority` (`:103`) | `impl ScopedAuthority for GrantAuthority` | `RefTarget` | `Forbidden` |
| `machine_authz::authorize` (`:68`) | `impl ScopedAuthority for MachineAuthority` + explicit `None` arm | `Option<TeamId>` (fails closed on `None`) | `Forbidden` |
| `cogmap_service::can_bind` (`:62`) **+** `context_service::can_share` (`:370`) | **one** `TwoSidedAuthority` resolver, parameterized by the subject-administration probe | `(RefTarget, TeamId)` | `Forbidden` |
| `connection_service` reach-grant (`:480,503`) | `ConnectionAuthority` | `(ConnectionId, TeamId)` | `Forbidden` |
| `team_service::team_detail` (`:280`) | `impl ScopedAuthority for TeamReadAuthority` | `TeamId` | **`NotFound`** |
| `admin_ledger_service::list_by_actor` actor axis (`:181`) | `impl ScopedAuthority for ActorHistoryAuthority` | `ProfileId` (the actor read about) | **`NotFound`** |

> **`list_by_actor` carries two checks, and only the second one migrates.** The `has_system_access`
> gate immediately above it (`:176`) is a **standing** question, not a scoped-authority one — it asks
> whether the caller is admitted at all. It stays exactly where it is. Only the `caller != actor &&
> !is_system_admin` axis (`:181`) becomes a `ScopedAuthority`. Folding the standing check in would
> conjoin a provisional fact into an authority decision, which is the shape `temper-principal`'s
> `admit` exists to forbid.
| `insert_grant` (`:289`) / `delete_grant` (`:320`) | take `GrantWarrant`; drop subject params; `pub` → `pub(crate)` | from the warrant | — |

**Out of scope, with the reason recorded** — so "deliberately absent" and "nobody got around to it"
stay distinguishable, the way `readable_event_types`' fail-closed default already does it:

- **Filter-shaped** — `connection_service::list` (`:75`), `machine_client_service::list` (`:96`). The
  authority becomes a `WHERE` clause, not a branch; bringing it under the trait means generating SQL
  from Rust authority values. The duplication between the two is real ("admin sees all OR
  owner-of-owning-team", hand-written twice) and should be **filed separately**, not bundled.
- **Projection-shaped** — `admin_ledger_service::readable_event_types` (`:73`). It returns the set of
  readable event types, not a yes/no, and is already the best-behaved gate in the codebase: it
  *calls* `can_administer_grant` rather than restating it, expressly so the two cannot drift. A
  capability-set output mode on the trait would be a much larger abstraction bought for one site.
- **Conditional/parametric** — `team_service::create`'s `auto_join_role` (`:150`). A required proof
  would break the base scoped op that non-admins legitimately use. Stays a bool sub-check, as the
  enclosure spec's Bucket 3 concluded.
- **Conditional/parametric** — `access_service::require_cogmap_write_admin`. **Added 2026-07-23:
  Task 12's sweep found it, and this list had missed it.** It is Bucket 3 for the same reason as
  `auto_join_role`: the admin requirement applies *only* when `cogmap_write_requires_admin` says the
  map is in the reserved-L0-or-gating-team regime, so every write to an ordinary cogmap returns `Ok`
  without consulting authority at all. A required proof would gate the base op that the whole
  non-reserved corpus uses. Reported rather than migrated, per the plan's Task 12 Step 2 rule.
- **`get_entitlements`** (`access_service.rs:1161`) — reports admin-ness; it is not a gate.

---

## 4. Decisions

- **D1 — Name it for what it is.** The task proposed `DomainAuthzAction`/`DomainActPolicy`. Dropped:
  that name belongs to the action×sufficient-states framing (§5, rejected). What we are building is a
  *scoped authority* resolved per domain, so it is `ScopedAuthority` + `Authorized<A>`. A name that
  misdescribes the thing misleads every future reader.
- **D2 — A trait, not one shared enum.** The three incumbents have different arm sets *and* different
  intents. Same shape, distinct types; the compiler keeps the boundary.
- **D3 — I/O-coupled, in temper-services.** `resolve` does its own sequenced probes, preserving the
  short-circuit (`grant_authority` costs 1 query for an admin, 3 for a denied delegate). This
  resolves the task's tension 1 by construction: SQL predicates stay authoritative and this is a
  **router, not a rewrite**. Accepted cost: judgment is testable only against a real DB, so this layer
  does not get `temper-principal`'s no-DB exhaustive-table property. That purity was not available
  anyway — `temper-principal` earns it by *"performing no I/O"* (`lib.rs:14`), and a gate that must
  ask the database cannot.
- **D4 — The proof reaches the write primitives.** Anything less leaves `insert_grant` callable
  ungated, so the guarantee would be "you can't skip your domain's gate" rather than "the effect
  cannot be produced unauthorized." This collects the enclosure spec's explicitly deferred
  write-primitive deepening (`:235`).
- **D5 — Genesis gets its own type, not a hatch on `Authorized`.** §2.4.
- **D6 — `denial()` is justified by migrating the two `NotFound` gates**, not by hypothetical future
  adopters. Without them it would be speculative generality with one possible answer.

---

## 5. Approach considered and rejected

**A central declarative `DomainAuthzAction` enum + registry** (the task's original framing): one table
listing every scoped action with the sufficient states that satisfy it, one authorizer dispatching over
it. Rejected because it is a **second representation of policy**, sitting beside the SQL predicates
that actually decide, free to drift from them — which is the exact failure this codebase has already
paid for twice (`can_administer_grant`'s restatement risk; the human/machine grant-sink drift). The
trait gets the enumerability another way: *all impls of `ScopedAuthority`* is a list the compiler
maintains, and it cannot describe a policy the code does not run.

---

## 6. Finding: the two-sided gate is inconsistent across its three instances

Surfaced by putting them side by side — which nothing in the codebase currently does.

| Gate | Subject side | Team side | Gating team excluded? |
|---|---|---|---|
| `cogmap_service::can_bind` (`:62`) | `can_grant` on the map | `can_manage` | **yes** — binding to the gating team flips the map into the `require_cogmap_write_admin` regime |
| `context_service::can_share` (`:370`) | administers the context | `can_manage` | **yes** — *"sharing into the root team is an instance-level escalation"* |
| `machine_authz::contain_target_team` (`:180`) | caller's `MachineAuthority` | `require_manage_on_team` | **no** |

Two of three treat binding into the root team as an instance-level escalation and keep it admin-only.
The third does not.

### 6.1 Resolved (2026-07-23, Pete): the asymmetry is correct, for three distinct reasons

**The framing above is wrong, and PR 2 grounding corrected it.** This is not one asymmetry with a
direction to pick. It is three sites whose gating-team relationship genuinely differs, two of which
carry a *reason that was never written down* and one of which carries a reason that **D11 invalidated
while leaving the guard standing**. The resolution is therefore **no behaviour change at any of the
three** — and the actual deliverable is the three reasons, recorded where each gate lives.

**`can_bind` — structurally load-bearing, and the comment understates it.** The gating-team join is a
direct input to the admin-write regime:

```sql
-- access_service.rs:434-458, cogmap_write_requires_admin
SELECT EXISTS( SELECT 1 FROM kb_team_cogmaps tc
   JOIN kb_teams t ON t.id = tc.team_id
   JOIN kb_system_settings s ON t.slug = s.gating_team_slug
  WHERE tc.cogmap_id = $1 )
```

`can_bind` gates **unbind** as well as bind (`cogmap_service.rs:121` — *"symmetric with bind — a
principal who could bind may unbind"*). The exclusion's sharper direction is therefore the one the
doc comment omits: without it, a non-admin holding `can_grant` on a map who also manages the gating
team could **unbind a protected map**, dropping it out of the admin-write regime. Binding is a
self-inflicted restriction; unbinding is a genuine escalation. Keep the guard; fix the comment.

**`can_share` — the guard is right, the stated reason is stale.** The comment reads *"sharing into the
root team is an instance-level escalation."* That held when gating-team membership **was** instance
access. It no longer is — the live predicates consult the gating team for neither question:

```sql
-- has_system_access(p_profile_id)
SELECT EXISTS (SELECT 1 FROM kb_principal_standing s
                WHERE s.profile_id = p_profile_id AND s.state = 'approved')
-- is_system_admin(p_profile_id)
SELECT EXISTS (SELECT 1 FROM kb_principal_governance g WHERE g.profile_id = p_profile_id)
```

Post-D11 the gating team is the join-request target and the cogmap-regime marker, and confers no
standing and no admin-ness. So the guard survives on a **different** and narrower footing: `can_share`
also gates `reassign` (`context_service.rs:505`), which is a transfer of *ownership* into the root
team, and that act is independently forbidden in plpgsql (§6.2). Relaxing the Rust half would put the
two copies into different error paths for the same act. Keep the guard; replace the reason.

**`contain_target_team` — no structural need for one, and that is deliberate.** A reach grant writes a
`kb_access_grants` row (`subject_table = 'kb_connections'`) conferring READ on what the connection
receives. It flips no regime, and what it exposes is the granter's own connection data. The absence is
the correct policy, and PR 2 pins it with a test so it reads as a decision rather than an oversight.

### 6.2 A fourth instance the table missed, and it is in SQL

`context_reassign` (`migrations/20260715000010_context_reassign_fns.sql:77-93`) re-implements this
entire policy in plpgsql — admin bypass, non-gating target team, owner/maintainer on it,
administers-the-context — with its own gating-team `RAISE … ERRCODE '42501'`. **Task 7's Rust collapse
cannot reach it**: after PR 2 the two-sided policy still exists twice, once in `TwoSidedAuthority` and
once in the reassign function. That is not a defect introduced here (the SQL copy is the atomic
enforcement behind a Rust pre-check, by design — `context_service.rs:629`), but it bounds the
collapse's claim: **one policy where there were three, in Rust.** Do not describe PR 2 as leaving one
copy.

### 6.3 What PR 2 does about it

Task 6 is **not** a red-green behaviour change. It is: correct `can_bind`'s comment to name the unbind
direction; replace `can_share`'s D11-invalidated rationale with the reassign/plpgsql one; add a test
pinning `contain_target_team`'s deliberate absence of a guard; and record §6.2 where the collapse
happens. The three existing tests that pin the two live exclusions
(`bind_cogmap_e2e.rs:424`, `context_share_e2e.rs:437`, `context_service.rs:946`) are **not touched** —
under the plan's own rule, needing to edit them would have been the signal that the direction was
wrong.

---

## 7. Finding: an enclosure gap, out of scope but filed here

`slack_disconnect_service::admin_disconnect_slack_principal` (`:267`) is a **pure system-authority**
act — `if !is_system_admin(pool, req.actor) { return Err(Forbidden) }` — that the enclosure's
*"empirical audit of every production `is_system_admin` decision point"* did not list, so it never got
its `&SystemAdmin`. It is Bucket 1 and it is still gated by discipline.

Two aggravating details, and one mitigating one:

- It gates on `req.actor`, a **field of a request struct**, not a caller parameter. The gate therefore
  binds whatever the caller put in the struct.
- The comment beside it names a **second, planned surface** (`@temper disconnect` in Slack) as the
  reason the gate sits in the service — so a future caller filling `actor` from somewhere other than
  auth is anticipated, not hypothetical.
- **No live vulnerability**: the sole caller today fills it from the authenticated principal
  (`handlers/slack_disconnect.rs:155`, `actor: ProfileId::from(auth.0.profile().id)`).

**File as a separate task against the enclosure**, not this design: it takes `&SystemAdmin` and reads
`admin.actor()`, which is the enclosure's pattern, not this one's. Recording it here so the audit's
gap is written down where the next person looks.

---

## 8. Testing

- **No `trybuild` fixtures for this layer — and that is a conclusion, not a gap.** The existing three
  fixtures work because `SystemAdmin`/`SystemAuthorized` are **`pub` types with private fields**, so an
  external fixture can name the type and fail on the field. Everything sealed here — `Authorized`,
  `BornSubject`, `GrantWarrant`, and the narrowed `insert_grant` — is **`pub(crate)`**, so an external
  fixture cannot name it at all and would fail with "item is private" *whether or not the seal
  existed*. That is a test that passes for the wrong reason, and it would rot into false assurance the
  first time someone loosened a field. The seal here rests on Rust's module privacy, which is exactly
  what `AuthorizedReach` (`machine_authz.rs:93` — `pub(crate)`, private fields, no fixture) has relied
  on since it shipped. **Do not add a fixture to "match" the enclosure.**
- **A call-site-count test for `BornSubject`** — in-crate, so unlike a fixture it *can* reach the type.
  In the style of `admission.rs`'s signature test: a new construction site fails a test rather than
  slipping through review. The test carries the reason, so the fix is never "bump the number."
- **`GrantWarrant` exhaustiveness** — a match over the enum with no `_ =>` arm, so a fifth way to write
  a grant row is a compile error at the primitive, which is the property §2.3 claims.
- **Behavioral parity** — `admin_ledger_test`, `access_grants_test`, `cogmap_authz_test`,
  `team_lifecycle_test`, `machine_authz`'s own suite, and the e2e admin surface stay green
  **unchanged**. Same outcomes; only the enforcement moves.
- **The §6 asymmetry** gets a test pinning whichever way PR 2 resolves it.
- **No migration, no `.sqlx` regen** (no SQL text changes — the predicates are called, not rewritten),
  **no OpenAPI/gem/`schema.ts` regen** (everything is `pub(crate)`; no DTO moves).

---

## 9. Sequencing

Three PRs, each cut off current `main`, **not stacked** — 3 depends on 1 and 2, so it waits for them
to merge rather than branching from them.

1. **Mechanism + incumbents.** `authz/` module, trait, `Authorized<A>`; migrate `GrantAuthority`,
   `MachineAuthority`, and the two `NotFound` read gates. Zero new policy — the shape is proven on
   code that already worked.
2. **Collapse the two-sided gates.** One `TwoSidedAuthority` replacing `can_bind`/`can_share`, plus
   `ConnectionAuthority`. Resolves §6.
3. **Seal the write primitives.** `GrantWarrant`, `BornSubject`, the `insert_grant`/`delete_grant`
   signature change.

Each is behavior-preserving and independently shippable. Additive-only on `main`; rides auto-deploy.

---

## 10. Honest limits

- **The warrant enum covers `kb_access_grants` and nothing else.** Its sibling write in the same
  loop — `apply_reach`'s raw `INSERT INTO kb_team_members` (`machine_registration_service.rs:121`) —
  is authorized only by sitting *inside* a function that takes `AuthorizedReach`, not by the write
  itself demanding a warrant. That is the weaker form, and it is the form every privileged write
  outside `kb_access_grants` still has. Extending §2.3's treatment to team-membership writes is a
  real follow-up, not a hypothetical one.
- `BornSubject` confines the genesis exception; it does not eliminate it (§2.4).
- This layer is DB-coupled by D3, so its judgment has no no-database exhaustive-table test the way
  `temper-principal`'s `transition()` does. Reviewability comes from the trait's impl list and the
  compile-fail fixtures instead.
