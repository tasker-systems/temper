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

> `access_service.rs:574-576` — *"Decoupled from `kb_profiles.system_access` (the auth gate reads
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

**In — governance (changed 2026-07-20, see D10).** Whether a principal may *change the rules*
(`is_system_admin`, promotion, demotion) was originally deferred to a separate spec, with this one
defining only the seam (§9). The pressure test showed the seam does not hold as written: admin-ness
*is* a `kb_team_members` row, so maintaining "admin implies Approved" by transition means adding a
twenty-first uncoordinated writer to the system's most-written table.

Governance is therefore **in scope and lands at the outset**. Admission still asks *may you act* and
governance still asks *may you govern* — two machines, two questions — but they ship together rather
than across a boundary. What this buys is disproportionate to what it costs, and the reason is
narrow: **gating-team ownership has exactly one authorization reader.**

```
SQL readers of is_system_admin : context_reassign (which simply calls it)
Rust callers                   : ~12, ALL routed through access_service::is_system_admin (:44)
```

One chokepoint. When governance holds its own state, `is_system_admin` changes body in one place and
gating-team ownership stops carrying authorization meaning at all — the ~20 writers to
`kb_team_members` become ordinary team-role churn. There is no cross-machine invariant left to
maintain, because there is no longer a second place where admin-ness lives.

**Out — Level 3.** Resource authorization (`resources_visible_to`, `can_modify_resource`, team
roles, cogmap grants) is untouched. It is SQL-resident, large, and has a settled design. A future
dedicated authz layer is plausible and explicitly not in scope here.

> **Correction (pressure test, 2026-07-20).** An earlier draft said Level 3 "keeps calling
> `has_system_access` and does not care that the predicate's basis changed." That is false and the
> error mattered, because it misdirected §7's audit. Neither `resources_visible_to` nor
> `can_modify_resource` references the predicate at all — verified by
> `pg_get_functiondef(...) ILIKE '%has_system_access%'`, both `false`. The complete set of SQL
> callers is three functions, none of them Level 3: `ensure_auto_join_memberships`,
> `sync_system_membership`, and `backfill_auto_join_team`. Level 3 is still untouched by this
> design; it is untouched because it never called the predicate, not because it calls it and does
> not care.

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
| **D10** | **Governance lands at the outset**, not as a later spec. Admin-ness moves off gating-team ownership onto governance state, which has one authorization reader to repoint. |
| **D11** | **Every provision path births `Denied`.** No door grants access. Approval is always a separate, admin-authored act. |
| **D12** | The birth state is `Denied`, **not `Requested`** — `Request` is an explicit act by the principal, because it is what captures terms consent. |
| **D13** | SAML's "the assertion *is* the grant" rationale is **withdrawn**. Identity assertion and access grant are different claims by different parties. |

### On D10 — governance at the outset

Deciding this first dissolves three of the pressure test's six findings rather than solving them
individually (§16). The seam problem (F2) disappears because there is no seam. The Phase 1
de-admin (F1) disappears by a less obvious route: `trg_sync_system_membership`'s harm was demoting
the gating-team owner, and once that role carries no authorization meaning, the demotion is
cosmetic. **Phase 1 can then write projections freely and the trigger can be dropped in Phase 2
where it belongs — the additive/destructive split survives.** What forced Phase 1 to be destructive
was never the trigger; it was that admin-ness lived in the table the trigger writes.

### On D11 — every door births `Denied`

The uniform rule retires machinery rather than adding it. **Minter containment (§6) becomes
unnecessary, not relocated**: a minter who cannot confer access is moot when minting never confers
access. That guard — and the escalation it failed to prevent — is what began this entire design arc,
and D11 ends it by removing the thing it was guarding.

It also closes F4 structurally. A revoked SAML principal re-asserting cannot be silently
re-approved, because `Provision` no longer grants under any path.

The genesis case is the deliberate exception: on a fresh instance no admin exists, so nobody could
ever be approved. The boot-seed mints the first admin, and that is **load-bearing rather than a
loose end** (F6). This is accepted, not worked around — bootstrapping temper already requires
database write access, and the bootstrap SoP and scripts foreground that reality.

