# Admin-event sink — design

**Date:** 2026-07-16
**Task:** `019f6055-6aea-7aa2-a133-61552dd3d7e4`
**Verdict:** **PAY.** Administration becomes event-sourced. The published claim is made true rather than retired.

---

## §1 — The question, and why it was open

Reader-facing docs state, as settled, that *"Administration is event-sourced (auditable by
construction)."* Five admin surfaces have shipped without writing an event, one of them declining
**in writing, in the DDL** (`migrations/20260714000010_connections.sql:4-9`). The task asked for
exactly one of two outcomes: **RETIRE** the claim, or **PAY** it. A third outcome — leaving it — was
ruled out, because every future admin surface faces the same undecided question and declines it
again. That is how the count reached five.

This spec pays it.

### The RETIRE case, steelmanned and rejected

The strongest argument against was written by the connections migration itself:

> *"It follows this goal's own invariant: the ledger records receipt, never elaboration. An admin
> creating a connection is not a receipt of anything external. The ledger's job is the outside
> world; provisioning is internal infra."*

This is a genuine architectural claim, not a rationalization, and it deserved adjudication on the
merits. It fails on evidence:

- **The ledger already carries system-configuration acts.** Five `lens_created` events exist today
  with a both-NULL producing anchor, emitted by the system actor
  (`crates/temper-substrate/tests/bootseed.rs:48-58`). A lens is not a receipt of anything external
  either. The invariant, as stated, is already not what the ledger does.
- **The design was already specified — twice — and never built.** §3.7 of the access-capability
  spec (`docs/superpowers/specs/2026-06-30-generalized-access-capability-model-design.md:371`)
  states the split this spec implements: *"The `granted_by_profile_id`/`granted_at` columns are the
  per-row provenance; the event log is the temporal record."* The emitters spec
  (`docs/superpowers/specs/2026-07-13-external-systems-as-subscribed-emitters-design.md:467`) calls
  it *"the existing admin-event-sourcing shape."*
- **The absence has a demonstrated cost**, not a hypothetical one. See §2.

The migration's ground 1 — the precedent argument ("it follows the shipped `kb_machine_clients`
precedent") — is exactly the reasoning an adjudication exists to break. Precedent chains that
nobody ever adjudicates are how five declines happen.

**What survives from the RETIRE case, and is honored here:** the *scoping* half. Admin acts must
not become cognition. That is not a reason to refuse the ledger; it is a requirement on how they
ride it. §5 makes it structural.

---

## §2 — What the probe found (live prod, 2026-07-16)

| fact | value |
|---|---|
| `kb_events` total | **13,190** (12,300 on 2026-07-14 — cognition flowing steadily) |
| `grant_created` events | **0** |
| `grant_revoked` events | **0** |
| `kb_access_grants` rows | 5 |
| `kb_machine_clients` rows | 1 |
| `kb_teams` rows | 8 |
| `kb_connections` rows | 0 |

**~14 administrative acts have occurred in prod; exactly zero produced an event.**

The count of declining *surfaces* is not five. A sweep of the service layer found **~20 admin-shaped
acts across 8 services**, none of which emit (§6).

### The demonstrated cost

`kb_access_grants` carries `UNIQUE (subject_table, subject_id, principal_table, principal_id)`, and
its upsert overwrites provenance (`crates/temper-services/src/services/access_service.rs:139`):

```sql
DO UPDATE SET …, granted_by_profile_id = EXCLUDED.granted_by_profile_id, granted_at = now()
```

A grant minted by A in January and re-upserted by B in July reads *"B, July"*. Revocation is a hard
`DELETE` (`access_service.rs:167`) — a revoked grant leaves **no trace at all**: not who granted it,
not who revoked it, not that it existed. `grant_revoked` is a declared event type whose entire
subject matter is destroyed by the operation it names.

B3's reach-affirmation columns (`reach_affirmed_by`/`reach_affirmed_at`/`reach_affirmation`) are the
same defect, reinvented six weeks later by an author with nowhere to write the temporal record. Its
own migration comment concedes it: *"single-valued, last-writer… an audit stamp, not a per-grant
ledger."* Affirm team β then team γ, and β's reason is gone.

**This is §3.7's split, attempted without a ledger to hold the other half.**

### Corrections to the task's own framing

Recorded because the task doc will outlive this session and two of its arguments do not survive
contact with the data:

