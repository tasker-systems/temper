# Machine-principal Phase B2 — team-owner registration and reach containment

Status: designed 2026-07-11
Goal: `019f4910` (temper-rb) · Task: `019f4f1d` · Follows: Phase A (PR #351), Phase B1 (PR #374)

## Problem

Machine-client registration is `is_system_admin`-only. Every machine principal — including one that
serves a single team — must be minted by a system admin. Phase B2 widens that to

```
is_system_admin OR is_team_owner(owner_team_id)
```

so a team owner can provision an agent for their own team without an operator in the loop.

**The task as handed off scoped this as "one predicate + tests, no migration." That framing is
wrong, and shipping it as written would open a privilege-escalation hole.** The gate is not only
guarding *who may register*; it is the sole authorization for *what reach the new machine receives*.
This spec exists to correct that scope.

### The hole

`machine_registration_service::apply_reach` writes the machine's reach with no authorization of its
own, by design. It inserts each requested `TeamSpec` straight into `kb_team_members` at whatever role
was asked for, and each requested `GrantSpec` via the low-level `insert_grant` — deliberately
bypassing `grant_capability`'s per-subject `can_administer_grant` check. Its own comment says so:

> *"the sole authorization here is the handler's `is_system_admin` gate (D5/D12 — Phase A
> registration is system-admin-only). A system admin may grant a machine write on any cogmap, so the
> per-subject `can_administer_grant` check is intentionally not applied. Do not 'tighten' this to
> `grant_capability` without revisiting D5."*

The inverse of that warning is the whole of B2: **the gate cannot be widened without containing
reach.** A team owner of team X who passes a widened gate with `owner_team_id = X` could also pass
`teams: [{team_id: Y, role: owner}]` and `grants: [{cogmap_id: <any>, can_write: true}]`, minting a
machine with owner reach into a team they do not own and write on any cogmap in the system. That is a
straight escalation to arbitrary team and cogmap write, available to any team owner.

Reach containment, not the predicate, is the substance of B2.

### What already exists (verified 2026-07-11)

Every primitive this design needs is already present in `temper-services` at `pub(crate)`. Nothing new
is invented.

- **`access_service::is_system_admin(pool, caller)`** → the SQL function
  (`migrations/20260624000002_canonical_functions.sql:1409`) resolves to *"caller holds role `owner`
  on the gating team."* System-adminhood **is** team ownership, of one distinguished team.
- **`team_service::role_on_team(pool, team_id, profile)` → `Option<TeamRole>`** — the one role check;
  `pub(crate)` precisely so sibling services reuse it.
- **`team_service::can_manage(role)`** → `Owner | Maintainer`. This is the bar `add_member` enforces.
- **`access_service::profile_can_grant(pool, caller, subject_table, subject_id)`** — the raw
  `can_grant` probe, built with *no* `is_system_admin` OR so that callers compose the admin case
  themselves. Exactly the primitive needed here.
- **`rebind`** copies `old.team_id` onto the new row, so keying it on the *old* row's team is
  coherent — a rebind never moves a machine between teams.
- **Surface scope:** registration is **temper-api + temper-cli only**. `temper-mcp` touches machine
  principals solely on the *authentication* side (`normalize_machine`, the Phase A gate) and exposes
  no registration tools. This change does not fan out across both surfaces, and the verifier is
  untouched.

## Design

### 1. The predicate — one authority, keyed by the owning team

`handlers::machine_clients::require_admin` is replaced by an authority resolution:

```
authorize(caller, team: Option<Uuid>) =
    is_system_admin(caller)                              // = owner of the gating team
 || team.is_some_and(|t| role_on_team(t, caller) == Owner)
```

`owner_team_id` is `Option<Uuid>`, so the non-admin branch **fails closed on `None`**. A teamless
machine (`team_id IS NULL`) remains admin-only to create, read, or operate. There is no
"no team to check, therefore permit" path — the NULL case denies.

This introduces no new *kind* of authority. `is_system_admin(p)` already **is**
`role_on_team(gating_team, p) == Owner`. B2's rule is: *registration is authorized by ownership of the
team that will own the machine*, and the gating team is the root of that hierarchy. Phase A's D6/D10
survive intact — `team_id` remains the machine's **owner** and never its **reach**; the predicate keys
on ownership of the *same* team the registration is performed on behalf of.