### On D12 — `Denied`, not `Requested`

`kb_join_requests` carries consent:

```
accepted_terms_version  varchar(32)
accepted_terms_at       timestamptz
source                  varchar(16) NOT NULL
```

Birthing a principal into `Requested` would produce a standing state whose paired record has no
terms acceptance — forcing either fabricated consent or an empty request row that lies about having
been requested. **Terms cannot be accepted on someone's behalf as a side effect of an IdP
assertion.** The `source` column makes the same point: the record is designed to say where the
asking came from, which presumes an asking happened.

Three parties, three acts: the IdP asserts *who you are*; only the principal accepts *the terms*;
only an admin grants *access*. Born-`Requested` collapses the first two.

The usual argument for born-`Requested` — that a principal landing on a bare 403 has no path
forward — is answered elsewhere: `Refusal` is a typed enum (§7), so `Denied` refuses with *"you may
request access"* and `Requested` with *"your request is pending."* That messaging distinction is the
real justification for `Requested` existing as a state, and it only works if the two states mean
different things.

`Requested` thereby earns a second job: with D5 moving the status column onto standing, it **is**
the duplicate-request guard — which matters, because dropping `status` also drops
`idx_join_requests_one_pending`. A per-principal standing state replaces a per-team partial index,
which is strictly more correct under D9.

*Terms are unconfigured today* (`terms_version` and `terms_resource_uri` are empty on both prod and
local), so `Request` must handle "no terms configured" without blocking, and the acceptance columns
stay nullable.

### On D13 — why the SAML rationale was withdrawn

The original §8 justified SAML → `Approved` on the grounds that the principal was provisioned
upstream. That conflates two different assertions by two different parties:

> *"It's one thing for SAML to say 'our org and IdP say you can use this', and another for the
> in-app version to say 'we agree and now you have access' — and I shouldn't conflate them, because
> it's across that precise boundary that interception and access escalation or mismanagement
> happen."*

Team assignment already works this way: SAML does not auto-assign teams; a person is invited by an
admin or a team owner. Auto-granting *system* access on an IdP assertion was the odd one out, and
there is no articulated JTBD spec for auto-SAML provisioning to justify it.

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

**Under D11 the doors no longer diverge — every path births `Denied`:**

| path | resulting standing | guard |
|---|---|---|
| SAML assertion | `Denied` | none needed — the assertion establishes identity, not access (D13) |
| OAuth first-login | `Denied` | none |
| Machine registration | `Denied` | none needed — containment is retired (D11) |
| Boot-seed (genesis) | `Approved` + admin | **the deliberate exception** — mints the first admin; see D11 |

This is the reason the uniform rule is worth having. **SAML and OAuth share one mint function**
(`create_new_profile_and_link`, reached from both `resolve_federated_human` and `authenticate`), and
under the old design the two doors had to diverge *at a shared site* — a constant at that site would
have been permissive and silent, opening the instance to anyone who could sign in, with nothing to
notice. A uniform birth state removes the divergence entirely, so there is no per-door constant left
to get wrong.

**`Provision` fires only on profile mint, never on a returning principal.** An existing auth link
returns at step 1 of `resolve_human_from_claims` and never reaches the mint; a returning principal's
standing is **loaded, not set**. Stated explicitly because the earlier per-*assertion* wording made
`Revoke` defeatable on the SAML door (F4) — D11 closes that structurally, and this sentence keeps it
closed if a future door is added.

> **Correction to the actor column.** `Provision`'s actor authority is listed above as *"none — the
> credential is the authority."* That is wrong for machines: `temper admin machine provision` is
> **admin-run**. Under D11 this matters less than it did (provision grants nothing either way), but
> the act table should say so rather than imply an unauthenticated mint path exists.

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

`has_system_access` stays a SQL function reading the standing table — **not** because Level 3 calls
it (it does not; see the correction in §2), but because the Level 2 gate and three SQL callers do.
It **must** be written as `EXISTS(SELECT 1 … WHERE state = 'approved')`, not
`SELECT state = 'approved' FROM …`. With no matching row the latter returns **`NULL`, and `NULL` is
not `false`**. `EXISTS` is total.

