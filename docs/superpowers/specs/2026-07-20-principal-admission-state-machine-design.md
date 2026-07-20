# Principal Admission — a fail-closed state machine

**Date:** 2026-07-20
**Status:** design, approved in brainstorm; not yet planned
**Supersedes in part:** the `access_mode` retirement goal (`019f7cdb-a1b6-7e80-b19a-349a3d427671`)

## §1 Why this exists

Answering *"may this principal use this instance?"* currently requires ANDing conditions across
multiple tables, written by uncoordinated call sites, whose meanings differ by which door the
principal entered through. That shape produces latent bugs faster than we can find them. Two were
found in a single morning, and neither was visible in the diff that would have introduced it.

**Bug one — the tier and the membership disagreed.** Approving a join request updated
`kb_join_requests.status` and inserted a `kb_team_members` row, but never wrote
`kb_profiles.system_access`. `promote_admin` likewise wrote only a membership — its own docstring
described this as intentional:

> `access_service.rs:571-576` — *"Decoupled from `kb_profiles.system_access` (the auth gate reads
> gating-team ownership, not the enum)."*

True when written. It meant that the moment the gate reads the enum, **both** of the system's
access-granting doors become silent no-ops, because `ensure_auto_join_memberships` opens with an
early return:

```sql
-- migrations/20260629000002_auto_join_team_generalization.sql:44
IF NOT has_system_access(p_profile) THEN
    RETURN;  -- not eligible (invite_only non-member); enroll nothing
END IF;
```

Approval would have inserted a membership that granted nothing, enrolled nothing, and returned
success.

**Bug two — a guard that stops guarding.** `enroll_in_gating_team` enforces a real invariant:

> `machine_registration_service.rs:44` — *"from then on a machine can only hold system access if the
> human who minted it did."*

It enforces it by conditioning the machine's gating-team membership on the *minter's* membership.
Write the tier unconditionally at provision, and the check still runs, still logs, still refuses —
but it now refuses a thing that no longer controls access. The guard is not removed; **access moves
out from under it.** The escalation only becomes reachable one deploy later, in a different file
from the one that caused it.

The common cause is not carelessness at either site. It is that **no component owns the question**.
Standing is spread across `kb_profiles.system_access`, `kb_profiles.is_active`, gating-team
membership, and `kb_join_requests.status`, each written by whoever happened to need it, and read by
a predicate whose basis has changed twice. This design gives the question exactly one owner.

## §2 Scope

**In:** Levels 1 and 2 — *principal admission*. From credential to "this principal may act at all."
Covers the four mint paths, standing, requests, revocation, deactivation, and machine registration's
containment rule.

**Out — governance.** Whether a principal may *change the rules* (`is_system_admin`, promotion,
demotion) is a **separate state machine with its own spec**, deliberately. Admission asks *may you
act*; governance asks *may you govern*. Different authority, different blast radius, different
revocation semantics. This spec defines only the seam (§9).

**Out — Level 3.** Resource authorization (`resources_visible_to`, `can_modify_resource`, team
roles, cogmap grants) is untouched. It is SQL-resident, large, and has a settled design. It keeps
calling `has_system_access` and does not care that the predicate's basis changed. A future
dedicated authz layer is plausible and explicitly not in scope here.

## §3 Decisions

| | Decision |
|---|---|
| **D1** | Two machines, explicitly separated: a **persisted** `Standing` lifecycle and a **pure, per-request** `Admission`. Standing is evidence the admission machine reads. |
| **D2** | Standing is **one authoritative state in one table**. Not a conjunction across tables. |
| **D3** | The pure machine lives in a **new crate, `temper-principal`**, with no `sqlx` dependency — purity enforced by the compiler, not by convention. |
| **D4** | Transitions write a **dedicated append-only log** *and* emit a registered **admin-category `kb_events`** record, atomically. |
| **D5** | "Requested" is a **standing state**; `kb_join_requests` keeps only request-shaped payload (message, terms acceptance, decision note) and loses its status column. |
| **D6** | `is_active` is **folded in** as the `Deactivated` state. Its non-auth readers move or consume a maintained projection. |
| **D7** | Connection profiles get **no standing row at all**. Absence denies, so their safety is structural. |
| **D8** | Backfill by evaluating the **old predicate**, making the cutover behaviour-preserving on every instance. |
| **D9** | `Requested` is **per-principal and carries no team dimension**. Asking to join a *team* is orthogonal to standing in the system and is out of scope. |

### On D9 — two different questions wearing one table

