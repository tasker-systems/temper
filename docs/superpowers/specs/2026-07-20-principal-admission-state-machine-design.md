# Principal Admission — a fail-closed state machine

**Date:** 2026-07-20
**Status:** design complete and **planned**. Phase 1's implementation plan is
[`docs/superpowers/plans/2026-07-20-principal-admission-phase-1.md`](../plans/2026-07-20-principal-admission-phase-1.md).
Phase 2 (the drops) is not yet planned.
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
SQL callers of is_system_admin : 1 — context_reassign_fns.sql:76 (in-database, never touches Rust)
Rust call sites                : 21 production, all reaching the SQL fn via a passthrough wrapper
```

One chokepoint — **the SQL function body.** When governance holds its own state, `is_system_admin`
changes body in one place and gating-team ownership stops carrying authorization meaning at all —
the ~20 writers to `kb_team_members` become ordinary team-role churn. There is no cross-machine
invariant left to maintain, because there is no longer a second place where admin-ness lives.

> **Correction (planning, 2026-07-20).** An earlier draft of this block said *"Rust callers: ~12,
> ALL routed through `access_service::is_system_admin` (:44)"*. **Both halves are wrong**, and the
> error is the same shape §2 and §7 correct below — a true observation wired to the wrong object.
>
> There are **21 production Rust call sites**, not ~12 (`db_backend.rs:2030`,
> `connection_service.rs:75`, `admin_ledger_service.rs:80,181`, `cogmap_service.rs:68`,
> `machine_registration_service.rs:379`, `slack_disconnect_service.rs:267`, `machine_authz.rs:51`,
> `context_service.rs:376`, `team_service.rs:150,283`, `machine_client_service.rs:93`,
> `access_service.rs:104,432,915`, `handlers/access.rs:145,163,189,205,223`, `handlers/embed.rs:195`).
> And they are **not** all routed through Rust: `20260715000010_context_reassign_fns.sql:76` calls
> the predicate *in-database*, where no Rust-level audit would ever see it.
>
> **The conclusion survives, via a better object.** `access_service::is_system_admin` (`:44`) is a
> pure passthrough — `sqlx::query_scalar!("SELECT is_system_admin($1)")` — so the real chokepoint
> is the **SQL body**, which is strictly better than the Rust wrapper this spec originally named:
> it covers the in-database caller too. Repointing one body moves all 22 sites at once. D10's
> economics are intact; only the thing to repoint changed.
>
> Worth carrying: naming the Rust wrapper as the chokepoint would have left
> `context_reassign_fns.sql:76` — an `IF NOT` site that fails **open into system admin** (§7) —
> silently unrepointed. *"Which object does this system actually use for this?"* is the question
> that catches it; *"is this claim true?"* is not.

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
| **D3** | The pure machine lives in a **new crate, `temper-principal`**, with no `sqlx` dependency — purity enforced by the compiler, not by convention. It therefore depends on **nothing in this workspace** and holds **no identifiers**; see below. |
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
| **D14** | `Approve` is legal from **`Denied` as well as `Requested`** — machines have no self and can never `Request`, so without this the entire machine surface is a dead end. |
| **D15** | A `Revoked` principal **cannot re-request**. `RequestReview` is a separate self act that sets a **marker and does not move standing** — so a revocation cannot be laundered back to `Denied`. |
| **D16** | **`Reinstate` is dropped.** It was identical to `Approve`-from-`Revoked`, and the log's `prior_state` already makes a reinstatement legible. Eight acts, not nine. |
| **D17** | Machine **credential revocation fires `Revoke` on standing in the same transaction** — one fact, one place, rather than two revocation facts that can drift. |
| **D18** | **`access_mode` is retired**, not left standing beside the machine. Phase 1 removes every reader and the concept; Phase 2 drops the column with the other drops. |

### On D18 — `access_mode` cannot survive alongside standing

*(Added during planning, 2026-07-20. §14 said the retirement was "still real work" but did not say
what this design does with it in the meantime; that gap turned out to be load-bearing.)*

`create_join_request` hard-rejects in open mode:

```rust
// access_service.rs:671-678
AccessMode::Open => return Err(ApiError::BadRequest(
    "System is in open mode — no access request needed".to_string())),