**The hazard is specific to one caller shape, and that is where the audit must look.** Measured
against local dev on 2026-07-20 (this table is a measurement, not an argument):

| caller shape | scalar form, no row | `EXISTS` form, no row |
|---|---|---|
| plpgsql `IF NOT <pred> THEN RETURN` | **falls through — proceeds** | guard fires — denies |
| `WHERE <pred>` | row excluded | row excluded |

A `NULL` in a `WHERE` clause is fail-**closed** — the row drops out either way. A `NULL` in
`IF NOT` is fail-**open**. So the obligation is not uniform across call sites, and an audit that
sweeps every caller equally will waste effort in the safe places and can still miss the dangerous
ones. There are exactly **two** `IF NOT` sites in the repo and both must be audited:

- `20260629000002_auto_join_team_generalization.sql:44` — `IF NOT has_system_access(...)`
- `20260715000010_context_reassign_fns.sql:76` — `IF NOT is_system_admin(...)`, which fails open
  into **system admin** — the higher blast radius of the two

The same obligation therefore applies to `is_system_admin`, at that specific site.

**This is demonstrable on the current functions, not only a future risk.** Both are shaped
`SELECT … FROM settings`; an empty `kb_system_settings` yields no rows, so both return `NULL` today
and both guards above fail open. It is **not presently reachable**: the row is seeded by
`20260624000003_canonical_seed.sql:23`, pinned by `CHECK (id = 1)`, and every production writer is
an `UPDATE` — there are no `DELETE`s. So this is a latent trap the new table must not inherit,
**not** a live exploit. (Stated precisely because the reachability check is what separates the two,
and skipping it is how a fixture gets mistaken for a production-reachable state.)

## §8 Design intent that must not be "fixed"

**The community edition has no paywall.** An OAuth signup being born `Denied` — requiring an admin to
enable it — **is** the access-control mechanism, deliberately. It is not a gap, not friction to
smooth, and not a default that fell out of the schema.

- Do **not** change OAuth's provision to `Approved` because new users are locked out. That is the
  feature.
- **Do not restore SAML's auto-approval** (D13). An earlier draft of this spec had SAML and machine
  registration born `Approved` because "the principal was provisioned upstream, so the assertion or
  the registration **is** the grant." That rationale was withdrawn by its own author. An IdP
  asserting *"our org says this person may use this"* and the instance deciding *"we agree, and now
  they have access"* are different claims by different parties, and **it is across exactly that
  boundary that interception and access escalation happen.** Team assignment already respects this
  boundary — SAML does not auto-assign teams — and system access was the odd one out.
- A future reader with an inbox full of pending SAML users will be tempted to auto-approve them.
  That temptation is what this bullet exists to refuse. If auto-provisioning is ever wanted, it needs
  an articulated JTBD spec first; there is none today, and its absence is why the shortcut was a
  mistake.
- Consequently `/admin/access` is not an inbox, it is **the gate** (task
  `019f7ce2-0b12-7420-b5f1-cb2ce78a743d`), and under D11 it is now the **only** way any principal
  gains access.

## §9 Governance — no longer a seam (D10)

**This section previously specified a seam between two separately-shipped machines. D10 removed the
seam by shipping them together; what follows is what replaces it.** The original text required the
invariant be "maintained by a transition, never checked at read time" — sound in the abstract, and
false in this schema, because admin-ness *is* a `kb_team_members` row (F2).

The invariant is unchanged:

- **`admin` implies `Approved`.** Promotion guards on standing being `Approved` — you cannot govern
  an instance you may not use.
- **Revoke and Deactivate demote**, so "admin, but admission revoked" is never representable.

What changes is where it is enforced. Governance holds **its own state**, so:

- `is_system_admin` reads governance state directly. It never consults admission at read time, and
  it never ANDs across tables — the property the old seam was trying to buy, obtained by
  construction instead of by discipline.