Each endpoint sources its team differently:

| Endpoint | Team keyed on |
|---|---|
| `provision`, `issue` | `request.owner_team_id` |
| `rebind`, `revoke`, `rotate_secret`, `get` | the existing row's `team_id`, loaded **before** any mutation |
| `list` | filtered in SQL to rows whose `team_id` the caller owns; admins see all rows including teamless |

### 2. Reach containment — `AuthorizedReach` makes the bypass unrepresentable

The containment checks could live in a pre-flight function that `apply_reach` is merely *expected* to
be called after. That is how we got here: the invariant lived in a comment. Instead, the checks
**produce a value**, and `apply_reach` consumes it:

```rust
/// Reach that has been authorized against a caller's authority. `apply_reach` takes this
/// instead of raw specs, so reach cannot be applied without having been authorized —
/// the check is not merely expected, it is required to construct the argument.
pub(crate) struct AuthorizedReach<'a> {
    teams: &'a [TeamSpec],
    grants: &'a [GrantSpec],
}

impl<'a> AuthorizedReach<'a> {
    /// Non-admin path: every team requires `can_manage` AND a non-`Owner` role (D4a — a
    /// gating-team maintainer must not be able to mint a system admin); every grant
    /// requires `can_grant` on its cogmap.
    async fn authorize(pool, caller, teams, grants) -> ApiResult<Self>;

    /// Admin path (Phase A D5): a system admin may grant a machine any reach. Named, so the
    /// bypass is visible at its call site rather than implicit in the absence of a check.
    fn system_admin(teams, grants) -> Self;
}
```

`apply_reach(conn, caller, profile_id, reach: AuthorizedReach)` — the raw specs are no longer callable.
The compiler now enforces what the comment asked for.

For a **non-admin** caller, `authorize` requires:

- every `TeamSpec.team_id`: `can_manage` (owner-or-maintainer) — **`add_member`'s membership bar**;
- every `TeamSpec.role`: **not `Owner`** — `add_member`'s *role* bar, as fixed by D7 (§4);
- every `GrantSpec.cogmap_id`: `profile_can_grant` on that cogmap — **precisely `grant_capability`'s
  bar for a non-admin** (`can_administer_grant` minus the admin OR, which is the non-admin case);
- `can_grant` on the minted grant stays `false`, as today — a machine can never re-delegate.

The invariant, stated once:

> **A machine can reach nothing the caller could not already have walked a human into.**

Because the bar is expressed by *calling* `can_manage` and `profile_can_grant` rather than by
restating their rules, any future tightening of the human surface tightens the machine surface with
it. There is no second copy of the policy to drift.

#### Why the role bar is load-bearing: the gating-team escalation

The membership bar alone is **not sufficient**, and the role bar is not decoration. `apply_reach`
inserts memberships with a **raw** `INSERT … ON CONFLICT DO UPDATE SET role = EXCLUDED.role` — it never
passes through `add_member`, so D7's guard does not protect it. Without an explicit role bar:

1. Alice owns some team `T` (anyone may create a team and own it) and is a **maintainer** of the
   gating team. She is **not** a system admin — `is_system_admin` requires role `owner`.
2. She registers a machine with `owner_team_id = T`, passing the widened gate as owner of `T`.
3. She passes `teams: [{team_id: <gating team>, role: owner}]`. This clears the `can_manage` bar,
   because *maintainer satisfies `can_manage`*.
4. `apply_reach` raw-inserts the machine as **owner of the gating team**. `is_system_admin(machine)`
   is now **true** — Alice has minted herself a system admin, from maintainer.

This is why D7 and the machine-side role bar are **the same rule enforced in two places**, not
alternatives. After D7, `add_member`'s bar is *`can_manage` **and** `role != owner`*; the machine
mirror is therefore *`can_manage` **and** `role != owner`*. Fixing only `add_member` leaves the raw
path open; capping only the machine path leaves the human path open. Both, or neither holds.

The general shape of the bug — *`can_manage` admits maintainers, but `owner` is the role that confers
authority* — is why any predicate that gates **granting a role** must be checked against the role
being granted, not only against the grantor's ability to touch the team at all.

Authorization runs **before the transaction opens** (auth before writes). A rejected reach must leave
the database untouched — no orphaned agent profile, no partial enrollment.

### 3. Read scoping