`kb_join_requests` is shaped as though requests were per-team: `team_id` is a NOT NULL FK and the
uniqueness constraint is `(team_id, requesting_profile_id) WHERE status = 'pending'`. In practice
`create_join_request` only ever targets the gating team — it resolves `gating_team_slug` and errors
if none is configured — so every row that exists is really *"may I use this instance?"* wearing a
per-team shape.

Those are two different questions. **"May I be in the system at all"** is a property of the
principal and is what standing models. **"May I join this team"** is a property of a
(principal, team) pair, is ordinary membership, and does not affect whether the principal is
admitted. Conflating them is what put a `team_id` on a system-access request.

So standing carries the system question with no team dimension, and team-join-as-such stays
orthogonal and unbuilt — there is very little of it today. The practical consequence for
implementation: **do not carry `team_id` into the standing tables**, and do not treat the existing
unique index as evidence that standing needs a per-team key. When the request record loses its
status column (D5), its `team_id` stays where it is, describing the request rather than the
standing.

### On D2 — why not AND

An intermediate design ANDed gating-team membership with the tier, on the reasoning that the
membership conjunct would preserve minter containment. It was rejected, and the reason generalizes:

*A conjunction across provisional conditions, written by different paths, is the bug shape itself.*

Concretely, the same AND produced opposite verdicts for two principals from one predicate. A machine
minted by an ineligible minter would be correctly denied (no membership). A SAML human born
`approved` would **also** be denied — permanently — because the auto-join trigger that would grant
the missing conjunct is itself gated on the predicate it would satisfy. One rule, two outcomes,
neither obvious from reading it.

Containment does not disappear; it **relocates to a guard on the transition** (§6), where it belongs.
That is strictly better than either alternative: the act is refused at the point of the act, with a
reason recorded, rather than granted and silently useless later.

## §4 The two machines

```
credential ─► classify ─► resolve profile ─► load standing ─► Admission
             └─ temper-services, unchanged ─┘   └── temper-principal, pure ──┘
                                                          │
                              Admitted(AdmittedPrincipal) ─┤
                              Refused(Refusal) ────────────┘ ──► typed 403

transitions run the other way:
  services gathers evidence ─► temper-principal decides ─► ONE SQL function commits
                                                          (row + log + event, one txn)
```

The claims→profile seam stays `pub(crate)` in `temper-services`. `temper-auth`'s module doc is
explicit that lifting it would silently destroy a security property:

> *"`authenticate` / `resolve_from_claims` are `pub(crate)` in temper-services **as a security
> property** (a surface cannot hand them claims it built itself). Lifting them into a shared crate
> would turn `pub(crate)` into `pub` across a crate boundary and the guarantee would evaporate
> silently."*

`temper-principal` never resolves a credential. It judges assembled evidence. That is what makes it
safe to share.

## §5 States

Five, plus absence.

```
        (no row) ──────────────► fail-closed: DENIED
            │
            │ Provision
            ▼
  ┌──── Denied ◄──────────┐
  │        │              │ Reject
  │        │ Request      │
  │        ▼              │
  │    Requested ─────────┘
  │        │ Approve
  │        ▼
  │    Approved
  │        │ Revoke
  │        ▼
  └───► Revoked

  Deactivate / Reactivate move any state to and from Deactivated.
```

- **`Denied`** — provisioned, never granted. Where OAuth first-login lands, by design (§8).
- **`Requested`** — has asked for **system** access. Still denied, but the refusal can say so and a
  duplicate is refusable. **Per-principal, with no team dimension** — see D9.
- **`Approved`** — may use the instance.
- **`Revoked`** — *was* granted and lost it. A different sentence to the user and a different signal
  in an audit than never having had it.
- **`Deactivated`** — the principal itself is disabled. Prior standing is recoverable from the log,
  so reactivation restores rather than guesses.

Rejection is deliberately **not** a state: a rejected request returns standing to `Denied` so the
principal may re-request — `join_request_rejection_allows_resubmit` (`access_gate_test.rs:403`)
already expects this — while the request record keeps the `decision_note`. Standing carries state;
the request carries payload.

## §6 Acts and guards

Nine acts. Three actor authorities: **none** (the credential itself is the authority), **self** (the
principal acting on its own standing), and **admin** — an actor for whom `is_system_admin` holds.

> **On "admin" as an actor authority when governance is out of scope (§2).** This spec *consumes*
> admin-ness as an input; it does not define or grant it. `is_system_admin` exists today and keeps
> working. The governance spec owns how a principal becomes admin. The two meet only at the seam in
> §9, which is what keeps that boundary from becoming a circular dependency.

| Act | Actor | Notes |
|---|---|---|
| `Provision { path }` | none | the credential is the authority; see below |
| `Request` / `Withdraw` | self | |
| `Approve` / `Reject` | admin | |
| `Revoke { reason }` / `Reinstate` | admin | |
| `Deactivate` / `Reactivate` | admin | |