1. **The `payload_schema` argument is void.** The task doc treats the two admin types' empty
   `payload_schema` as damning — *"EMPTY, unlike every cognition event type (which carry full JSON
   Schema)."* **31 of 33 event types have NULL `payload_schema`.** Only `region_materialized` and
   `lens_created` carry one. The column comment says NULL is legitimate
   (`migrations/20260624000001_canonical_schema.sql:445-447`). Empty payload_schema distinguishes
   nothing.
2. **"Declared but never written" is not unique to admin types.** `block_folded`,
   `property_folded`, `relationship_retracted`, `resource_rehomed` and several more sit at 0. Those
   read as "hasn't happened yet," which is the point — the count alone proves nothing.
3. **`context_reassigned` is NOT the admin-act precedent it appears to be.** It is tempting to cite
   (`context_service.rs:549-561` fires it with `EventContext::default()`) but it is a **cognition
   act**: it anchors to `'kb_contexts', v_context`
   (`migrations/20260715000010_context_reassign_fns.sql:115` — **not** NULL), it is gated by the
   context-owner/`can_share` rule, and its projector mutates `kb_contexts.owner_table/owner_id` — it
   *moves a cognition home*. It is zero evidence that `machine provision` belongs on the ledger. The
   real precedent is `lens_created` (§1).

`EventContext::default()` governs authorship/invocation/correlation. **The producing anchor is a
separate `_event_append` parameter.** Conflating the two is easy and was done during this design;
the anchor rule in §4 exists partly to make it un-conflatable.

---

## §3 — Decisions

| # | Decision | Rationale |
|---|---|---|
| **D1** | **PAY**, not RETIRE | §1. Evidence is `lens_created` + §3.7 + demonstrated cost — *not* `context_reassigned` |
| **D2** | Sink is **`kb_events`**, not a separate `kb_admin_events` | The nullable anchor exists for system acts. A separate table forfeits `correlation_id`, replay, and the claim's literal truth, to dodge readers that never see a NULL-anchored event |
| **D3** | Emit site is **SQL-resident `SeedAction` arms** (event + projection, one txn) | §7 |
| **D4** | Scope is **authority + principal lifecycle**, ~20 acts | §6 |
| **D5** | **No backfill.** Ship an epoch marker | §8 |
| **D6** | The cognition firewall is the **NULL anchor** — no new predicate | §5. Verified, not assumed |
| **D7** | Read path is **`kb_events."references"`**, two axes | §5 |
| **D8** | `kb_access_grants` joins **`INPUT_TABLES`**; projectors are idempotent re-apply | §7 |
| **D9** | Fused acts: **caller mints a `CorrelationId`** and threads it to both SQL fns | §7 |

---

## §4 — The anchor rule

> **The producing anchor is for cognition acts. Every authority act is NULL-anchored, regardless of
> its subject.**

`context_reassigned` is anchored and immortal (append-only). It is not an exception to this rule —
it is **outside** it, because it is cognition: it moves a home. `share`/`unshare` are *not* its
siblings despite touching the same object; they change *who reaches* a context (`kb_team_contexts`)
without moving the home, which makes them authority acts and therefore NULL-anchored.

§3.7 forces this independently: *"No grant ever becomes substrate an agent reasons over."* A grant
with `subject_table='kb_contexts'` does what `share` does. If `share` were anchored and the
equivalent grant NULL-anchored, **the firewall would depend on which API the caller used.** One rule,
no per-act judgement.

**NULL anchor means "no cognition home" — it does NOT mean "admin."** `lens_created` is already in
the NULL bucket. Any future reader that infers "NULL anchor ⇒ system config" will silently absorb
admin events. Readers discriminate by **event type** or by **`references`**, never by anchor
nullity. `bootseed.rs:50` is correct today only because it also filters `et.name='lens_created'`.

---

## §5 — The firewall, and the read path

### The firewall is structural (D6) — verified, not asserted

Every `kb_events` reader was enumerated. All scope by `producing_anchor_table`/`_id`, so a both-NULL
admin event is genuinely invisible to each:

| reader | evidence |
|---|---|
| `steward_ingest_delta` — `new_events` is a bare `count(*)` **but its WHERE scopes by anchor** | `migrations/20260701000005_steward_ingest_watermark.sql:59-63` |
| `replay::touched_since` | `crates/temper-substrate/src/replay.rs:637` |
| `formation_touched_count_since` | `replay.rs:688` |
| `content_touched_resources_since` | `replay.rs:718` |
| `event_service::latest_event_id_for_context` | `crates/temper-services/src/services/event_service.rs:25-26` |
| `db_backend` materialize attribution | `crates/temper-services/src/backend/db_backend.rs:2489` |
| `last_materialize_event` | `replay.rs:609-614` (also filters by type + payload `lens_id`) |

`unified_search` does not read `kb_events` at all. No region producer reads it un-anchored. **No
reader scopes by emitter, `correlation_id`, `invocation_id`, or `occurred_at` without an anchor
filter.**

This upgrades the published claim from *"firewalled by intent"*
(`docs/cognitive-maps/07b-governance-and-administration.md:70`) to **firewalled by construction**.

### …which is exactly why the read path cannot be the anchor (D7)

The firewall that hides admin events from cognition hides them from **every** reader —
`event_service` has only two read functions, both anchor-scoped or payload-shape-matched. A sink with
no reader is not a feature; it is ~100 artifacts writing rows nothing can query. Per the standing
rule that full MCP+API+CLI surface parity is always intended, **the read path is specified first and
the writers are built against it.**

The read path is `kb_events."references"` — `JSONB NOT NULL DEFAULT '[]'`, **GIN-indexed**
(`idx_kb_events_references … USING GIN ("references" jsonb_path_ops)`), documented as *"Typed
provenance pointers: `[{rel, target:{kind,id}}]`"*, and **never written in 9,835 events**. An admin
event is precisely a typed provenance pointer at a subject with no cognition home. The `rel`
vocabulary is a **comment, not a CHECK**, so extending it costs no constraint change.

Each admin event carries pointers at the entities it concerns:

```json
grant_created.references = [
  {"rel": "subject",   "target": {"kind": "kb_contexts", "id": "…"}},
  {"rel": "principal", "target": {"kind": "kb_teams",    "id": "…"}}
]
```

Two axes, both index-backed, both orthogonal to the cognition anchor:

- **"Who was granted what on this subject, and when?"** →
  `references @> '[{"target":{"kind":"kb_contexts","id":…}}]'` (GIN, `jsonb_path_ops`)
- **"What did this admin do?"** → `emitter_entity_id = … ORDER BY occurred_at DESC`
  (`idx_kb_events_emitter`, exists today)

**The firewall holds because no cognition reader consults `references`.** Admin events stay invisible
to maps while being fully queryable by the audit surface. This is the property a payload-key
convention could not deliver.

### The `element_trail` hazard — a declared invariant, with a test

`element_trail_node`/`element_trail_edge`
(`migrations/20260706000002_element_trail_payload_actor.sql:7-52`) have **no type filter**. They match
purely on payload **key shape** — `(payload->>'resource_id')::uuid`, `payload->'owner'->>'table'`,
`payload->>'block_id'`. Their authz gate is `resources_visible_to(p_profile)` (`:47-49`), so **any
reader of a resource would see who was granted access to it** — an authority leak to non-admins.

The live hazard is real: a grant with `subject_table='kb_resources'` *is about* a resource, and
`owner` is a very natural key for an admin payload.

> **Invariant (tested, not conventional):** no admin payload may spell a key `resource_id`,
> `block_id`, `edge_id`, or `owner:{table,id}`. Subjects are spelled `subject_table`/`subject_id`
> and carried in `references`. A test asserts no admin event type ever appears in an
> `element_trail_node`/`element_trail_edge` result.

### Read authorization — **flagged for review**

Proposed: **the read gate mirrors the write gate.** If you could perform the act, you may read the
record of it — `is_system_admin` OR owner of the owning team, reusing `machine_authz::authorize`
verbatim, introducing no new predicate (the move the connections migration itself made). System
settings and `promote_admin` records are `is_system_admin`-only.

This is the one decision in this spec taken without adversarial challenge. It should be confirmed
before the read surface is built.

---

## §6 — Scope: the act catalogue (D4)

All verified as emitting nothing today. Every one takes `caller: ProfileId` at the service layer
already — **the actor needs no plumbing** — except the three noted.

### Authority tier