`list` gains a caller-scoped SQL predicate rather than a post-filter:

```sql
WHERE (mc.revoked_at IS NULL OR $include_revoked)
  AND ( $is_admin
        OR (mc.team_id IS NOT NULL
            AND EXISTS (SELECT 1 FROM kb_team_members tm
                         WHERE tm.team_id = mc.team_id
                           AND tm.profile_id = $caller
                           AND tm.role = 'owner')) )
```

`EXISTS` (not `array_agg`) so an empty scope denies rather than falling open. `get` and the
row-addressed lifecycle operations load the row, then apply `authorize(caller, row.team_id)` — a
non-owner receives `403 Forbidden`, indistinguishable from a non-existent row to a caller without
authority.

### 4. The adjacent fix — `add_member`'s missing owner-guard

`change_role` explicitly refuses to grant `owner`:

> *"cannot grant owner via role change; use ownership transfer"*

`add_member` has **no such guard**, and its `ON CONFLICT (team_id, profile_id) DO UPDATE SET role =
EXCLUDED.role` will happily upgrade an existing member to `owner` — bypassing `change_role`'s rule
entirely. This is a pre-existing hole on the human surface, independent of machines.

It is fixed here because B2's containment argument is *measured against `add_member`*. A bar that
leaks is not a bar. `add_member` gains `change_role`'s owner-guard, with its own tests.

This is bundled rather than extracted per the repo's stated convention: a fix whose story is *"this
PR's code path made it load-bearing"* belongs to the PR that surfaced it.

### 5. Testing

Access-semantics changes are the case where a green `test-db` run is a false signal, so the bar is
**e2e** (`tests/e2e/tests/machine_gate_e2e.rs` and a sibling for the widened paths):

| Assertion | Expect |
|---|---|
| Team owner provisions/issues into their own team | `200` |
| Non-owner (member/maintainer/stranger) of the owning team registers | `403` |
| Non-admin requests `teams` reach into a team they do not manage | `403` **and the DB is unchanged** |
| Non-admin requests a `grant` on a cogmap they cannot `can_grant` | `403` **and the DB is unchanged** |
| Non-admin requests a `TeamSpec` at `role = owner` on a team they manage | `403` (D4a) |
| **Gating-team maintainer** mints a machine at `role = owner` on the gating team | `403`, and `is_system_admin(machine)` is **false** — the escalation bite test (§2) |
| Non-admin registers with `owner_team_id: null` | `403` (fail-closed on NULL) |
| System admin retains full, unchecked reach | `200` |
| `list` as a team owner | only rows owned by their teams; no teamless rows |
| `get`/`revoke`/`rotate_secret`/`rebind` on another team's machine | `403` |
| `add_member` with `role = owner` | `400` (the adjacent fix) |
| Phase A + B1 gate tests | still green — the verifier and the auth seam are untouched |

The "DB is unchanged" assertions are the auth-before-writes proof and are the reason authorization is
not folded into the transaction.

## Decisions

- **D1 — Registration authority is team ownership; system-adminhood is its root case.** The predicate
  is `is_system_admin OR role_on_team(owner_team_id) == Owner`, and `is_system_admin` already resolves
  to ownership of the gating team. One concept, not two.

- **D2 — The NULL owning team denies.** `owner_team_id: Option<Uuid>` fails closed for non-admins;
  teamless machines stay admin-only across create, read, and lifecycle. An absent scope must never
  fall open.

- **D3 — Reach is contained by construction, via `AuthorizedReach`.** `apply_reach` takes an
  authorized value, not raw specs, so the unchecked path is unrepresentable rather than merely
  discouraged. Phase A's D5 admin bypass survives as a **named constructor**, visible at its call site.