- Gating-team ownership **stops being an authorization fact**. It becomes an ordinary team role.
  The ~20 uncoordinated writers to `kb_team_members` (`promote_admin`, `ensure_auto_join_memberships`,
  `sync_system_membership`, `enroll_in_gating_team`, `apply_reach`, the `team_service` member
  operations, `saml_provisioning_service`, and the rest) stop being able to alter anyone's authority,
  because there is no longer authority stored there to alter.
- `promote_admin`'s raw INSERT is retired in the same change. Under the old design this was a
  *requirement* for the invariant to hold (one writer maintaining it against nineteen breaking it);
  under D10 it is merely cleanup, because the row it writes no longer confers anything.

**The pre-existing demotion bug dies with it.** Today `promote_admin` writes the membership but
deliberately not the tier, while `ensure_auto_join_memberships` derives the role *from* the tier —
so they disagree, the tier wins, and a promoted admin is silently demoted `owner → watcher` by any
join-request approval (§16). Once admin-ness is not a team role, there is nothing for the two
writers to disagree about.

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
  `UPDATE_SCHEMA=1 cargo make test-schema`. The task is **already** `-p temper-substrate`-scoped
  (`tools/cargo-make/main.toml:39`, which says so in its own description) — you do not add the flag.
  What you must not do is run the schema tests under `--workspace`: the emitted shape differs under
  feature unification.
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
- **Containment is retired (D11), so its tests change shape.** There is no longer a minter-standing
  guard to test. What replaces it is the stronger assertion: **no provision path, under any actor,
  yields `Approved`** — one test per door, plus a test that a machine minted by an admin is still
  born `Denied`. That is a property of the whole surface rather than of one guard, and it fails
  loudly if a future door is added carelessly.
- **The mint split gets one test per path, never one for the pair** — the two doors share a mint
  function. Under D11 both must birth `Denied`, so this test now guards *uniformity* rather than
  *divergence*, but the reason for testing each path separately is unchanged.
- **`Provision` on a returning principal must not touch standing** — assert that a `Revoked` SAML
  principal re-asserting through the IdP stays `Revoked`.

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

Verified against `main` @ `8a77bf46` on 2026-07-20, then re-verified under adversarial pressure test
the same day. Two of the original grounding claims did not survive; they are corrected here rather
than quietly dropped.

**Holds as written:**

- live `\sf has_system_access` **body** is identical to
  `migrations/20260624000002_canonical_functions.sql:1388-1406` ("byte-identical" was loose — psql's
  reconstructed header differs in case and schema-qualification)
- `handlers::teams::create` sits in `gated_routes()` (`routes.rs:97`), so a team owner necessarily
  held system access
- `temper-system` carries `auto_join_role = 'watcher'`

**Corrected — `system_access` writes.** The claim "zero production writes of `system_access`" is
**wrong**. Three non-test-gated writers exist, all on temper-substrate's seed/scenario path:

```
scenario/bootseed.rs:32        INSERT … (handle, display_name, system_access) VALUES ('system','System','admin')
scenario/loader.rs:53          (from scenario YAML)
scenario/access/loader.rs:143  (from scenario YAML)
```

What actually holds is narrower: there is no `system_access` write on any **request** path. These
three must be re-pointed to mint standing rows, and **the boot-seed is a provision door that §6's
act table does not list** — see §16.

**Corrected — the `kb_events` grep.** "Zero production `INSERT INTO kb_events`" is right in
substance but describes a grep that does not produce that result: `crates/*/src` returns four hits,
**all** inside `#[cfg(all(test, feature = "test-db"))]` modules. State it that way. As originally
written it invites the next reader to run the grep, get four hits, and conclude the spec is wrong.

### Production, re-verified foregrounded on 2026-07-20

| | |
|---|---|
| `access_mode` | **`invite_only`** — it moved again; earlier drafts reasoned against `open` |
| gating team | `temper-system`, `auto_join_role = 'watcher'`, 6 members |
| principals | 6 total — 2 `admin`, 3 `approved`, 1 `none`; **every one `is_active = true`** |
| old predicate | **`true` for all 6** → backfill lands 6 × `Approved`; no `Denied`, no `Deactivated` |
| `anonymous` | tier `none`, gating member, old predicate **`true`** — §11's D8 case, confirmed at exactly one row |
| `kb_join_requests` | **zero rows** — D5's status-column drop has no data to migrate |

