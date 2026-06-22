# WS6 Migration Endgame — namespace collapse, split deletion, bootstrap export

Design spec for the step beyond the flip: collapsing the `temper_next` / `public`
two-schema split back to a single canonical schema, deleting the migration machinery,
and (as a *separate, enabled* follow-on) exporting a clean OSS bootstrap migration set.

Sibling specs (both *to be written*): **shim-exit** (native surface shape / retire
`reconstruct_resource_row`, backlog task `019ee5a4-710a`) and **bootstrap-export** (OSS
migration set — this spec *enables* it but does not contain it). Supersedes the framing in the
`migration-endgame` backlog task (the assumptions below correct it).

---

## Problem — and a correction to the original assumption

The flip plan stopped at "set `kb_backend_selection=next` → rename `public.*` aside." Three
things were left unspecified: schema promotion/collapse, the legacy drop, and the sequencing
that keeps surfaces functional throughout.

**The 2026-06-22 post-flip investigation corrected a load-bearing assumption.** The original
endgame framing treats `public` as the live/authoritative schema and `temper_next` as a
candidate to be promoted. Reality:

- **`temper_next` is already live.** Prod has served reads+writes from `temper_next` since the
  cutover (flag flipped 2026-06-16; data wholesale-(re)synthesized 2026-06-21 13:29). New
  writes land in `temper_next`.
- **`public` is stale.** Frozen at 2026-06-21 13:23; it is missing every write since go-live.

So the endgame is **not** "promote the candidate." It is "**ratify the already-live
`temper_next` as canonical and retire the stale `public`.**" Two consequences:

1. **The rollback boundary inverts.** `public` is no longer a safe rollback target — falling
   back to it loses all post-go-live writes. The real safety net is a snapshot/branch of the
   **live (`temper_next`) state**, not `public`.
2. **The split never fully hydrated.** A class of read paths still resolves against `public`
   on the raw connection pool (confirmed: `list --meta-only`, `show --edges`; code-confirmed:
   graph aggregator, events feed — see findings task / [[project_ws6_flip_already_executed]]).
   These serve **stale** data today. This is not a bug to fix in the split — it is the
   strongest argument *for* collapsing the split, which makes every read correct by
   construction. It also exposes a validation gap (below).

---

## End state (target)

1. **One canonical schema** containing every intended table + function, with the cognitive-map
   substrate native to it. No `temper_next` namespace; no `kb_backend_selection` flag.
2. **The split machinery is deleted** (see *Deletion scope*). Every surface reads/writes the
   one schema directly — nothing routes through a flag, a `read_selector`, or a readback shim.
3. **The cognitive-map substrate is preserved and re-homed** — `temper-next`'s product logic
   (affinity, cluster, drift, cogmap regions, replay, event-sourcing) survives; only the
   *namespace it targets* and the *migration/readback scaffolding* go away.
4. This **enables** (does not include) a clean OSS bootstrap migration set — a separate spec.

> **Naming note.** "Canonical schema" deliberately avoids prejudging `public` vs a renamed
> `temper_next`. The mechanics section weighs rename-vs-search-path; the *name* is an
> implementation choice, the *collapse to one* is the requirement.

---

## What is scaffolding (delete) vs substrate (keep) — the disentanglement

`crates/temper-next` is two things wearing one crate. The endgame must separate them; this is
the spec's central analysis task and the input to crate extraction (shim-exit spec).

| Component (temper-next/src) | Role | Fate |
|---|---|---|
| `synthesis/` | builds `temper_next` *from* `public` frontmatter | **scaffolding** — retire after collapse (one-shot migration) |
| `readback/` | reconstructs prod-shape rows from `temper_next` | **scaffolding** — retire via shim-exit spec |
| `affinity`, `cluster`, `drift` | cogmap region/affinity computation | **substrate** — keep, re-home onto canonical schema |
| `scenario`, `replay`, `events` | event-sourcing + declarative runbooks | **substrate** — keep (verify each) |
| `write`/`writes`, `payloads`, `ids`, `fingerprint`, `content`, `embed` | mutation + content plumbing | **mixed** — audit per-item |

In temper-api: `backend/read_selector.rs` (429L), `backend/next_backend.rs` (545L),
`backend/selection.rs` (163L), `services/backend_selection_service.rs`, the
`select_backend`/flag dispatch in handlers (`resources`, `meta`, `edges`, `search`, `ingest`)
— **all scaffolding, all deleted.** `db_backend.rs` collapses to *the* backend.

---

## Mechanics — promotion / collapse

Two candidate mechanisms; the spec recommends evaluating both against the live state, but
leans **rename**:

- **Rename** `temper_next.*` → the canonical name; drop the stale old `public.*`. Pro: the live
  data does not move; atomic-ish via `ALTER SCHEMA ... RENAME`. Con: name churn; the old
  `public` must be dropped (after a snapshot).
- **Search-path / in-place** — keep the schema, repoint everything. Rejected as primary: leaves
  a `temper_next`-named canonical schema permanently, contradicting "collapse."