| act | site |
|---|---|
| `grant_capability` / `revoke_capability` | `access_service.rs:184` / `:213` |
| `insert_grant` / `delete_grant` (the chokepoint) | `access_service.rs:128` / `:159` — **`delete_grant` takes no actor** |
| connection `grant_reach` (+ fused affirmation) | `connection_service.rs:434`, `:458-469` |
| connection `revoke_reach` | `connection_service.rs:532` |
| machine `provision` / `issue` / `rebind` | `machine_registration_service.rs:197` / `:267` / `:337` |
| machine `revoke` / `rotate_secret` | `machine_client_service.rs:124` / `:150` |
| connection `provision` / `revoke` / `attach_credential` | `connection_service.rs:111` / `:215` / `:263` |
| team `change_role` | `team_service.rs:473` |
| `promote_admin` | `access_service.rs:391` — **takes no `caller`** |
| `update_system_settings` | `access_service.rs:310` — **takes no `caller`** |
| cogmap `bind_team` / `unbind_team` | `cogmap_service.rs:27` / `:114` |
| context `share` / `unshare` | `context_service.rs:430` / `:463` |
| join-request `review_request` | `access_service.rs:650` |

### Principal-lifecycle tier

| act | site |
|---|---|
| team `create_team` / `delete_team` | `team_service.rs:83` / `:328` |
| team `add_member` / `remove_member` | `team_service.rs:175` / `:387` |
| invitation `create` / `accept` / `decline` | `invitation_service.rs:37` / `:88` / `:175` |
| SAML `reconcile_idp_memberships` | `saml_provisioning_service.rs:54` — actor is a **system reconciler, not a profile** |

**The three actorless signatures are a plumbing gap, not an auth hole.** Every route in is
authenticated — OAuth, SAML, or M2M — and resolves to a profile and thence to an emitting entity.
The handler holds the caller and simply does not pass it down. Thread the `ProfileId`; that is the
whole fix.

`promote_admin` and `change_role` are the two to prioritize: privilege escalation with no record
that it happened.

---

## §7 — The writers (D3, D8, D9)

### Why SQL-resident

Cognition events are not fired from Rust alongside a Rust write. `fire()` dispatches a `SeedAction`
to a **SQL function that appends the event and does the projection in one transaction**
(`crates/temper-substrate/src/events.rs:535-547` → `_event_append`,
`migrations/20260624000002_canonical_functions.sql:765-787`). Admin acts follow the same shape:
`insert_grant` becomes a thin wrapper over `_admin_grant_created`.

The two rejected alternatives, and why:

- **Rust service-layer, event-only fire.** Rejected. It reproduces the failure under adjudication —
  it makes emitting something each surface must *remember*, and the evidence that surfaces don't
  remember is the five declines. Concretely, `connection_service::grant_reach` **bypasses
  `grant_capability`** and calls `access_service::insert_grant` directly
  (`connection_service.rs:467`, `:486`), so a service-layer sink misses it on day one. It is also a
  layering departure: every existing `fire` arm projects; these would not.
- **Rust chokepoint at `insert_grant`/`delete_grant`.** Rejected. Drift-resistant for grants, but
  misses app-level SQL-originated grants (`cogmap_genesis`, the L0 kernel migration) and helps only
  the grant family.

> **Note the boundary precisely.** The docs already bracket *"a command issued straight to
> Postgres"* as a system-responsibility boundary, not a gap
> (`07b-governance-and-administration.md:71-74`). That bracket covers a DBA at a `psql` prompt. It
> does **not** cover `cogmap_genesis`, which is application code that happens to be plpgsql. The
> SQL-resident sink catches the latter; nothing catches the former, by design.

### Replay ownership (D8)

`kb_access_grants` is in neither `INPUT_TABLES` (`replay.rs:74-100`) nor `PROJECTION_DUMPS`
(`replay.rs:25+`) — replay does not have grants today. It **joins `INPUT_TABLES`**, like
`kb_contexts`, and projectors are **idempotent re-apply** — the shape `context_reassigned` already
uses.

This resolves the revoke question. `delete_grant`'s hard `DELETE` **stays**: the row is the
current-state projection; the ledger is the temporal record (§3.7, exactly). Replay walks
`ORDER BY e.id` — UUIDv7, time-sortable — so `grant_created` re-applies and a later `grant_revoked`
deletes. Net state correct. Pre-epoch grants have no events, so replay leaves them untouched: an
input table is not reconstructed from nothing. **`PROJECTION_DUMPS` was rejected for exactly this
reason** — it would diff the 5 pre-epoch grants as spurious, forever.