Provision is where the doors diverge, and it is the entire reason this exists:

| path | resulting standing | guard |
|---|---|---|
| SAML assertion | `Approved` | none — the IdP is the authority; no human in the loop to bound |
| OAuth first-login | `Denied` | none |
| Machine registration | `Approved` | **containment** — may not exceed the minter's own standing |

**SAML and OAuth share one mint function today** (`create_new_profile_and_link`, reached from both
`resolve_federated_human` and `authenticate`). The divergence must therefore be carried by the act,
never by a constant at the shared site. The failure direction is permissive and silent: every OAuth
signup born `Approved` would open the instance to anyone who can sign in, and nothing would notice.

**The containment guard refuses the act rather than minting a denied machine.** Minting a credential
*is* approval by intent, so a minter who cannot confer access should not receive a credential that
silently does not work. Today that case registers cleanly and 403s later, with the explanation in a
log line nobody reads.

## §7 Fail-closed obligations

Three, all about edges rather than the happy path.

1. **Absence denies.** No standing row is not an error and not a default-grant. This is what makes
   D7 structural.
2. **An unrecognized state denies.** Parsing is `&str -> Option<Standing>`; `None` refuses. Never a
   panic, never a default. The column can hold a value a given binary does not know during a rolling
   deploy or after a rollback. **This is the obligation most likely to be got wrong, because it only
   bites inside a deploy window.**
3. **No catchall admits.** Every `match` over `Standing` is exhaustive with no `_ =>` arm. Adding a
   state becomes a compile error at every decision site — the property the separate crate buys.

`AdmittedPrincipal` is constructible only by the machine, preserving the type-state guarantee
`SystemAuthorized` has today: passing Level 2 without having passed Level 1 stays unrepresentable.

`Refusal` is a typed enum, which retires a wart — the enriched 403 currently carries
`access_mode: String`, and its tests assert a sentinel `"join_request"` that is not a real mode
(`temper-services/src/error.rs:299,377`).

### The SQL read must be total

Level 3 predicates call `has_system_access` inside data queries, so it stays a SQL function reading
the standing table. It **must** be written as `EXISTS(SELECT 1 … WHERE state = 'approved')`, not
`SELECT state = 'approved' FROM …`.

With no matching row the latter returns **`NULL`, and `NULL` is not `false`**. In
`IF NOT has_system_access(...) THEN`, a `NULL` makes the guard silently not fire — so a principal
with no standing row sails past a check that reads as though it denies. `EXISTS` is total. The same
obligation applies to `is_system_admin`.

## §8 Design intent that must not be "fixed"

**The community edition has no paywall.** An OAuth signup being born `Denied` — requiring an admin to
enable it — **is** the access-control mechanism, deliberately. It is not a gap, not friction to
smooth, and not a default that fell out of the schema.

- Do **not** change OAuth's provision to `Approved` because new users are locked out. That is the
  feature.
- SAML and machine registration are `Approved` for a *different* reason: the principal was
  provisioned upstream, so the assertion or the registration **is** the grant. Same resulting state,
  two unrelated rationales — do not collapse them into one rule.
- Consequently `/admin/access` is not an inbox, it is **the gate** (task
  `019f7ce2-0b12-7420-b5f1-cb2ce78a743d`).

## §9 The governance seam

Governance is a separate machine and a separate spec. This spec fixes only the invariant between
them, and requires it be **maintained by a transition, never checked at read time**:

- **`admin` implies `Approved`.** Promotion guards on standing being `Approved` — you cannot govern
  an instance you may not use.
- **Revoke and Deactivate fire a demotion in the governance machine, in the same transaction.**

Without this, "admin, but admission revoked" is representable, and `is_system_admin` would have to
consult admission to be safe — reintroducing a cross-table AND in the hottest predicate in the
system. Maintaining it by transition lets `is_system_admin` read one thing and be correct by
construction.

## §10 Persistence

- **`kb_principal_standing`** — one row per principal, the current state. What the SQL predicates
  read.
- **`kb_principal_standing_events`** — append-only: act, actor, prior state, resulting state, reason,
  timestamp.
- **One SQL function per transition** writes the row, appends the log, and emits the `kb_events`
  record in a single transaction, so a standing change without its audit record is not
  representable.

This mirrors the hybrid the repo already chose. Production event emission is SQL-resident — there
are **zero** production `INSERT INTO kb_events` statements in `crates/`; substrate's `events.rs`
describes itself as the firing surface for *"seeding, scenario loading, and tests"*, while *"the SQL
functions stay the atomic event+materialize+commit mechanism."*