**Why the join-request table is empty, and why that matters.** Prod was never really wired up for
join requests: the instance was left `open` until the two other alpha-tester accounts joined, then
closed to `invite_only`. Nobody has ever submitted one. So the request surface can be reshaped
freely — D5's status-column drop, the loss of `idx_join_requests_one_pending`, and the D12 rework of
`Request` all land on an empty table with no migration story and no user-visible regression. This is
unusual latitude and should be used now rather than assumed to persist.

Two consequences worth carrying into planning. **D8 is vindicated empirically:** `anonymous` has
access purely via membership with tier `none`, so a tier-based backfill would have locked it out,
exactly as §11 predicted and at exactly the predicted cardinality. And because every prod principal
is active, the deactivated-principal defect (§16) has **no affected rows on temperkb.io** — the
enterprise instance remains unverified.

**Production state moves and must be re-verified foregrounded before implementation.** `access_mode`
has now changed twice across the sessions that produced this design.

## §16 Pressure-test findings — status

The 2026-07-20 pressure test confirmed the design intent (the two-machine split, the fail-closed
obligations, D8) and found six places where the spec was **not implementable as written**. D10–D13
resolved four of them. **Do not write an implementation plan that silently picks an answer to the
two that remain.**

| | finding | status |
|---|---|---|
| **F1** | Phase 1 de-admins the instance | **RESOLVED by D10** — gating-owner loses authz meaning, so the trigger's demotion is cosmetic; Phase 1 stays additive and the trigger drops in Phase 2 |
| **F2** | §9's seam costs a 21st writer to `kb_team_members` | **RESOLVED by D10** — no seam; governance holds its own state and `is_system_admin` has one reader to repoint |
| **F3** | Eight of nine acts have no resulting state | **OPEN** — still the largest gap; §12's table test cannot be written until it closes |
| **F4** | SAML `Provision` read literally defeats `Revoke` | **RESOLVED by D11 + D13** — no door grants, and `Provision` fires only on mint |
| **F5** | Machine credential revocation vs standing `Revoked` | **OPEN** — still a cross-table AND, still needs a rule |
| **F6** | The boot-seed is a fourth provision door | **RESOLVED by D11, and promoted** — it is now the load-bearing genesis exception, deliberately accepted |

The pre-existing admin-demotion bug (below) is **superseded by D10** rather than fixed: once
admin-ness is not a team role, the two disagreeing writers have nothing to disagree about. It may
still be worth filing if the fix is wanted before this design ships.

> **Historical note — the original text of the four resolved findings is kept below**, because the
> reasoning that produced D10–D13 is only legible against the problem it solved. Do not read them as
> open work.

**F1 — §11 Phase 1 de-admins the instance.** §11 calls `system_access` a "maintained projection so
nothing reading them breaks mid-flight." It is not inert. `trg_sync_system_membership` fires
`AFTER INSERT OR UPDATE OF system_access` and re-derives gating-team role from the tier via
`ON CONFLICT … DO UPDATE SET role = EXCLUDED.role`. Measured in `BEGIN/ROLLBACK` on local dev:
writing the admission value `approved` onto a gating-team owner flips `owner → watcher` and
`is_system_admin` `true → false`; writing `admin` preserves both. So Phase 1 has no compliant exit —
write `approved` and it de-admins (prod has 2 admins), write `admin` and the admission machine
encodes governance state, collapsing §2's separation. The honest fix (drop the trigger in Phase 1,
move demotion to governance) is **destructive**, which breaks the additive/destructive split Phase 1
is built on. *That consequence is the decision.*

**F2 — §9's seam is not free.** `is_system_admin` reads a `kb_team_members` row with
`role = 'owner'`, so "fire a demotion in the same transaction" *is* a write to `kb_team_members` — a
table with roughly twenty uncoordinated writers and **no `demote_admin` anywhere**. §1 diagnoses the
disease as uncoordinated call sites; §9 prescribes adding one more and does not say so. Salvageable
only if *all* gating-owner writes funnel through the governance machine and `promote_admin`'s raw
INSERT is retired in the same change. §9 must state this cost.