`delete_grant` needs the actor threaded to fire `grant_revoked`.

### Correlation (D9)

`grant_reach` fuses the affirmation `UPDATE` and `insert_grant` into one transaction
(`connection_service.rs:454-470`) — *"never affirmation-without-grant or grant-without-affirmation."*
Two SQL fns produce two events, and `_event_append`'s `p_correlation` defaults to
`COALESCE(p_correlation, v_ev)` (`canonical_functions.sql:786`) — **each event self-roots**, so the
fusion is lost by default.

The service fn **mints a `CorrelationId` and threads it to both**, matching the column's documented
purpose (*"groups a multi-event act"*). The read surface groups by it.

### Replay must not break

`replay::replay` (`replay.rs:332-345`) walks every row and hard-fails on an unknown type
(`EventKind::from_canonical_name(&name)` + `?`). **Every admin type needs an `EventKind` variant and
a projector before its first event exists**, or full-ledger replay breaks.

### The honest cost

Not "~20 SQL functions." Per act: a mutation fn + a replay-pure projector half + an `EventKind`
variant + a `kb_event_types` seed row + a payload struct + a JSON-Schema entry. **~100+ artifacts
across ~20 acts.**

`connection_service::provision` (`:111-175`) is the worst case and sets the realistic bar: it
resolves authz **before the tx opens** (deliberately — `:117-119`), runs a slug-uniqueness loop
(`:133`), creates a profile (`:136`), and writes `kb_entities` + `kb_contexts` + `kb_connections`.
Porting means moving the slug loop and typed error mapping into SQL and splitting a **replay-pure**
projector half (no `now()`, no authz — `20260715000010:40-41` is explicit that only the mutation half
authorizes). That is not a thin wrapper.

Sub-attacks that **fail**, and are therefore not obstacles:

- `RETURNING (xmax = 0)` (`access_service.rs:131-140`) ports fine: `… RETURNING (xmax = 0) INTO
  v_inserted`.
- The `CREATE OR REPLACE` cannot-add-a-param trap bites the *next widening*, not new functions. But
  its companion warning applies with force: **a mutation-fn signature change is a write outage
  across deploy skew, and `main` auto-deploys.** Get the signatures right the first time.

### Prerequisite — P0, lands first

`resolve_emitter` is `fetch_one` with no lazy creation (`crates/temper-substrate/src/writes.rs:50`);
its own doc says a new marker *"needs its entity provisioned and backfilled (a migration) before any
caller can send it."* **3 of 6 prod profiles have no emitter entity.** The first live admin act by one
of them 500s.

A migration provisioning emitters for every profile — on `cli` as well as `web`, since the admin
surface is `temper admin …` — **lands before the first fire arm.** This is task 1, not a risk-register
line.

---

## §8 — The epoch, not a backfill (D5)

**No backfill.** A single `admin_ledger_opened` event marks T; the read surface reports *"no admin
history before T."*

Backfilling was considered and withdrawn on evidence:

- **The grant columns are last-writer, not original.** The upsert overwrites both
  `granted_by_profile_id` and `granted_at = now()` (`access_service.rs:139`). Synthesizing events
  from them asserts a **current snapshot with fabricated timestamps and possibly the wrong actor** —
  as immortal, append-only rows.
- **`kb_teams` has no creator column** (`id, slug, name, created, auto_join_role, description,
  is_active`). The eight teams — the largest group of historical acts, and the doc's own lead example
  of a settled admin event — are permanently unreconstructable.
- **`kb_team_members` has no actor** (`team_id, profile_id, role, created, source`).
- **Revoked grants are already gone** (hard `DELETE`).

A partially-backfilled ledger is **worse than an honestly-empty one**: a reader cannot distinguish
"no event" from "predates the writer" from "reconstruction with the wrong actor." An empty ledger
with an epoch is unambiguous.

> `kb_team_members.source` (`team_member_source ENUM ('native','idp')`) **does exist** — added by
> `migrations/20260702000001_saml_group_provisioning.sql:5-7`, not by the origin migration. Reading
> the origin migration says otherwise; the live DB is authoritative. `source='idp'` memberships have
> a genuinely knowable actor (the SAML reconciler). They are **still not backfilled** — the epoch
> rule is uniform, and one class of reconstructable rows does not justify a mixed ledger the reader
> cannot interpret.