Substrate's role here is the **payload wire contract** (`payloads.rs`), one struct per new event
type.

Two mechanical consequences, both easy to trip over:

- Touching `payloads.rs` restales the payload JSON-Schema snapshot. Regenerate with
  `UPDATE_SCHEMA=1 cargo make test-schema`, scoped **`-p temper-substrate`**, never `--workspace` —
  the emitted shape differs under feature unification.
- Every new event type must spell `category` explicitly. The `DEFAULT` was dropped in
  `20260719000010`, so an unstamped registration fails `23502` at apply time.

## §11 Backfill and phasing

**Backfill by evaluating the old predicate, not by reading the tier.** For each profile, run today's
`has_system_access` at migration time: `true` → `Approved`, `false` → `Denied`; `is_active = false`
→ `Deactivated`.

This makes the cutover **behaviour-preserving by construction** on every instance, whatever its
configuration. A tier-based backfill would silently lock out anyone whose access comes entirely from
gating-team membership with `system_access = 'none'` — a population that is one row here and could
be most of an instance elsewhere.

It also gives the one row we *do* want to change a better story: `anonymous` backfills to `Approved`
(it is a gating-team member today), and revoking it becomes a **deliberate, audited transition**
rather than a side effect of a schema change. It cannot authenticate either way, so the stakes are
nil — but it establishes that tightening access is an act, not a migration.

**Deployment surface is small and known:** temperkb.io, plus one enterprise instance with ~12 alpha
testers.

**Phasing follows deployment character, not size.**

1. **Additive** — add the tables, backfill, repoint the predicates, route all writes through the
   machine. Rides auto-deploy safely under the additive-only-on-`main` invariant. `system_access` and
   `is_active` survive as maintained projections so nothing reading them breaks mid-flight.
2. **Destructive** — drop `kb_profiles.system_access`, `kb_profiles.is_active`, and
   `kb_join_requests.status`. Operator-run per target via the cutover runbook. Separate PR.

## §12 Verification

- **The state × act matrix is exhaustively enumerable** — five states × nine acts × actor-authority
  variants, as a table test with no database. Adding a state fails compilation until every cell is
  filled.
- **The backfill gets a differential test.** Seed a population spanning `open`/`invite_only`, member
  and non-member, all three tiers, active and deactivated; assert
  `old_has_system_access(p) == new_has_system_access(p)` for every `p`. Hand-writing the expected set
  is where a subtle case gets missed.
- **SQL totality has its own test** — `has_system_access` and `is_system_admin` return non-`NULL` for
  a profile with no standing row, a deactivated one, and an unknown state value.
- **Containment gets a test per door**, including a minter without standing being refused *at the
  act*, with a reason.
- **The mint split gets one test per path, never one for the pair** — the two doors share a mint
  function and the failure is silent and permissive.

## §13 Open questions

1. **Where the `has_system_access` call sites belong.** The predicate's *definition* is settled; its
   *placement* across Level 3's SQL is not, and may want rethinking once standing exists. Deliberately
   deferred, not forgotten.
2. **Which non-auth `is_active` readers move** versus consume a maintained projection (D6).

## §14 What this supersedes

The regate goal `019f7cdb-a1b6-7e80-b19a-349a3d427671` should be **retargeted, not closed**. Its
D2/D3/D4 decisions survive as inputs; its three-chunk sequence does not, because the predicate no
longer moves to `system_access`. The `access_mode` retirement is still real work and still wants the
additive/destructive split.

Branch `jct/system-access-regate` holds the abandoned Session A at commit `8938a251`. **It does not
compile** (five `query!` sites missing from `.sqlx`, deliberately not regenerated) and must not be
built on. Its three tests in `auth/mod.rs` encode assertions this design still owes.

Unaffected and still filed separately: the machine role cap
(`019f7f12-db7e-77c0-945b-a8c992c03e9d`), `/admin/access` operability
(`019f7ce2-0b12-7420-b5f1-cb2ce78a743d`), and MCP's total absence from the join-request surface.

## §15 Grounding

Verified against `main` @ `8a77bf46` on 2026-07-20: live `\sf has_system_access` is byte-identical to
`migrations/20260624000002_canonical_functions.sql:1388-1406`; `grep` over `crates/*/src` confirms
zero production writes of `system_access` and zero production `INSERT INTO kb_events`;
`handlers::teams::create` sits in `gated_routes()` (`routes.rs:97`), so a team owner necessarily held
system access; `temper-system` carries `auto_join_role = 'watcher'` locally.

**Production state moves and must be re-verified foregrounded before implementation.** `access_mode`
and the `system` profile's tier both changed during the session that filed the original task.
