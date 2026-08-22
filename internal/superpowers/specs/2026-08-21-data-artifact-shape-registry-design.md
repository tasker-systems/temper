# Design spec: The data-artifact shape registry — home, key, authority, and what a verdict means

**Status:** design ruled, not implemented. Precedes an implementation plan.
**Goal:** Structured data is stored as structured data — `01a02163-ba6a-7b00-91f5-5f416e43f4f6`
**Predecessor design:** `internal/superpowers/specs/2026-08-20-resource-owned-data-artifacts-design.md`
(vault research `01a02163-8670-7cc2-96a6-1a520ec8a0f8`)
**Grounded against:** `main` at `7d2e74ed`

This spec closes the question the artifact-store design left open, and only that question. The
storage substrate, read path, API, MCP and CLI surfaces for *committing and retrieving* artifacts
already shipped (PRs #738, #740, #742, #746, #750, #751). Nothing here changes them.

---

## 1. The finding: the blocking item was two questions, and only one was open

The goal register's Remainder carries this as a single unexamined item:

> **Registry tenancy and registration authority — unexamined, and blocking for the shape registry.**
> Whether a family's shape is global, team-scoped, or context-scoped, and which principal may
> declare or amend one, was raised and never resolved.

Against disk that is two questions, and the first was decided and shipped three weeks before the
register recorded it as open.

**The namespace was ruled on 2026-08-20 and is load-bearing in DDL.**
`migrations/20260820000020_data_artifacts.sql:17-19`:

```sql
kind_owner_table      VARCHAR(64) NOT NULL
                          CHECK (kind_owner_table IN ('kb_profiles', 'kb_teams')),
kind_owner_id         UUID NOT NULL,
artifact_kind         TEXT NOT NULL,
```

Context-scoped is excluded by that CHECK. A flat global namespace is excluded by construction —
no unqualified form exists. The predecessor spec labels it `**Decided 2026-08-20.**`
(`2026-08-20-resource-owned-data-artifacts-design.md:149`) and the reason is carried in the
committed `payload_schema` inside the database itself, on `KindOwner`:

> "Registering a shape for a family validates the existing backlog and records a conformance
> verdict against every artifact of that family — so under a flat namespace one tenant's
> registration would stamp verdicts on another tenant's data, which it cannot even read.
> Qualification makes that impossible by construction rather than by a resolution rule."

**What was genuinely open** is what the same spec names at line 192:

> **Still open: registration authority.** *Which* principal may register or amend a family within
> an owner's namespace is not decided here. Qualification bounds the blast radius to one namespace;
> it does not say who may act inside it.

The register's Remainder is amended accordingly in §9.

---

## 2. Two axes, and the predecessor design only settled one

`(kind_owner, artifact_kind)` answers *whose name this is*. It does not answer *where the registry
entry lives*, and in this codebase those are separate axes with separate value sets.

| Axis | Values | Answers |
|---|---|---|
| **Home** — navigation, visibility, event anchor | `kb_contexts` \| `kb_cogmaps` | Where the thing lives and how it is reached |
| **Owner / grantee** — namespace, standing | `kb_profiles` \| `kb_teams` | Whose name it is, who may act |

The codebase draws this distinction explicitly, at
`migrations/20260624000001_canonical_schema.sql:293-295`:

> "Cogmaps read via the `resources_accessible_to_cogmap` intersection (never per-resource grants);
> **a context is a navigation home, not a grantee.**"

---

## 3. Ruling 1 — a shape entry is homed, polymorphically over `(kb_contexts, kb_cogmaps)`

**CONFORM.** This is forced by the schema, not chosen.

Declaring a shape must be an event: the goal clause `the-governance-record-outlives-the-data`
requires it, and the predecessor spec gives the registry row an `asserted_by_event_id`
(`2026-08-20-resource-owned-data-artifacts-design.md:140-144`). An event cannot anchor to a
profile or a team:

```sql
-- migrations/20260624000001_canonical_schema.sql:470
producing_anchor_table VARCHAR(64) CHECK (producing_anchor_table IN ('kb_contexts', 'kb_cogmaps')),

-- migrations/20260624000001_canonical_schema.sql:279
anchor_table          VARCHAR(64) NOT NULL CHECK (anchor_table IN ('kb_contexts', 'kb_cogmaps')),
```

**The pattern to copy is `kb_connections`**, a first-class non-resource table that carries both
axes at once (`migrations/20260714000010_connections.sql:26,32`):

```sql
owner_team_id            UUID            NULL REFERENCES kb_teams(id),
home_context_id          UUID        NOT NULL REFERENCES kb_contexts(id),
```

with `owner_team_id` commented *"NEVER consulted for authorization"* and `home_context_id`
commented *"Contexts-as-home also means read authz inherits for free."*

**Cogmaps are included, not deferred.** `_data_artifact_anchor`
(`20260820000020_data_artifacts.sql`) resolves cogmap homes deliberately, preferring them over
contexts, so artifacts owned by cogmap-homed resources demonstrably exist. A context-only registry
would leave those artifacts permanently `NeverDeclared` with no way to change it — a hole that
would have to be declared in the register rather than left silent. Making the home polymorphic
costs one extra authority arm that is **already written** (§5).

The column pair follows the established precedent at
`migrations/20260712000030_region_anchor_expand.sql:14-17`, which added exactly this shape to four
region tables:

```sql
home_anchor_table VARCHAR(64) CHECK (home_anchor_table IN ('kb_contexts', 'kb_cogmaps')),
home_anchor_id    UUID
```

---

## 4. Ruling 2 — the shape in force is keyed per home, not per owner

**EXTEND.** Authorized by the predecessor spec's open item (line 192), which leaves the
inside-a-namespace question to be decided.

```
UNIQUE (home_anchor_table, home_anchor_id, kind_owner_table, kind_owner_id, artifact_kind)
```

**Why not `(kind_owner, artifact_kind)` alone.** Under a per-owner key, a shape declared in a
context you cannot read would still govern your artifacts — *your data shape could be rejected for
reasons you cannot see*. Qualification bounds the blast radius to one namespace, but a team
namespace is not a visibility boundary: a team owns contexts that only some of its members can
read.

**What this trades away, deliberately.** The team-defaulted `kind_owner` in
`_data_artifact_kind_owner` exists so that "each member mints families privately and the team never
converges on a shared shape" cannot happen. Under a per-home key, team-wide convergence becomes an
**adoption-of-consistency practice rather than a forcing function**. In practice a context-specific
shape is the common case. `kind_owner` keeps its job — it still prevents *name collision* across
tenants, and it is still frozen into the commit payload — it simply no longer determines *which*
shape governs.

**Consequence, stated rather than discovered later.** `resource_rehome`
(`_project_resource_rehomed`, `20260624000002_canonical_functions.sql:1109`) moves a resource
between homes. Under this key that changes the shape in force over the resource's artifacts without
touching the artifacts at all. This is correct under the ruling's own logic — the shape that governs
your data is the one where your data lives — but it makes shape-state a property of the datum *and
its resource's current home*, which amends the register's equivalence claim 3 (§9).

---

## 5. Ruling 3 — authority is an incumbent gate, with no new RBAC surface

**CONFORM.** Declaring or amending a shape requires authority over its home:

```
kb_contexts → context_authorable_by_profile(p_profile, home_anchor_id)
kb_cogmaps  → cogmap_authorable_by_profile(p_profile, home_anchor_id)
```

This is not merely convenient, it is **sufficient**, and the reason is the container-write cascade
(`migrations/20260706000003_container_write_cascade.sql:5-10`):

> "Unix directory semantics: whoever may write a container may modify (and supersede) any node
> homed in it, regardless of the node's own owner/originator."

Because ruling 2 confines a shape's reach to one home, anyone who may author that home can already
`can_modify_resource` every resource homed there, and therefore already reaches every artifact whose
verdict the shape would record. **Declaring a shape confers no reach the declarer lacked.**

The exact two-arm branch already exists in the current `can_modify_resource`
(`migrations/20260804000020_profile_reachable_teams_write_gates.sql:102-108`):

```sql
-- container-write cascade: whoever may author the home container may modify its nodes.
SELECT 1 FROM kb_resource_homes h
 WHERE h.resource_id = p_resource
   AND CASE h.anchor_table
         WHEN 'kb_cogmaps'  THEN cogmap_authorable_by_profile(p_profile, h.anchor_id)
         WHEN 'kb_contexts' THEN context_authorable_by_profile(p_profile, h.anchor_id)
         ELSE false
       END
```

Note that it **calls** the predicates rather than inlining them, so it tracks the narrowing applied
in `20260712000010_context_read_predicates.sql:171-198` (direct membership in the owning team with
an authoring role; `watcher` is read-only; mutation does not inherit up the enclosure chain).
The registry must do the same — call, never restate.

**A dead end, recorded so it is not re-attempted.** A namespace-scoped grant has no seam:
`kb_access_grants.subject_table` is `CHECK (subject_table IN ('kb_resources','kb_contexts','kb_cogmaps'))`
(`migrations/20260630000001_access_grants_seam.sql:26`) — a team or profile is not a grantable
subject. Delegating "may declare shapes in team T's namespace" would have required widening that
CHECK. Ruling 2 removes the need entirely.

---

## 6. Ruling 4 — descriptive by default, coercive only by opt-in

**AMEND.** This changes the register's Refusal face; see §9.

The registry exists to let agents and humans **declare the shapes of the data they are storing**, so
those shapes are discoverable and informative. It does not assume a `--strict` flag on everything.

**Strict conformance is an opt-in property of a shape.** The system stays flexibly usable and
reaches consistency eventually, rather than forcing a writer to be right up front and then churn
through revising both the JSON Schema and already-persisted artifact bodies to clear reconciliation
failures. These are data artifacts, not rows in a NoSQL store.

**Enforcement is a second closed vocabulary**, alongside the three selection intents:

| Term | Meaning |
|---|---|
| `advisory` | **Default.** A non-conforming commit succeeds and is recorded as non-conforming. |
| `enforcing` | A non-conforming commit is refused, and the refusal carries what failed. |

Because it is closed, the goal clause `a-declined-act-teaches-its-vocabulary` applies to it: an
unrecognized enforcement term is refused with the vocabulary, exactly as
`data_artifact_commit` already does for intent (`20260820000020_data_artifacts.sql`).

---

## 7. The verdict model

### 7.1 Commit-time is synchronous, and the register requires it

**CONFORM.** From the register's three-way Then:

> **Synchronously**, on commit: the datum is retrievable whole by its writer, and the writer knows
> whether it satisfied any shape in force for its family.

So the commit path resolves the shape in force for `(home, kind_owner, artifact_kind)`, validates,
and records the verdict in the same act. Under `enforcing`, a failure refuses; under `advisory`, it
records.

### 7.2 Validation is Rust-side, necessarily

**CONFORM.** There is no in-database JSON Schema validator — no `pg_jsonschema` — and the incumbent
registry does not validate either. Twice in migrations:

> "`_event_append` does NOT validate payloads against this schema"
> — `20260712000040_region_anchor_functions.sql:116`, and again at
> `20260712000050_workflow_default_lens.sql:140`

`kb_event_types.payload_schema` is a *published contract* enforced by typed Rust structs plus a
committed-snapshot test (`crates/temper-substrate/tests/payload_schema.rs`), not by the database.
The artifact registry must go further than that — the register demands a real verdict — so
validation runs in Rust. The crate is already in the workspace: `jsonschema = "0.45"`
(`crates/temper-workflow/Cargo.toml:10`).

### 7.3 Pre-existing artifacts reconcile asynchronously, on the incumbent queue

**CONFORM.** Declaring a shape leaves the family's existing artifacts in `DeclaredNotYetChecked` and
enqueues a job; a worker verdicts them. This is what the register already describes — its Closure
section enumerates "declared and not yet checked" as a shape-state, and its Then reads:

> **Eventually, once everything downstream has settled**: … every stored datum carries a verdict
> consistent with every shape currently in force for its family.

**No new queue infrastructure.** `kb_workflow_jobs` gained an anchor scope in
`migrations/20260802000020_workflow_jobs_anchor_scope.sql`:

- `context_id uuid REFERENCES kb_contexts(id) ON DELETE CASCADE` (line 33), with
  `CHECK (num_nonnulls(cogmap_id, resource_id, context_id) = 1)` (line 44)
- `uq_workflow_jobs_in_flight_context` (line 53) — a partial-unique index whose stated purpose is
  that "N settling-worthy arrivals on one context collapse to ONE job"
- `workflow_job_enqueue_anchor(p_cogmap, p_context, p_persona, p_dispatch_type, p_payload)` (line 68),
  idempotent, returning NULL when a job is already in flight
- `workflow_job_claim_anchor` — the claim twin, `FOR UPDATE SKIP LOCKED`, returning both scope
  columns so the worker can rebuild the `HomeAnchor`

The anchor pair is `(cogmap, context)` with exactly one non-null, which matches ruling 1's home
exactly. **Reconciliation is enqueued on declare, on amend, and on `resource_rehome`.**

### 7.4 A verdict is a disposable read-model, not an event

**EXTEND.** One hard constraint decides where it cannot go; the rest is a ruled judgment.

`kb_data_artifacts` **cannot** hold the verdict. Its own header states the invariant
(`20260820000020_data_artifacts.sql:7-9`):

> "Every column here is derivable from the event payload, so the table byte-diffs under replay; the
> bytes ride a sidecar and are proved by `content_hash`."

A verdict depends on a shape declared *after* the commit, so it is not payload-derivable. A verdict
column would break the replay byte-diff that `artifacts_replay_byte_identically` currently
witnesses.

Given that, the ruling is a **separate table, not event-sourced** — rebuildable at any time from
`(artifact content, shape in force)`, because that is exactly what a verdict is: a derived fact.
Event-sourcing it would generate large event volume for something reconstructible, and the
staleness guard in §7.5 already means a verdict is never trusted on its own authority.

The *governance* record of the declaration itself is still an event (§3). It is only the
per-artifact verdict that is disposable.

No incumbent was found for this shape in the substrate — the comparable tables here are
event-backed — so this is a new posture rather than a pattern being followed, and §12 records it as
such.

### 7.5 Staleness must be unrepresentable as conformance

**CONFORM** — this is what `unchecked-never-reads-as-checked` demands.

A stored verdict is trusted **only** while its `(shape_id, shape_version, content_hash)` triple
matches the currently governing shape and the artifact's current content. Anything else reads as
`DeclaredNotYetChecked`.

This makes the negative clause hold **by construction rather than by a worker running on time** —
which matters most precisely when the worker is behind. It also makes `resource_rehome` correct for
free: after a rehome the governing `shape_id` differs, so the stored verdict stops matching and the
artifact reads as unchecked until reconciled.

### 7.6 Shape-state variant names are already reserved — conform to them

**CONFORM.** `crates/temper-substrate/src/payloads.rs:652-657` reserves the exact names:

```rust
// Future variants, when the shape registry lands:
//   DeclaredSatisfied — a shape is in force and the artifact conforms.
//   DeclaredNotSatisfied — a shape is in force and the artifact does not conform.
//   DeclaredNotYetChecked — a shape was registered but the validation sweep has not reached
//     this artifact.
```

Use these, with the `snake_case` serde renames as the SQL literals
(`declared_satisfied`, `declared_not_satisfied`, `declared_not_yet_checked`), matching the existing
`never_declared`. `parse_shape_state` (`crates/temper-substrate/src/readback/mod.rs:2117`) gains the
three arms; its `bail!` default is retained so an unknown literal stays a decode error, never a
silent "looks fine."

---

## 8. Amendment: a shape revises by assert/fold

**CONFORM.** The predecessor spec gives the registry "a schema version"
(`2026-08-20-resource-owned-data-artifacts-design.md:141-143`), but every revisable thing in this
substrate revises by assert/fold rather than by mutating a row —
`kb_properties`, `kb_edges`, and `kb_data_artifacts` itself, whose header records the reasoning
(`20260820000020_data_artifacts.sql:28-30`):

> "Assert/fold, as `kb_properties` and `kb_edges`. No mutable `revised` column: revision IS the
> folded chain, and a mutable timestamp would be the one non-payload-derivable column."

So amending a shape folds the prior row and inserts a new one; the version is the chain depth, not a
mutable counter. This keeps the lineage the rest of the substrate keeps, and it makes the
`shape_version` half of the staleness triple (§7.5) meaningful — a verdict recorded against a folded
shape version stops matching, and the artifact correctly reads as unchecked.

---

## 9. Register amendments this design requires

The goal register records intent; refining the design is expected to move it. These are the moves.

**Refusal face — qualify the conformance refusal.**

> ~~Data that does not satisfy a shape already in force for its family. The decline carries what
> failed.~~
>
> → Data that does not satisfy a shape in force **and declared `enforcing`** for its family. The
> decline carries what failed. Against an `advisory` shape, non-conformance is recorded, never
> declined.

**Remainder — retire the tenancy item.**

> ~~Registry tenancy and registration authority — unexamined, and blocking for the shape registry.~~
>
> → Namespace decided 2026-08-20 and shipped in the `kind_owner_table` CHECK. Home ruled polymorphic
> over `(kb_contexts, kb_cogmaps)`; the shape in force keyed per home; authority ruled to the
> incumbent `context_authorable_by_profile` / `cogmap_authorable_by_profile` pair. This item retires.

**Closure — add an axis.** Enforcement mode is a closed vocabulary the register closes over:
`advisory` · `enforcing`.

**Equivalence claim 3 — record the second attack surface.** "A datum's shape-state is a property of
the datum, not of the reader" was already attackable under shape versioning. Under a per-home key it
is additionally a property of *the datum and its resource's current home*: `resource_rehome` changes
shape-state without touching the artifact.

---

## 10. Findability

**Findable means enumerable through the registry, never found by resemblance.** The predecessor
spec is explicit that "a registered schema never enters the chunk/embed pipeline and never appears
in search", and `structured-data-is-never-found-by-resemblance` holds the same line for artifacts.

What must work is a direct enumeration surface:

```
temper data-artifact schema list --context <ref>     # what shapes are declared here
temper data-artifact schema show <ref>               # one shape, its version, its enforcement mode
temper data-artifact schema declare <ref> --kind <k> # gated on context_authorable_by_profile
```

**Nested under the existing `data-artifact` group**, not a new top-level command: every command noun
in the CLI is singular (`cogmap`, `context`, `invocation`, `data-artifact`), and two-level nesting
already exists (`admin_connection.rs`, `admin_machine.rs` → `temper admin connection`). This keeps
the artifact family in one group and avoids introducing the CLI's only plural command.

The surface must reach parity across CLI, API and MCP in the same way the commit/retrieve surfaces
did — an auth change or a new read touches `temper-api` **and** `temper-mcp`, never one alone.

---

## 11. Out of scope

### Rejected

- **A schema as a resource** (`doc_type: schema`). Rejected in the predecessor design and still
  rejected: `body_storage` is *derived* from block coverage (`_recompute_body_storage`), never
  asserted, so a schema-as-resource cannot opt out of being blocked, chunked, embedded and searched
  — the exact failure this whole feature exists to end.
- **A global or team-scoped shape registry.** Ruled out by ruling 2; a shape you cannot read must
  not verdict your data.
- **Widening `kb_access_grants.subject_table` to admit teams or profiles.** Ruling 2 removes the
  need; widening it for this would be a large RBAC change bought for nothing.
- **Verdict columns on `kb_data_artifacts`.** Breaks the replay byte-diff invariant (§7.4).
- **In-database JSON Schema validation.** No validator is available, and the incumbent registry
  deliberately does not validate (§7.2).

### Deferred

- **Shape versioning semantics for readers.** A verdict is recorded against a specific
  `shape_version`. Whether a reader can ask "does this conform to version N" rather than "to the
  version in force" is a real question and is not answered here.
- **Cross-home shape reuse.** Declaring the same JSON Schema in five contexts currently means five
  rows. Whether a shape can be *referenced* from another home, rather than copied, is deferred —
  it reintroduces exactly the cross-visibility question ruling 2 closed, and needs its own ruling.
- **Bulk/administrative reconciliation controls.** Forcing a re-verdict sweep, or reporting sweep
  lag, is operator surface that can follow the mechanism.
- **The rate-shaped axes the register already names open.** Artifact size, commit rate, count per
  resource, registry cardinality. Still unmeasured; still open. Async reconciliation makes a large
  backlog *tolerable* rather than fatal, which lowers the urgency without answering the question.

---

## 12. Confidence

Stated so a later reader can weigh it rather than infer it from the document's tone.

- **Rulings 1 and 3 are forced.** The event-anchor CHECK and the container-write cascade are on
  disk and quoted above; there is little room for these to be wrong.
- **Ruling 2 is a genuine judgment** with a clear stated rationale and an acknowledged cost (team
  convergence becomes practice, not enforcement).
- **Ruling 4 is a product posture**, deliberately chosen.
- **The two forks in §7.4 and §8** — disposable read-model, and assert/fold — were decided because
  the rationale holds and fits how the rest of this feature set works, **not** because a serious
  counter-argument was constructed and defeated. If either turns out wrong, this is the paragraph
  that says where to look.
- **§8 (assert/fold) has an incumbent; §7.4 (disposable read-model) does not.** Assert/fold is the
  substrate's existing revision pattern and is cited. The disposable read-model is a *new* posture
  here: a candidate precedent was checked and rejected — `kb_principal_standing` is written only
  through transition functions that emit a ledger event
  (`migrations/20260720000010_principal_standing.sql:35-38`), so it is event-backed, not
  disposable. §7.4 therefore rests on the replay-invariant constraint plus judgment, and is the
  weaker of the two.

---

## 13. What is not yet true

Nothing in this document is built. There is no registry table, no `shape_declared` event, no
verdict read-model, no reconciler, and no CLI subgroup. Every artifact in production reports
`never_declared` from a hardcoded SQL literal
(`migrations/20260820000030_data_artifact_reads.sql:79,154`), and will continue to until this ships.

The two goal clauses this unblocks — `unchecked-never-reads-as-checked` (partial) and
`declaring-a-shape-never-destroys-what-came-before` (declared-uncovered) — stay exactly as the
register describes them until witnesses are authored inside the build, not decomposed from this
spec in advance.