```

Under D11 every door births `Denied` **regardless of mode**, so an `open` instance would mint
principals who are denied and then refuse them the one act that could change it. **A dead end, and
one this design creates.** Leaving `access_mode` in place is therefore not a neutral deferral.

Nothing is lost by retiring it. `access_mode` selected between "everyone has access" and
"gating-team members have access"; standing answers that per-principal, which is strictly more
expressive. `open` is simply the state where every principal happens to be `Approved` — and there is
no longer a single `UPDATE` that can flip a whole instance's access. **That loss of a global switch
is the point, not a regression to compensate for.**

**Phasing:** the *concept* goes in Phase 1 (every reader, the settings flag, the gate-path usage);
the *column* goes in Phase 2 with `system_access`, `is_active`, and `kb_join_requests.status`. That
keeps Phase 1 additive-on-schema and preserves the auto-deploy invariant D10 just recovered.

### On D3 — "no `sqlx`" rules out `temper-core`, and that is a feature

*(Added during planning, 2026-07-20 — the constraint was not anticipated when D3 was written.)*

D3 asks for compiler-enforced purity. The obvious way to write the crate — take `ProfileId` from
`temper-core` — **defeats it silently**, because `temper-core`'s `sqlx` dependency is not optional:

```toml
# crates/temper-core/Cargo.toml:22-30
sqlx = { version = "0.8", features = ["chrono","json","macros","postgres","runtime-tokio-rustls","uuid"] }
```

and `define_id!` (`temper-core/src/types/ids.rs:6-108`) emits `Type`/`Encode`/`Decode`/
`PgHasArrayType` impls **ungated**, so `ProfileId` is sqlx-coupled by construction. A
`temper-principal` that took one would link sqlx transitively and D3 would hold only by wishful
thinking — the exact "enforced by convention" failure D3 exists to prevent.

So the crate depends on **nothing in this workspace** and takes **no identifiers at all**: the
machine judges assembled evidence and every id stays on the seam's side of the boundary. That is
§4's own description of the design (*"`temper-principal` never resolves a credential. It judges
assembled evidence"*), and it satisfies D3 more strongly than D3 asked for. `cargo tree -p
temper-principal | grep -c sqlx` returning `0` is the check that keeps it honest.

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

### On D15 — review is a marker, not a transition

A `Revoked` principal must be able to ask for reconsideration without that request being able to
erase the revocation. The rejected alternative was to allow `Revoked → Request → Requested` and have
`Withdraw` return to the *prior* state — which works, but preserves the audit signal by careful
bookkeeping. **D15 makes it structural instead: there is no path out of `Revoked` except an admin
act, so there is nothing to launder.**

It is also the more honest model. *"Please let me in"* and *"please reconsider your decision"* are
different speech acts with different admin context — a reviewer needs the revocation reason, which a
plain `Request` has no slot for. This mirrors D5's separation exactly: **standing carries state, the
record carries payload.**

Three obligations, each a place this can quietly go wrong:

1. **The marker is never an admission input.** It is an inbox signal only. Admission reads standing
   and nothing else; `Revoked` denies whether or not a review is pending. ANDing the marker into the
   decision would restore precisely the conjunction-across-provisional-facts shape D2 forbids. This
   is stated as an obligation rather than left implied **because it is the tempting change** — a
   future reader will see a pending review and reach for it.
2. **It needs its own duplicate guard.** For join requests, `Requested` standing *is* the duplicate
   guard (D12). A review does not move standing, so it does not inherit one — it needs its own
   (e.g. a unique partial index on `(profile_id) WHERE decided_at IS NULL`). This is what
   `idx_join_requests_one_pending` used to do, reappearing for a different reason.
3. **Its open/decided lifecycle is not a regression on D5.** We remove a status column in D5 and
   reintroduce a status-shaped thing here, so the distinction must be explicit:
   `kb_join_requests.status` was removed because it **duplicated standing**. A review's open/decided
   state duplicates nothing — standing stays `Revoked` throughout, whatever the outcome. Different
   question, so it gets its own answer.

### On D17 — one revocation fact, not two

The pressure test reported machine revocation as a live D2 violation — a cross-table AND of
`kb_machine_clients.revoked_at` with standing. **That was overstated, and the correction matters.**
Credential revocation is rejected at *authentication*, not authorization:

```rust
// profile_service.rs:243-247
if let Some(revoked_at) = client.revoked_at {
    tracing::warn!(client_id, %revoked_at, "machine gate: rejected (revoked client)");
    return Err(ApiError::Unauthorized(...))
}
```

`Unauthorized` is Level 1. A revoked machine never reaches admission, so this is authenticate-then-
authorize — the layered design working, exactly as `is_active` is caught before the gate. (The error
shape is the same one §2 corrects: a true observation — two tables hold revocation facts — wired to
the wrong conclusion.)

What is real is thinner. The two facts can **disagree**: a machine can sit credential-revoked with
standing `Approved` and its grants and memberships intact. That state is inert today — nothing can
authenticate as it, `rebind` refuses a revoked source row, and a fresh `provision` mints a new
profile — so the exposure is *drift*, not a present hole. It also reads badly in audit: the operator
believes the machine is cut off while the ledger says the principal is admitted.

D17 applies the same move D10 made: one fact, one place. `revoked_at` becomes purely an
authentication detail, standing tells the whole story, and the two cannot drift because there is
only one decision. No tenth act is needed — it is `Revoke` with a machine-shaped trigger.

**One prior intent to preserve:** revocation deliberately *leaves* grants and memberships, so a
`rebind` cannot silently resurrect them (`machine_registration_service.rs:381-386`). Firing `Revoke`
on standing denies admission, which sits above grants and does not touch them — so that intent
survives. Confirm it during implementation rather than assume it; this is the one place D17 reaches
into a deliberate earlier decision.

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

> **§6's transition table is authoritative. This diagram is an aid.** The earlier version of this
> sketch showed an edge from `Denied` into `Revoked` that no act produces, and in this repo a
> disagreement between prose and sketch tends to resolve in the sketch's favour. If the two ever
> diverge again, §6 wins.

```
   (no row) ──────────────────────────────────► fail-closed: DENIED
       │
       │ Provision — every door births Denied (D11)
       ▼
   ┌──────────┐    Request     ┌───────────┐
   │  Denied  │───────────────►│ Requested │
   │          │◄───────────────│           │
   └──────────┘  Withdraw      └───────────┘
        │         Reject             │
        │                            │ Approve
        │  Approve (D14 — machines)  │
        └──────────────┐             │
                       ▼             ▼
                  ┌──────────────────────┐
                  │       Approved       │
                  └──────────────────────┘
                             │ Revoke
                             ▼
                  ┌──────────────────────┐   Approve (D16 — no separate Reinstate)
                  │       Revoked        │──────────────────────► Approved
                  └──────────────────────┘
                        ▲        │
                        └────────┘  RequestReview — sets a marker;
                                    standing UNCHANGED (D15)

   Deactivate : any live state ─────► Deactivated
   Reactivate : Deactivated ────────► the prior state, read from the log