**F3 — §6 is not yet a state machine.** Eight of nine acts have **no specified resulting state**;
only `Provision` has one. §12 promises an exhaustive state × act table test, but there is nothing to
test against — the spec never says where `Reinstate` goes. §5's diagram and prose also disagree on
it (the left rail's arrowhead lands *into* `Revoked`, an edge no act produces), and
`Reinstate → Approved` vs `Reinstate → Denied` are materially different products. §6 needs a
resulting-state column, filled for all nine acts, plus refusal semantics for illegal pairs.

**F4 — §6 read literally defeats `Revoke` for SAML.** "SAML assertion → `Approved`, guard: none" is
worded per-*assertion*, not per-*mint*, so a revoked SAML user logs in and is silently re-approved.
The code makes the safe reading available — `resolve_human_from_claims` returns at step 1 for an
existing link and never reaches the mint — but the spec must say `Provision` fires **only on profile
mint**, and a returning principal's standing is *loaded*, never *set*.

**F5 — machine-client revocation and standing `Revoked` are two independent revocations.**
`kb_machine_clients.revoked_at` deliberately leaves memberships and grants (D11), so "may this
machine act?" still requires ANDing `revoked_at IS NULL` with standing. D2 is satisfied for humans
and violated for machines. Needs either a tenth act or an explicit rule that credential revocation
fires `Revoke` on standing in the same transaction.

**F6 — the boot-seed is a fourth provision door.** Per §15's correction, `bootseed.rs:32` mints the
`system` profile outside the machine. §6 lists three doors. Either it lists four, or §11 states that
the boot-seed is covered by the backfill.

### Also required in §11, independent of the forks

- **Clause precedence and a NULL arm.** §11's rules (`true → Approved`, `false → Denied`,
  `is_active = false → Deactivated`) have no stated ordering, and both apply to a deactivated
  principal whose old predicate is true. There is **no ordering that satisfies both §11's
  preservation claim and D6** — they are contradictory requirements, not an underspecified detail.
  The claim "behaviour-preserving **by construction**" must be narrowed to what is true: the
  *predicate* is not preserved for deactivated principals, but *auth-observable* behaviour is,
  because `gate_resolved_profile` (`auth/mod.rs:242-246`) rejects `!is_active` at Level 1 and the
  type-state makes it impossible to reach Level 2 without passing Level 1. The named,
  deliberate behaviour changes are in the three non-auth callers that pass a third party's id.
- **§12's differential test is unsatisfiable as written.** `old(p) == new(p) ∀p` cannot pass on any
  population containing a deactivated profile. It must be `∀p WHERE is_active`, plus a separate
  assertion that deactivated rows flip `true → false` intentionally.
- **Connection profiles must be excluded explicitly.** Under `open` the old predicate is `true` for
  every profile, so a literal per-profile backfill mints connection profiles standing rows —
  contradicting D7 and dissolving the structural safety D7 claims. There is no discriminator column;
  kind is FK-inferred only.
- **Backfilled-`Deactivated` has no log to restore from** (the log begins at migration time) while
  `sync_system_membership` deletes the underlying membership evidence — so §5's "recoverable, not
  guessed" is false for exactly those rows.
- **A pending-request second pass.** The old predicate cannot see `status = 'pending'`, so in-flight
  requests backfill to `Denied` and silently lose their request-ness; `Requested` is unreachable by
  the backfill as specified.

### Pre-existing bug found while testing (not caused by this design)

`promote_admin` writes the gating-team membership but deliberately **not** the tier (the §1
decoupling), while `ensure_auto_join_memberships` derives the role **from** the tier. They disagree
and the tier wins: an admin promoted via `promote_admin` (tier left `none`) is silently demoted
`owner → watcher` by any call that runs the auto-join sync — including every join-request approval
(`access_service.rs:877-895`). Verified in `BEGIN/ROLLBACK` against local dev. Prod is not currently
exposed because both its admins happen to carry tier `admin`, so the derivation restores `owner`; it
bites the next admin promoted through `promote_admin`. **Worth filing separately** — this design
does not fix it, and F1 widens it.