**Adding `created_by` to `kb_teams`** is worth a follow-on task so future teams record a creator
independent of the sink. Out of scope here.

---

## §9 — Sequencing

Additive-only on `main`; each step a forward migration.

1. **Emitter backfill migration** (P0 — §7). Every profile has an emitter on `web` and `cli`.
2. **`references` contract + the read surface.** `rel` vocabulary extension (`subject`, `principal`),
   the typed Rust shape, the two query axes, the authz gate (§5 — confirm first), DTO, and
   API + CLI + MCP parity. Reads return only the epoch marker until step 5. **This ships something
   queryable before anything writes.**
3. **The `element_trail` invariant + its test** (§5). Lands before any admin payload exists.
4. **`admin_ledger_opened`** — the epoch marker at T.
5. **The grant chokepoint** — `_admin_grant_created` / `_admin_grant_revoked`, `EventKind` variants,
   idempotent-re-apply projectors, `kb_access_grants` → `INPUT_TABLES`, actor threaded into
   `delete_grant`. This is the proving pair: it catches the generic path **and** `grant_reach`'s
   bypass, and exercises replay ownership end-to-end.
6. **The rest of the authority tier**, then the lifecycle tier (§6).
7. **The doc amendments** (§10) land with step 5 — not before. The claim becomes true when the first
   writer ships, not when the spec merges.

The three actorless signatures (`delete_grant`, `promote_admin`, `update_system_settings`) are
threaded as their acts land.

---

## §10 — Doc amendments required

PAY does not spare the docs. The published claim is wrong in a way RETIRE would not have fixed
either: it names the wrong **mechanism**.

| file | line | issue |
|---|---|---|
| `docs/cognitive-maps/07-operating-temper.md` | 95 | *"settled"* — true only once writers ship (§9 step 7) |
| `07-operating-temper.md` | 96 | *"with an emitter and **a producing anchor**"* — **the anchor must be NULL** |
| `07b-governance-and-administration.md` | frontmatter `description` | same claim |
| `07b` | 17-18 | *"every one of those administrative acts is an event"* |
| `07b` | 58-59 | *"each with an emitter and a producing anchor"* — same wrong mechanism |
| `07b` | 70 | *"firewalled **by intent**"* → **by construction** (§5) |
| access spec | §3.7 | **keep** — it is this design. Note it as unbuilt until now |
| emitters spec | 467 | *"the **existing** admin-event-sourcing shape"* — it did not exist |

The mechanism error matters more than the overclaim: *"with an emitter and a producing anchor"*
describes the **opposite** of what the next sentence promises. A literal implementation of line 96
would anchor admin events to contexts and break the *"do not participate in cognitive maps"*
boundary the same paragraph guarantees. Anyone implementing from the docs would have built the leak.

The `07b` visualization placeholder — admin events flowing into *"a separate channel that does not
feed the cognitive maps"* — survives as-is. NULL-anchoring implements it faithfully.

**The two orphan `kb_event_types` rows stay.** `grant_created`/`grant_revoked` are step 5's types.
They stop being orphans.

---

## §11 — Open questions

1. **Read authorization** (§5) — proposed as write-gate-mirroring, taken without adversarial
   challenge. **Confirm before building the read surface.**
2. **`kb_teams.created_by`** — additive column so future teams record a creator. Follow-on task.
3. **Multi-tenancy** — a self-hosted instance replaying its own ledger inherits §8's epoch semantics
   with different data. The epoch is per-instance. Unexamined.
4. **Should this task link to the ledger-as-readable-surface goal (`019f51e3`)?** The task deferred
   the link pending sub-question 1. Sub-question 1 is answered — the ledger *does* admit admin acts —
   and D7 makes this a ledger-read-surface deliverable. **Link it.**

---

## §12 — What this spec does not claim

- It does not claim administration *is* event-sourced. It claims it will be, per §9, and that the
  docs must not say otherwise until step 7.
- It does not claim the ledger reaches below the application. A `psql` command bypasses it, by
  design (§7).
- It does not claim history is recoverable. **Eight of ~14 historical acts are gone**, and the
  design says so plainly rather than synthesizing them (§8).