```

- **`Denied`** — provisioned, never granted. Where OAuth first-login lands, by design (§8).
- **`Requested`** — has asked for **system** access. Still denied, but the refusal can say so and a
  duplicate is refusable. **Per-principal, with no team dimension** — see D9.
- **`Approved`** — may use the instance.
- **`Revoked`** — *was* granted and lost it. A different sentence to the user and a different signal
  in an audit than never having had it. **Only an admin act leaves this state** (D15): the principal
  may `RequestReview`, which sets a marker and moves nothing.
- **`Deactivated`** — the principal itself is disabled. Prior standing is recoverable from the log,
  so reactivation restores rather than guesses. **Backfilled rows are the exception** — the log
  begins at migration time, so they have no prior state to restore; see §11.

Rejection is deliberately **not** a state: a rejected request returns standing to `Denied` so the
principal may re-request — `join_request_rejection_allows_resubmit` (`access_gate_test.rs:403`)
already expects this — while the request record keeps the `decision_note`. Standing carries state;
the request carries payload.

## §6 Acts and guards

Nine acts. Three actor authorities: **none** (the credential itself is the authority), **self** (the
principal acting on its own standing), and **admin** — an actor for whom `is_system_admin` holds.

> **On "admin" as an actor authority.** *(Rewritten — D10 superseded this note.)* An earlier version
> read *"when governance is out of scope (§2)"* and said a separate governance spec owns how a
> principal becomes admin, with the two meeting only at §9's seam. **D10 removed the seam by
> shipping both machines together**, so there is no other spec and no boundary to keep from becoming
> circular.
>
> What survives is the separation of *questions*, which is the part that mattered: admission asks
> *may you act*, governance asks *may you govern*, and `is_system_admin` reads governance state
> alone — never ANDing across tables at read time (§9). The one direction they touch is
> one-directional and by transition: `Revoke` and `Deactivate` demote, so "admin, but admission
> revoked" is never representable. Promotion guards on standing being `Approved`.

**Eight acts** (D16 dropped `Reinstate`), each with its legal source states and resulting state.
Every cell not listed here is **illegal and refused with a reason** — there is no catchall.

| Act | Actor | Legal from | → Resulting standing |
|---|---|---|---|
| `Provision { path }` | admin / none¹ | *absence only* | `Denied` (boot-seed: `Approved` + admin) |
| `Request` | self | `Denied` | `Requested` |
| `Withdraw` | self | `Requested` | `Denied` |
| `Approve` | admin | `Requested`, **`Denied`** (D14), `Revoked` | `Approved` |
| `Reject` | admin | `Requested` | `Denied` |
| `Revoke { reason }` | admin | `Approved` | `Revoked` |
| `Deactivate` | admin | any live state | `Deactivated` |
| `Reactivate` | admin | `Deactivated` | **prior state, read from the log** |
| `RequestReview` | self | `Revoked` | **unchanged — `Revoked`** (D15) |

¹ `Provision`'s actor is **not** "none" universally: `temper admin machine provision` is admin-run.
Under D11 this grants nothing either way, but the table should not imply an unauthenticated mint
path exists.

**`Reactivate` is the only data-dependent target in the machine.** That is a deliberate property and
worth protecting — D15 exists partly to keep it at one, and a future act whose target depends on
history should be treated as a design smell until argued for.

**Two pipelines, not one.** `Request` is the *consent-capturing* act (D12), and machines have no
terms to accept and no self to act — so:

```
humans   :  Denied ──Request──► Requested ──Approve──► Approved
machines :  Denied ─────────────Approve─────────────► Approved
```

This makes `Requested` a human-only state, and `/admin/access` shows two kinds of row: people who
asked, and machines awaiting a direct grant. That is intended, not an inconsistency to smooth.

**Every self act is illegal from `Deactivated`, and is specified so explicitly** — even though
`gate_resolved_profile` (`auth/mod.rs:242-246`) already makes it unreachable by refusing an
`AuthenticatedProfile` to a deactivated principal. Leaning on another layer to make a cell
unreachable is the cross-layer reasoning that produced this design's original bugs; the table stays
total on its own terms.

**`Revoke` is illegal from `Denied` and `Requested`** — you cannot revoke what was never granted.
§5's diagram shows an arrow *into* `Revoked` originating at `Denied`; no act produces that edge, and
the diagram should be redrawn.

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

`AdmittedPrincipal` is constructible only by the machine: passing Level 2 without having passed
Level 1 stays unrepresentable.

> **Correction (planning, 2026-07-20).** This sentence originally ended *"…preserving the type-state
> guarantee `SystemAuthorized` has today."* **There is no such guarantee today, and the premise was
> false.**
>
> ```rust
> // temper-services/src/auth/mod.rs:263
> pub struct SystemAuthorized(pub AuthenticatedProfile);
> // temper-core/src/types/auth.rs:63-66 — both fields pub
> pub struct AuthenticatedProfile { pub profile: Profile, pub claims: AuthClaims }
> ```
>
> `grep -rn 'impl AuthenticatedProfile\|impl SystemAuthorized' crates/` returns **nothing**. Both
> types have fully public fields and no constructor, so any crate can build either by struct
> literal. `SystemAuthorized`'s own doc comment claims it is *"only obtainable from
> `require_system_access`"* and `gate_resolved_profile`'s claims that constructing an
> `AuthenticatedProfile` without passing the gate *"requires going out of your way"* — both
> describe an intent the types do not enforce.
>
> So `AdmittedPrincipal` must **add** the property (private field, no public constructor), not
> inherit it. This is deliberately scoped to the new type: retrofitting the two older ones touches
> every construction site across both surfaces and is filed separately. **The hazard worth naming
> is the doc comments**, which will keep telling a future reader that a guarantee exists.

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

The **event** half mirrors the hybrid the repo already chose. Production event emission is
SQL-resident — there are **zero** production `INSERT INTO kb_events` statements in `crates/`;
substrate's `events.rs` describes itself as the firing surface for *"seeding, scenario loading, and
tests"*, while *"the SQL functions stay the atomic event+materialize+commit mechanism."*

> **Correction (planning, 2026-07-20).** The three-part shape *"writes the row, appends the log, and
> emits the `kb_events` record"* reads as though it follows an existing pattern. **It does not — the
> repo's pattern is two-part.** Every atomic function in `migrations/` mutates the projection row
> and then calls `PERFORM _event_append(…)`; there is no separate transition-log table alongside a
> ledger emission anywhere. The only log-shaped table is `kb_resource_audits`, which no
> `_event_append` function writes.
>
> The dedicated log is therefore **new construction**, and an implementer should treat it as such
> rather than looking for a template that does not exist. The *event* half has one —
> `20260718000010_admin_grant_fns.sql` and `20260719000020_slack_disconnect_event.sql:144-206`.
>
> **Why both halves are still right**, stated now that they cannot be justified by precedent:
> `Reactivate` must restore the prior state rather than guess it (§5), and reading that from
> `kb_events` would put the admission machine behind the admin-ledger read gate — which dispatches
> per act and answers a different question entirely. One cheap local read beats coupling the gate
> to the ledger.

**One committer, not one per transition.** Read literally, *"one SQL function per transition"* is
nine near-identical functions differing by a string literal — the enumerate-don't-compose shape.
What this clause is *buying* is atomicity: a standing change without its audit record must be
unrepresentable. A single committer that always writes all three in one statement buys exactly that,
in one place rather than nine that can drift. **The legality decision stays in `temper-principal`**;
if the SQL grows a transition table there are two of them, in two languages, and they will disagree.

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

**Backfill by evaluating the old predicate, not by reading the tier.** A tier-based backfill would
silently lock out anyone whose access comes entirely from gating-team membership with
`system_access = 'none'` — confirmed on prod as exactly the `anonymous` row (§15), and potentially
most of an instance elsewhere.

### The rule, with precedence and a total arm

The original three-line rule had **no stated ordering** and **no NULL arm**, and both gaps are
load-bearing. Evaluated in order, first match wins:

| # | condition | standing |
|---|---|---|
| 0 | profile is a **connection profile** | **no row at all** (D7) |
| 1 | `is_active = false` | `Deactivated` |
| 2 | `has_system_access(id) IS TRUE` | `Approved` |
| 3 | otherwise — including **`NULL`** | `Denied` |

Rule 3 is written `IS TRUE … else Denied` rather than `false → Denied` **so that `NULL` is handled
by decision rather than by omission.** Today's predicate returns `NULL` when `kb_system_settings` is
empty (§7), and a rule with only `true`/`false` arms would leave that case to whatever the migration
happened to do.

Rule 0 is not optional. Under `access_mode = 'open'` the old predicate returns `true` for **every**
profile, so a literal per-profile backfill would mint connection profiles `Approved` rows — directly
contradicting D7 and **dissolving the structural safety D7 claims**. There is no discriminator
column on `kb_profiles`; kind is inferable only via `NOT EXISTS (SELECT 1 FROM kb_connections …)`.

**Rules 1 and 2 both match a deactivated principal whose old predicate is true, and the ordering is
the decision.** `Deactivated` wins, because D6 folds `is_active` in and a principal who is disabled
is disabled. The cost is stated honestly below.

### What "behaviour-preserving" does and does not mean

The original claim — behaviour-preserving **by construction**, on every instance — is **too strong,
and the overstatement hid a real contradiction.** §11's rule and D6 cannot both be fully satisfied:
for a deactivated principal with old-predicate-true, the backfill assigns `Deactivated`, so the new
predicate returns `false` where the old returned `true`. On an `open` instance that is *every*
deactivated profile.

The precise claim, which is true:

- **The predicate is not preserved for deactivated principals.** It flips `true → false`,
  deliberately.
- **Auth-observable behaviour *is* preserved**, because `gate_resolved_profile`
  (`auth/mod.rs:242-246`) rejects `!profile.is_active` at **Level 1**, and `require_system_access`
  only accepts an `AuthenticatedProfile` — the type-state makes reaching Level 2 without Level 1
  impossible. No deactivated principal ever reaches the predicate through auth.
- **The behaviour that does change** is in the non-auth callers that pass a third party's id.
  `backfill_auto_join_team` (`WHERE has_system_access(p.id)`) today enrols deactivated profiles and
  will stop; `ensure_auto_join_memberships` and `access_service.rs:914` likewise. These are
  **deliberate, named changes**, not incidental fallout.

**On temperkb.io this cell is empty** — all six principals are `is_active = true` (§15) — so the
change is unobservable there. The enterprise instance is unverified, which is why the rule is
specified rather than assumed harmless.

### Two passes the single rule cannot express

**Pending requests.** The old predicate cannot see `status = 'pending'`, so in-flight requests would
backfill to `Denied` and silently lose their request-ness — `Requested` would be unreachable by the
backfill. A second pass sets `Requested` for profiles with a pending row. **Prod has zero join
requests** (§15), so this is correctness-only there, but the enterprise instance is unverified.

**Governance — a third pass this section originally omitted.** *(Added during planning, 2026-07-20.)*
§11 was written before D10 brought governance into scope, so it enumerates no governance backfill.
**Without one, repointing `is_system_admin` de-admins every existing admin** — and under D11 no door
grants access, so the instance would hold zero admins and have no way to make one. A migration would
lock the operator out of their own instance.

Existing admins are gating-team **owners** under the old definition, so the pass is an
`INSERT … SELECT` over `kb_team_members` where `role = 'owner'` on the gating team, with
`granted_by` left NULL — a migration is not an actor, and inventing one would put a fabricated
attribution on the ledger. **This is the highest-stakes statement in the backfill**, and the
migration should assert the resulting count rather than trust it.

The §9 invariant is worth asserting in the same breath: every governance row must belong to a
principal whose standing is `Approved`. If that fires, the pass admitted someone to govern an
instance they may not use.

**A synthetic genesis log entry.** §5 promises `Reactivate` "restores rather than guesses", but the
standing log begins at migration time — so every backfilled `Deactivated` row has no prior state to
restore, which is exactly the case §5 says cannot happen. The backfill must write a genesis entry
recording the pre-deactivation standing (rule 2 evaluated *ignoring* rule 1), or `Reactivate` is
undefined for every pre-existing deactivated principal.

This matters more than it looks, because the evidence is actively destroyed: `sync_system_membership`
**deletes** auto-join memberships whenever the predicate reads false, so once the predicate is
repointed, a `Deactivated` principal's gating-team membership — the very thing their access was
derived from — is gone and does not come back.

### The three `system_access` writers

`bootseed.rs:32`, `scenario/loader.rs:53`, and `scenario/access/loader.rs:143` write the column
directly (§15) and are **not** test-gated. They must be re-pointed to mint standing rows in Phase 1,
before the column is dropped. The boot-seed is also the genesis admin door (D11), so it is the one
writer that legitimately produces `Approved` + admin.

It also gives the one row we *do* want to change a better story: `anonymous` backfills to `Approved`
(it is a gating-team member today), and revoking it becomes a **deliberate, audited transition**
rather than a side effect of a schema change. It cannot authenticate either way, so the stakes are
nil — but it establishes that tightening access is an act, not a migration.

**Deployment surface is small and known:** temperkb.io, plus one enterprise instance with ~12 alpha
testers.

**Phasing follows deployment character, not size.**

1. **Additive** — add the standing and governance tables, backfill (including both extra passes),
   repoint `has_system_access` and `is_system_admin`, re-point the three `system_access` writers, and
   route all writes through the machines. Rides auto-deploy safely under the additive-only-on-`main`
   invariant. `system_access` and `is_active` survive as projections so nothing reading them breaks
   mid-flight.
2. **Destructive** — drop `kb_profiles.system_access`, `kb_profiles.is_active`,
   `kb_join_requests.status`, and `trg_sync_system_membership`. Operator-run per target via the
   cutover runbook. Separate PR.

> **Why Phase 1 is additive again (D10).** Before governance moved in scope, it was not: writing the
> projection value `approved` fires `trg_sync_system_membership`, which re-derives gating-team role
> from the tier and **strips admin** — measured, `owner → watcher`, `is_system_admin` `true → false`
> (§16 F1). Phase 1 had no compliant exit. Under D10 admin-ness no longer lives in
> `kb_team_members`, so that demotion is cosmetic team-role churn and the projection is safe to
> write. **The trigger was never the problem; the problem was that admin-ness lived in the table the
> trigger writes.**

**Phase 2 must enumerate what the drops break.** Dropping `system_access` breaks **two** SQL
functions whose bodies reference the column — `ensure_auto_join_memberships` and
`backfill_auto_join_team` — plus the **trigger definition** `trg_sync_system_membership`, which
names the column in its `AFTER INSERT OR UPDATE OF system_access` clause. Dropping
`kb_profiles.is_active` breaks **no SQL at all**. Each needs its rewrite specified in the migration,
not discovered at apply time.

> **Correction (planning, 2026-07-20), and it makes Phase 2 materially smaller.** This paragraph
> originally said dropping `is_active` breaks four functions — `can_modify_resource`,
> `context_authorable_by_profile`, `graph_home_contexts`, `resources_visible_to`. **All four read a
> different table's column:**
>
> ```
> can_modify_resource            ::  r.is_active   → kb_resources
> context_authorable_by_profile  ::  t.is_active   → kb_teams
> graph_home_contexts            ::  rr.is_active  → kb_resources
> resources_visible_to           ::  r.is_active   → kb_resources
> ```
>
> Derived two independent ways that agree: `pg_proc` introspection restricted to functions whose
> body mentions **both** `kb_profiles` and `is_active` returns exactly those four, and inspecting
> each shows the alias binds elsewhere; and a separate sweep over `migrations/` finds **no** SQL
> function or view anywhere reading `kb_profiles.is_active`. There is no index or constraint on it
> either.
>
> The list was almost certainly produced by grepping for `is_active` inside functions that mention
> `kb_profiles` — a true observation, wired to the wrong conclusion, for the third time in this
> document. **Checking the alias is the whole difference**, and it is the same discipline that
> separated §7's latent trap from a live exploit.
>
> It also said `sync_system_membership` references `system_access`. Its **body** does not — it calls
> `has_system_access(NEW.id)` and touches only `kb_team_members`/`kb_teams`. The column appears in
> the **trigger**, not the function. Immaterial to the outcome, but Phase 2's migration must drop
> the right object.
>
> Consequence: profile deactivation is enforced **entirely in Rust**, at exactly two sites —
> `auth/mod.rs:246` and `slack_grant_vault_service.rs:214`. That answers §13's open question 2
> empirically (below).

**Order matters within Phase 2.** The trigger's `ELSE DELETE` branch is currently the only automatic
path by which losing access removes gating-team membership. It may only be dropped *after* the
governance machine owns demotion — otherwise there is a window in which a `Revoke` leaves the owner
row intact.

**Dropping `kb_join_requests.status` also drops `idx_join_requests_one_pending`.** That is intended,
not collateral: `Requested` standing is the duplicate guard now (D12), and it is per-principal rather
than per-team, which is more correct under D9. Say so in the migration so the index's disappearance
is not a surprise. Note this leaves review requests needing their **own** guard (D15).

## §12 Verification

- **The state × act matrix is exhaustively enumerable** — five states × **eight** acts (D16) ×
  actor-authority variants, as a table test with no database. Adding a state fails compilation until
  every cell is filled. **This test is writable only because §6 now specifies a resulting state for
  every act**; it could not be written against the original spec, which named eight acts and gave a
  target for one.
- **Every illegal cell asserts a *reason*, not just a refusal.** The point of refusing at the act
  (§3 D2) is that the actor learns why; a test that only checks "not admitted" would pass on a
  silent denial.
- **`RequestReview` must not change admission** — assert that a `Revoked` principal with a pending
  review is still refused, and refused *identically* to one without. This is the D15 obligation that
  a future reader is most likely to break.
- **The backfill gets a differential test — but not the one originally specified.**
  `old(p) == new(p) ∀p` is **unsatisfiable** on any population containing a deactivated profile, by
  the deliberate flip in §11. The test is:
  - `old(p) == new(p)` for every `p` **where `is_active`** — the preservation claim, scoped to where
    it is true;
  - a **separate** assertion that deactivated profiles flip `true → false`, so the intended change is
    pinned rather than merely tolerated;
  - connection profiles get **no standing row** (D7 / rule 0);
  - profiles with a pending request land in `Requested`, not `Denied`.

  **`system_access` is not a dimension of the predicate** — it appears nowhere in
  `has_system_access`'s body. Fanning the population across all three tiers triples its size and
  tests nothing about admission. Keep one representative per tier only to exercise
  `trg_sync_system_membership`, which *does* read it.

  Configurations to run the whole population against: `open`/`gating set`,
  `invite_only`/`gating set`, `invite_only`/`gating NULL`, `open`/`gating NULL`, and
  **`kb_system_settings` empty** (the NULL arm — this one fails against today's function, which is
  the point).
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
   deferred, not forgotten. **Still open.**
2. ~~**Which non-auth `is_active` readers move** versus consume a maintained projection (D6).~~
   **ANSWERED empirically during planning, 2026-07-20 — see §11's correction.** There are exactly
   **two** readers of `kb_profiles.is_active` and both are Rust: `auth/mod.rs:246` (the Level 1
   deactivation gate) and `slack_grant_vault_service.rs:214`. **Zero** SQL functions, views, or
   indexes read it. The question presumed a population of non-auth readers large enough to need a
   policy; there is one, and `slack_link_service.rs:85` documents a third site that deliberately
   does *not* filter on it. Phase 1 leaves both in place reading the surviving column; Phase 2 moves
   them alongside the drop. No projection is needed.

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
obligations, D8) and found six places where the spec was **not implementable as written**. D10–D17
resolved **all six**, and the §11/§5/§12 consequences have now been drafted into those sections.
**The design is complete and Phase 1 is planned** —
[`plans/2026-07-20-principal-admission-phase-1.md`](../plans/2026-07-20-principal-admission-phase-1.md),
17 tasks across 8 beats.

### The planning pass found four more false claims — all in the *grounding*, none in the design

Writing the plan meant re-verifying every symbol this spec names. **The design survived intact; four
of its factual claims did not.** They are corrected in place above rather than dropped, because each
was load-bearing on some piece of work:

| | claim | status |
|---|---|---|
| §2 | "~12 Rust callers, ALL routed through `access_service::is_system_admin`" | **CORRECTED** — 21 sites plus an in-database caller. Conclusion survives via a better object: the SQL body, not the Rust wrapper |
| §7 | "`AdmittedPrincipal` … preserving the type-state guarantee `SystemAuthorized` has today" | **CORRECTED** — no such guarantee exists; both older types have public fields and no `impl` block. The property must be *added* |
| §10 | "row + log + event" reads as an existing pattern | **CORRECTED** — the repo's pattern is two-part; the dedicated log is new construction |
| §11 | "Dropping `is_active` breaks four more" SQL functions | **CORRECTED** — it breaks **zero**; all four read another table's column. Makes Phase 2 materially smaller and answers §13 Q2 |

**Three of the four are the same error shape**, and it is the one this document has now committed
five times counting the pressure test's own F5: *a true observation wired to the wrong object.* Every
one of them survived a reading that asked "is this claim true?" and died to one that asked "**which
object does this system actually use for this?**" §2's callers really do exist; §11's four functions
really do mention `is_active`; §10's atomic functions really are the pattern to copy — for the event
half. The predicate held and the referent was wrong.

Two constraints the spec did not anticipate were also found, and both are recorded above as
decisions rather than as errata: **D3 rules out depending on `temper-core`** (its `sqlx` dep is not
optional), and **D18 retires `access_mode`**, which could not be deferred because D11 turns an
`open` instance into a dead end. §11 also gained a **governance backfill pass** it never had — without
it, repointing `is_system_admin` de-admins the instance and D11 leaves no door to make a new admin.

One finding (F5) was **overstated by the pressure test itself**, and that is recorded rather than
quietly dropped: the same error shape it caught in §2 and §7 — a true observation wired to the wrong
conclusion — it also committed. Adversarial review is not self-verifying either.

| | finding | status |
|---|---|---|
| **F1** | Phase 1 de-admins the instance | **RESOLVED by D10** — gating-owner loses authz meaning, so the trigger's demotion is cosmetic; Phase 1 stays additive and the trigger drops in Phase 2 |
| **F2** | §9's seam costs a 21st writer to `kb_team_members` | **RESOLVED by D10** — no seam; governance holds its own state and `is_system_admin` has one reader to repoint |
| **F3** | Eight of nine acts have no resulting state | **RESOLVED by D14–D16** — §6 now carries a full transition table; §12's matrix test is writable |
| **F4** | SAML `Provision` read literally defeats `Revoke` | **RESOLVED by D11 + D13** — no door grants, and `Provision` fires only on mint |
| **F5** | Machine credential revocation vs standing `Revoked` | **RESOLVED by D17** — and the finding itself was **overstated**: revocation denies at authentication, not authorization, so it was never a cross-table AND. See "On D17" |
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