- **D4 — The containment bar is the human bar, by call and not by copy.** Teams require `can_manage`
  (`add_member`'s membership bar) **and** a non-`Owner` role (`add_member`'s role bar, per D7); grants
  require `profile_can_grant` (`grant_capability`'s non-admin bar). Calling the existing predicates —
  rather than restating their rules — means the machine surface tightens automatically whenever the
  human surface does.

- **D4a — The role bar is not optional, because `can_manage` admits maintainers.** A gating-team
  *maintainer* is not a system admin, but clears `can_manage` on the gating team. Without a role bar
  they could mint a machine at `role = owner` on the gating team — an `is_system_admin` principal —
  escalating themselves to operator. `apply_reach`'s raw `ON CONFLICT DO UPDATE SET role` never passes
  through `add_member`, so D7 alone does not close this. See §2. **Any gate on granting a role must be
  checked against the role being granted, not merely against the grantor's access to the team.**

- **D5 — Full lifecycle, keyed by the owning team.** `provision`/`issue` key on the request's
  `owner_team_id`; `rebind`/`revoke`/`rotate_secret`/`get` key on the existing row's `team_id`; `list`
  is SQL-scoped. A team owner who mints a machine can operate it — rotate its secret, revoke it —
  without an operator, which is the point of the phase.

- **D6 — Machines never re-delegate.** The minted grant keeps `can_grant = false` (unchanged from
  Phase A). Reach containment bounds what a machine *receives*; this bounds what it can *pass on*.

- **D7 — `add_member` gains `change_role`'s owner-guard.** Bundled, because B2 measures containment
  against `add_member` and a leaking bar invalidates the argument. D7 closes the **human** path; D4a
  closes the **machine** path; they are one rule enforced at two write sites, and neither alone is
  sufficient (§2).

- **D8 — No migration.** `team_id` landed in Phase A; every predicate this needs already exists. This
  is the one part of the handed-off task's framing that survives intact.

## Rejected

- **Routing non-admin reach through `team_service::add_member` and `access_service::grant_capability`
  literally.** The purest form of "subset of the caller's authority" — it would execute the same code
  path a human does. Rejected because both take `&PgPool` while `apply_reach` runs inside a
  transaction; honoring it would force an `Acquire`-generic refactor of two shared services. D4 buys
  the same no-drift property by calling their *predicates*, at a fraction of the blast radius.

- **A pre-flight check that `apply_reach` is merely expected to follow.** Functionally equivalent to
  D3 on the day it ships, and identical to the arrangement that produced this hole in the first place.
  The typed value costs one newtype and buys compiler enforcement.

- **Capping machine team roles below `owner` *instead of* fixing `add_member`.** Considered as a
  narrower alternative to D7, and rejected — but note the cap itself is **adopted** (D4a), because it
  turned out to be load-bearing rather than optional. What is rejected is treating the two as
  alternatives: the machine cap alone leaves the human `add_member` path open, and D7 alone leaves
  `apply_reach`'s raw insert open. Both write sites enforce the same rule (§2).

- **Widening the gate to `can_manage` (admitting maintainers) for *registration*.** Reach delegation
  uses `can_manage`, but minting a credential requires `Owner`. Registration is the higher-consequence
  act: the credential is long-lived, non-interactive, and outlives the maintainer's tenure.

- **Unscoped `list` for anyone who passes the gate.** Machine rows carry no secret material, so this
  was tempting and cheaper. Rejected: it leaks the full machine inventory and team topology to every
  team owner.

## Deferred

- **Repointing the steward from Auth0 to temper's issuer.** Unchanged from B1 — waits on self-hosted
  infra being genuinely up.

- **`EMBED_DISPATCH_SECRET` / `INTERNAL_RECONCILE_SECRET` scheme harmonization.** Still orthogonal;
  carried from the Phase A and B1 Deferred lists.

- **Team-owner-visible audit of machine activity** (`last_seen_at` surfacing, per-team activity view).
  B2 gives a team owner the ability to mint and revoke; observing what the machine *did* is its own
  story.

## Open questions and risks

- **Blast radius of the `add_member` fix (D7).** It is a behavior change to a shared team surface: any
  existing caller relying on `add_member`-as-owner-grant will begin receiving `400`. The e2e and
  `test-db` tiers should catch this; if a legitimate caller surfaces, the fallback is D7's narrow
  variant (cap the machine path only) rather than dropping the guard.

- **TOCTOU between the authority check and the write.** Authorization resolves before the transaction
  (required for the auth-before-writes property). A caller stripped of ownership in the microseconds
  between check and commit would land one last registration. Accepted: `team_id` on a machine row is
  immutable, the window is negligible, and revocation is the remedy — the same posture the rest of the
  codebase takes.

- **Deploy ordering.** None. B2 adds no schema, so it is a pure code deploy with no
  migrate-then-deploy dance — unlike Phase A and B1, whose migrations must already be applied to prod.