**sqlx implications (must be in the executable plan):**
- The temper-next per-crate `.sqlx` cache targets the `temper_next` namespace; the workspace
  caches target `public`. After collapse, *all* macros resolve against one schema — regenerate
  every cache (`prepare-*` tasks) and re-unify the search-path assumptions baked into CI
  (`SQLX_OFFLINE=true`).
- The artifact-schema (`schema-artifact/01_schema.sql` + `02_functions.sql`) and the sqlx
  `migrations/` must reconcile to one source of truth (today they are two: the artifact builds
  `temper_next`, migrations build `public`). This reconciliation IS the bootstrap-export spec's
  seam — call it out, don't solve it here.

---

## Validation — close the gap this week exposed

The §9 read-floor harness validated **data parity** (does `temper_next` *contain* the same data
as `public`?) but **not surface coverage** (does every read/write *endpoint* actually resolve to
the live schema?). The read-path defect lived exactly in that gap: synthesis was perfect, wiring
was not, no test exercised "a schema-only-resident resource through each HTTP surface."

**Requirement:** the endgame ships with an **end-to-end surface-parity test** — create a resource,
update content + properties, assert an edge, then assert *every* read surface (list, list
`--meta-only`, show, show `--meta-only`, show `--edges`, search, graph, events) returns it. Post
collapse there is one schema, so this is simply "every surface sees the one truth" — no flag
matrix. This test is the durable artifact; it would have caught today's bug and guards the
collapse.

---

## Sequencing (surfaces stay functional throughout)

1. **Snapshot the live (`temper_next`) state** — the rollback target (NOT `public`).
2. **Disentangle** substrate from scaffolding (the table above), landing substrate so it can run
   against the canonical schema. (Largest analysis chunk; overlaps shim-exit.)
3. **Collapse** — rename/promote to one schema in a window where no surface resolves to a
   moved/dropped namespace. Redeploy (flag read once at startup; see runbook).
4. **Delete the split machinery** + the flag + scaffolding; regenerate sqlx caches.
5. **Surface-parity test green** → drop the stale old schema (after the snapshot window).
6. **Then**, separately: bootstrap-export spec (OSS migrations) + shim-exit (native shape).

Order vs siblings: **endgame collapse → shim-exit (native shape) → bootstrap-export → crate
extraction (last)**. Shim-exit can begin once the schema is singular; bootstrap-export needs the
reconciled single source of truth from step 4.

---

## Rollback boundary

- **Before the stale-schema drop (step 5):** roll back by repointing to the snapshot of the live
  `temper_next` state. `public` is NOT a rollback target (stale).
- **After the drop:** rollback = restore from snapshot only. This is the point of no return;
  gate it on the surface-parity test being green and a held snapshot.

---

## Risks

- **URL/tool mismatch catastrophe.** This week a "trial" `flip-load-next` ran against prod main
  by mistake (harmless only by luck). Endgame steps that `DROP`/`RENAME` schemas on the live DB
  carry the same blast radius — every destructive step gates on (a) a held snapshot and (b) an
  explicit confirmation of the target connection.
- **Incomplete hydration, unknown unknowns.** The split leaked to ≥4 read paths with no test
  catching it; assume other scaffolding assumptions are similarly unaudited. The surface-parity
  test is the mitigation — it does not require a perfect audit, it dissolves the question.
- **Substrate/scaffolding misclassification.** Deleting a "scaffolding" file that the cogmap
  product actually needs. Mitigation: the disentanglement table is reviewed before any deletion;
  deletions land behind the green surface-parity + cogmap tests.

---

## Out of scope (separate specs)

- **Bootstrap-export / OSS-vs-instance separation** (points 2–3 of the goal): clean foundational
  migrations (structure + functions + system seed) scrubbed of temperkb.io instance data; the
  three-artifact model (bootstrap / instance data / archived evolution). *Enabled by* this spec;
  its own spec + task.
- **Shim-exit / native surface shape** — sibling spec.
- **Crate extraction** (`temper-substrate` / `temper-workflow`) — last, after shape is native.

---

## References

- **Canonical-layer draft** (graft/reconcile/carry-over, verified read-only vs the live substrate):
  `2026-06-22-ws6-canonical-layer-draft.sql` — feeds collapse step 3. Reconciles `kb_profiles`,
  grafts the 7 substrate-absent infra tables + enums, carries the identity data via INSERT…SELECT.
- Schema diff + Flag resolutions (identity union; `kb_scopes` superseded): `2026-06-22-ws6-endgame-schema-diff.md`
- [[project_ws6_flip_already_executed]] (live-on-`temper_next` timeline; read-path defect)
- Findings task `019eefbe` (raw-pool reads serve stale `public`)
- `crates/temper-api/src/backend/{read_selector,next_backend,selection,db_backend}.rs`
- Flip runbook: `docs/guides/ws6-flip-runbook.md`
- Convergence/§9 floor: `docs/superpowers/specs/2026-06-12-ws6-convergence-delta-adjudication-design.md`
