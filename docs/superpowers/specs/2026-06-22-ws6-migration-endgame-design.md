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
   These serve **stale** data today. Collapsing the split makes the *flag-aware* reads correct
   by construction — but **not** the raw-pool leak paths. The completeness review (below) found
   `graph_service`/`event_service` query the *legacy table shape* (`kb_resource_edges`; the old
   direct-FK `kb_events` columns), which the canonical substrate does not have — so they need
   genuine **rewrites onto the substrate shape**, not search_path inheritance. "Correct by
   construction" holds only after that rewrite ships (it is in the §Coincident code changes
   manifest). This also exposes a validation gap (below).

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

> **✅ RESOLVED (2026-06-22) — full per-symbol audit: `2026-06-22-ws6-disentanglement-audit.md`.**
> The audit (six-way parallel read, every verdict caller-cited, load-bearing claims re-grepped)
> corrects this table's grain. **The "mixed" band is not mixed — `write`/`writes`/`payloads`/`ids`/
> `fingerprint`/`content`/`embed` are ALL substrate (KEEP).** The real disentanglement is at the
> **symbol** grain *inside* the scaffolding: `synthesis/` is confirmed scaffolding but harbours
> **permanent live-path code** that must be re-homed *before* deletion — a blanket "delete
> `synthesis/`" would delete the live write path's property-classifier (`key_fate`, used at
> `next_backend.rs:27,91`) and break compilation (`slugify`, used at `writes.rs:64`). Three buckets,
> not two: KEEP / DELETE / **RE-HOME**.

| Component (temper-next/src) | Role | Fate (audited) |
|---|---|---|
| `synthesis/{source,mod,bootstrap,parity}` | builds `temper_next` *from* `public` frontmatter | **scaffolding — DELETE** at collapse, *after* carrying survivors (below) |
| `synthesis/key_fate` | §7 key→property-tier classification (no SQL) | **RE-HOME (permanent)** — on the live **write** path; mis-filed under `synthesis/`. The standout finding |
| `readback/` | reconstructs prod-shape rows from `temper_next` | **scaffolding — DELETE-after-shim-exit** (later than collapse; with the legacy shape) |
| `affinity`, `cluster`, `fingerprint`, `ids` | cogmap math + pure helpers/types (zero schema coupling) | **substrate — KEEP**; extraction-ready (lift wholesale to `temper-cogmap`) |
| `drift`, `substrate`, `write`, `embed` | cogmap formation/materialize + DB access | **substrate — KEEP**; `substrate.rs:20` carries the load-bearing search_path rewrite |
| `events`, `replay`, `writes`, `payloads`, `content` | forward event-sourcing + live write composition + content plumbing | **substrate — KEEP**; `replay` is forward-ledger (not legacy-trail); `writes` carries the bulk of the de-qualify work |
| `scenario/**` | declarative seeds + scenario runbooks + access proofs | **substrate — KEEP** (surviving product feature, *not* on the live request path); 2 carve-outs |

**Survivors to carry out before deleting `synthesis/`** (full table + re-home targets in the audit):
`key_fate.rs` (whole module → canonical write layer), `bootstrap::slugify` (→ a KEEP util shared by
`writes`+`scenario`), `parity::{reconstruct_body,new_substrate_chunks,ReadChunk}` (→ move next to
`readback/`, retire together at shim-exit). `bootseed::system_event_type_names()` goes dead with
synthesis (sole caller is `bootstrap.rs:105`). **`content.rs` must NOT be deleted with synthesis**
despite synthesis importing its types — it is live-write substrate.

In temper-api: `backend/read_selector.rs` (429L), `backend/next_backend.rs` (545L),
`backend/selection.rs` (163L), `services/backend_selection_service.rs`, the
`select_backend`/flag dispatch in handlers (`resources`, `meta`, `edges`, `search`, `ingest`)
— **all scaffolding, all deleted.** `db_backend.rs` collapses to *the* backend.

---

## Mechanics — promotion / collapse

**Decision (committed): rename `temper_next` → `public`.** Grounded in the live connection
mechanics, not preference:

- The app's bare pool connects with **no search_path set** (`temper-api/src/main.rs:23`), so the
  connection default is `public`. The legacy read path uses unqualified SQL against that default;
  the Next path opts into the substrate per-operation via `SET LOCAL search_path TO temper_next,
  public` (`next_backend.rs:172`, `read_selector.rs:234`, `writes.rs:83`) — **plus** `substrate`
  builds its *own* pool that sets the path pool-wide via `.after_connect(… SET search_path =
  temper_next, public)` (`substrate.rs:18-22`, **not** a per-op `SET LOCAL`). Both styles collapse
  away once `public` is the default, but the edits differ: delete the per-op lines vs. delete the
  `after_connect` builder.
- Renaming `temper_next` → `public` therefore makes the canonical schema the **connection default**
  — every per-op `SET LOCAL search_path` line and every `temper_next.`-qualified query *simplifies
  away to plain unqualified SQL on the default schema* (the §C collapse-rewrite inventory in the
  disentanglement audit becomes "delete these search_path hooks / de-qualify these refs"). The live
  data does not move; the rename is atomic via `ALTER SCHEMA ... RENAME`.
- **Search-path / in-place** (keep the `temper_next` name, repoint the default) is **rejected**: it
  leaves a `temper_next`-named canonical schema permanently (contradicts "collapse"), AND it would
  require setting a non-default search_path on *every* connection forever (the bare-pool default
  would point at the dropped/empty `public`) — strictly more coupling than the rename removes.

### Executable collapse sequence (the DDL run order)

Builds directly on the validated canonical-layer draft (`2026-06-22-ws6-canonical-layer-draft.sql`,
which grafts infra + carries identity *into* `temper_next` from the renamed-aside legacy) and reuses
the flip runbook's freeze/snapshot/redeploy discipline. Runs in an operator-controlled window
(single-user; arc-1 accepts brief downtime). **Prod is already on `next`**, so reads+writes serve
from `temper_next` throughout until the rename.

> **⚠️ Two infrastructure blockers (completeness review, 2026-06-22) — resolve BEFORE this is a real
> migration.** `public` is not just the stale schema; it is also the **home of the `vector` extension,
> the `pg_uuidv7` extension + `public.uuid_generate_v7()` generator, and the `_sqlx_migrations`
> ledger** — none owned by `temper_next`, which only *reaches* them via search_path (`schema-artifact/
> 01_schema.sql:10-12`, `migrations/20260330000001_consolidated_schema.sql:9`,
> `migrations/20260420000012_uuidv7_portability.sql:28`). A naïve `public → public_legacy` rename
> strands all of them. The sequence below now handles them explicitly (steps 3 + 5b); the flip runbook
> *already learned* this (it pre-installs the uuid shim into the target's `public`) — do not regress.

1. **Freeze + snapshot.** Operator stops reading/writing; `neonctl branches create` from `main` →
   the rollback target (record branch id + LSN). This is the real safety net — `public` is stale,
   NOT a rollback target.
2. **Rename the stale schema aside:** `ALTER SCHEMA public RENAME TO public_legacy;`. Safe under the
   freeze — the only live-path reference to `public.*` is `writes.rs:30`'s prod→next profile bridge,
   which the write-freeze guarantees is unexercised.
3. **Relocate the shared infrastructure into the surviving schema** (the BLOCKER-1 fix): make `vector`
   and a `uuid_generate_v7()` generator resident in `temper_next` (→ the new `public`) so the canonical
   schema *owns* them and the legacy drop can't cascade into them. **Committed mechanism (asymmetric —
   the two have different relocatability):**
   - **`vector` — relocate the extension.** `ALTER EXTENSION vector SET SCHEMA temper_next;` (pgvector
     ships `relocatable = true`). After step 5's rename it ends up resident in the canonical `public`.
   - **`uuid_generate_v7()` — re-create as a plain SQL function; do NOT relocate `pg_uuidv7`.** Re-create
     the generator *in* `temper_next` mirroring `tools/flip/uuid_portable.sql` (`CREATE OR REPLACE
     FUNCTION temper_next.uuid_generate_v7() …`). The canonical schema then owns a **self-contained**
     generator with no extension dependency, so the legacy `pg_uuidv7` extension drops harmlessly with
     `public_legacy` (step 8). Substrate DDL calls `uuid_generate_v7()` *unqualified* (resolved via
     search_path today); post-rename it resolves to this function in the canonical `public`. Chosen over
     `ALTER EXTENSION pg_uuidv7 SET SCHEMA` precisely because the flip already learned `pg_uuidv7` may
     not be restorable/relocatable on bare PG17 — re-creating the function sidesteps the question.
   - **Live-validation gate (highest-risk DDL — run on a throwaway Neon PG17 branch BEFORE the step-1
     window, not as an inline judgment call):** confirm (a) `ALTER EXTENSION vector SET SCHEMA` succeeds
     and a `::vector` cast + an HNSW/IVF index still resolve afterward; (b) the re-created
     `uuid_generate_v7()` mints valid, unique, time-sortable v7 UUIDs.
4. **Apply the canonical-layer graft/reconcile/carry-over** (the validated draft, now executable):
   reconcile `kb_profiles` (re-add `email`/`preferences`), graft the 7 substrate-absent infra tables
   + enums, `INSERT…SELECT` the identity/auth/seed data from `public_legacy` into `temper_next`. All
   FKs target substrate-present ids (carry-over FK order verified); verified read-only-GREEN vs the
   live snapshot. (Steps 2–5a are one transaction — `ALTER SCHEMA RENAME` + the graft are DDL,
   transactional in PG.)
5. **Promote:** `ALTER SCHEMA temper_next RENAME TO public;`. The canonical schema is now `public` —
   the connection default, now owning its extensions + uuid generator (step 3).
   - **5b. Remove the boot-time `migrate!` call (the BLOCKER-2 fix) — committed decision.**
     `temper-api/src/main.rs:27` runs `sqlx::migrate!("../../migrations")` **unconditionally at startup**.
     The new `public` (ex-`temper_next`) was built by the *artifact* (`schema-artifact/01+02.sql`), not by
     `migrations/`, and has no matching `_sqlx_migrations` ledger → `migrate!` would replay all ~44 legacy
     migrations, collide with the substrate objects, and panic at boot (`main.rs:30` `.expect`).
     **Decision: delete the boot-time `migrate!` call** in the coincident redeploy (§Coincident code
     changes). Rationale: post-collapse `migrations/` *no longer governs the canonical schema* (the
     artifact does), so auto-running it is semantically wrong independent of the panic; and prod's schema
     already exists (the renamed `temper_next`), so nothing needs applying at boot. A *meaningful*
     boot-time migrate is restored by **bootstrap-export** once it reconciles the artifact + `migrations/`
     into one source of truth (with a fresh ledger). **Rejected alternative — carry `_sqlx_migrations`
     forward** (`INSERT…SELECT` the legacy ledger into the new `public`): it no-ops the panic, but (a)
     preserves a *fictional* ledger (the schema was not built by those 44 files) and (b) actively collides
     with bootstrap-export — when it rewrites `migrations/` into a clean set, the carried checksums
     mismatch and `migrate!` panics on checksum drift. Removal leaves bootstrap-export a clean slate.
     (This is a code change in the redeploy, not a DDL step — listed in §Coincident code changes; resolve
     coincident with step 6.)
6. **Redeploy the collapsed code coincident with the rename** (the flag is read once at startup —
   `main.rs:34` — so the running split-code process cannot survive the rename; its
   `temper_next.`-qualified SQL would 42P01). The deploy must ship the §"coincident code changes"
   manifest below. Verify the surface-parity gate (`surface_parity_next.rs`, now un-ignorable green)
   over the live schema.
7. **Unfreeze.** Reads+writes resume against the one `public` schema, no flag, no search_path hooks.
8. **Drop `public_legacy`** after the retention window. This is the point of no return — gate it on the
   held snapshot + the 2 Flag-2 content-hash spot-checks + the **dependency guard below returning clean**
   (step 3 must have moved `vector` out; the re-created `uuid_generate_v7()` must own no `pg_uuidv7`
   dependency):
   ```sql
   -- (a) vector must be resident in the canonical schema, NOT public_legacy:
   SELECT n.nspname AS vector_schema
   FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace
   WHERE e.extname = 'vector';            -- expect: 'public' (post-rename canonical)

   -- (b) no canonical (`public`) object may depend on any object in public_legacy:
   SELECT c.relname AS canonical_obj, rc.relname AS legacy_dep
   FROM pg_depend d
   JOIN pg_class c  ON c.oid  = d.objid    JOIN pg_namespace n  ON n.oid  = c.relnamespace
   JOIN pg_class rc ON rc.oid = d.refobjid JOIN pg_namespace rn ON rn.oid = rc.relnamespace
   WHERE n.nspname = 'public' AND rn.nspname = 'public_legacy';   -- expect: zero rows
   ```
   Only with (a) = `public` and (b) = zero rows: `DROP SCHEMA public_legacy CASCADE;` (CASCADE clears
   public_legacy's *internal* objects + the now-orphaned `pg_uuidv7` extension; the guard proves it
   reaches nothing canonical).

### Coincident code changes (must ship in the step-5 redeploy)

The schema rename and these edits are **one atomic release** — the running process references schema
names that the rename changes:

- **Delete the split machinery** (per §"What is scaffolding"): `backend/selection.rs`,
  `read_selector.rs`, `services/backend_selection_service.rs`, the `select_backend`/flag dispatch in
  handlers, the `kb_backend_selection` startup read (`main.rs:34`). `db_backend.rs` collapses to *the*
  backend; `NextBackend` becomes the backend.
- **Remove the boot-time `migrate!` call** (`main.rs:27-30`) — per step 5b, post-collapse `migrations/`
  no longer governs the canonical schema, so the unconditional boot migrate is **deleted** (not gated).
  Local/CI test-harness schema provisioning shifts from "boot applies `migrations/`" to "apply the
  artifact (`schema-artifact/01+02.sql`)"; bootstrap-export later restores a single reconciled
  migrate path + fresh ledger. Audit any other `migrate!`/`migrations/` provisioning site (test
  harness, `cargo make db-*`) in the same release so a fresh clone/CI still gets a schema.
- **Drop the search_path hooks** — `writes.rs:83`'s per-op `SET LOCAL search_path TO temper_next,
  public` *and* `substrate.rs:18-22`'s pool-wide `.after_connect` search_path (distinct mechanisms,
  both now redundant: `public` is the default), and **de-qualify** the `temper_next.`-prefixed SQL in
  `writes.rs` (`:36,:52,:66`) and `next_backend.rs`.
- **readback is de-qualified, NOT deleted at collapse.** Its 53 `temper_next.`-qualified refs +
  search_path lines must resolve to the renamed `public` (else reads 42P01), but the module survives
  until **shim-exit** retires the legacy prod-row shape (sibling spec). Sequencing handoff: collapse
  makes readback resolve to the one schema; shim-exit removes it. (Carry `parity::{reconstruct_body,
  new_substrate_chunks,ReadChunk}` with it — see audit survivor table.)
- **Re-home the synthesis survivors** so `synthesis/` deletes cleanly (audit §B): `key_fate.rs`
  (whole module → canonical write layer), `bootstrap::slugify`. `synthesis/` + the binary
  `Synthesize` subcommand are deleted in this release (the one-shot migration is spent).
- **Rewrite the raw-pool leak services onto the substrate shape** (completeness-review BLOCKER-3 —
  NOT fixed by the rename): `services/graph_service.rs` queries `kb_resource_edges` (DIES →
  `kb_edges`) and `services/event_service.rs` queries the old direct-FK `kb_events` columns
  (`profile_id`/`resource_id`/`kb_context_id`/`device_id`, moved to the entity/anchor/invocation
  model). Both run on the raw pool, so the rename makes them resolve to canonical and **break on
  missing tables/columns** — they need real rewrites, which the surface-parity gate's RED on
  `graph`/`events` already proves. This is the largest code work-item the first draft missed.
- **temper-mcp tool surface** dispatches through the same `AppState.backend_selection` /
  `select_backend` / `read_selector` being deleted (`temper-mcp/src/tools/{relationships,resources,
  search}.rs`). Deleting the field breaks temper-mcp compilation — update its tool handlers to call
  the single backend in the same release.
- **e2e split tests + the `next-backend` feature gate** (`tests/e2e/tests/{backend_read_path_next,
  backend_write_path_next,backend_selection_gate}.rs`, the `mcp_round_trip_test`, and the
  `next-backend` Cargo feature on `temper-api`/`tests/e2e`) test the split itself and stop compiling
  when `selection` is deleted — retire/rewrite them alongside; keep `surface_parity_next.rs` (the
  green acceptance gate).
- **temper-events `kb_scopes`/`porosity` retirement** — `temper-events/src/types/scope.rs` (`enum
  Porosity`, `#[sqlx(type_name="porosity")]`) + `ledger.rs:50`'s `FROM kb_scopes`. `kb_scopes` is
  dropped (schema-diff Flag-1 correction), so this scaffolding must retire in the same disentanglement
  (the audit scoped itself to `temper-next`; this is its temper-events sibling).

**sqlx implications (must be in the executable plan):**
- The temper-next per-crate `.sqlx` cache targets the `temper_next` namespace; the workspace
  caches target `public`. After collapse, *all* macros resolve against one schema — regenerate
  every cache (`prepare-*` tasks, including repointing `prepare-next`'s hardcoded
  `search_path=temper_next` in `Makefile.toml`) and re-unify the search-path assumptions baked into
  CI (`SQLX_OFFLINE=true`).
- The artifact-schema (`schema-artifact/01_schema.sql` + `02_functions.sql`) and the sqlx
  `migrations/` must reconcile to one source of truth (today they are two: the artifact builds
  `temper_next`, migrations build `public`). **The line is now drawn** (resolving the original punt):
  the *collapse* owns only the minimal fix — **delete the boot-time `migrate!`** (step 5b) so boot
  cannot panic — and the **bootstrap-export spec owns the full reconciliation** to one source of truth
  + a fresh ledger + restoring a meaningful boot migrate. The collapse does *not* attempt any ledger
  carry-over or migration rewrite; it simply stops auto-running a migration set that no longer governs
  the canonical schema.

---

## Validation — close the gap this week exposed

The §9 read-floor harness validated **data parity** (does `temper_next` *contain* the same data
as `public`?) but **not surface coverage** (does every read/write *endpoint* actually resolve to
the live schema?). The read-path defect lived exactly in that gap: synthesis was perfect, wiring
was not, no test exercised "a schema-only-resident resource through each HTTP surface."

**Requirement:** the endgame ships with an **end-to-end surface-parity test** — create a resource,
update content + properties, assert an edge, then assert *every* read surface (list, list
`--meta-only`, show, show `--meta-only`, show `--edges`, content, search, graph, events — **nine**)
returns it. Post collapse there is one schema, so this is simply "every surface sees the one truth"
— no flag matrix. This test is the durable artifact; it would have caught today's bug and guards the
collapse.

> **✅ Built (2026-06-22): `tests/e2e/tests/surface_parity_next.rs`** (gated `test-db,next-backend`,
> shipped `#[ignore]`d as the collapse acceptance gate). It creates a **schema-only-resident**
> resource — written via `NextBackend` so it lands ONLY in `temper_next` (+ a property + an edge via
> the live 4c `assert_relationship` path + a minted `resource_created` event), with schema-qualified
> negative controls proving it's absent from `public` — then drives all nine read surfaces over the
> real HTTP stack under `BackendSelection::Next`. The assertions encode the **desired post-collapse
> end-state** (every surface MUST resolve it); they are never inverted to "must NOT see", so the test
> documents the leak without codifying the bug. **Verified RED today on exactly the four leaking
> surfaces** (`list --meta-only`, `show --edges`, `graph subgraph`, `events`) and **GREEN on the five
> flag-aware ones** (`list`, `show`, `show --meta`, `content`, `search`) — no leak-map drift from the
> investigation above. When the collapse dissolves the split (one schema, every surface resolves to
> it) this test goes fully green; **un-ignoring it green is the collapse's acceptance criterion.**
> Run: `cargo nextest run -p temper-e2e --features test-db,next-backend --run-ignored all -E 'test(all_read_surfaces_resolve_next_only_resource)'` (needs `SQLX_OFFLINE=true`).

### Completeness review (2026-06-22) — what it changed

A three-way adversarial review (cross-artifact consistency · DDL-sequence soundness · classification
coverage) verified every load-bearing `file:line` claim true (flag-read-once, bare-pool default, the
two search_path hooks, `key_fate` live-path use, the 53-ref readback count, all 7 grafted-table +
3-enum DDLs vs the cited migrations — several exact to the digit) and confirmed the disentanglement +
table classification **complete**. It surfaced **three execution blockers the design altitude hid**,
now folded in above:

1. **Extension/uuid homing** — `vector` + `pg_uuidv7`/`uuid_generate_v7()` live in `public`; the naïve
   rename strands them and the legacy drop would cascade into the extension the live columns need.
   *(Two reviewers found this independently — high confidence.)* → sequence step 3 + step 8 gate.
2. **Boot-time `migrate!`** replays the legacy set against a ledger-less renamed schema and panics →
   the "deferred" migration reconciliation is a hard prerequisite. → step 5b.
3. **Raw-pool leak services** (`graph_service`/`event_service`) query the legacy table shape and need
   real rewrites, not search_path inheritance. → §Coincident code changes.

Plus coverage gaps (temper-mcp, e2e split tests + feature gate, temper-events `kb_scopes`/`porosity`)
— all now in the manifest. **Unverifiable from repo (flagged for human at execution):** the canonical
draft's "GREEN vs the live `flip-rollback-2026-06-22` snapshot" claim and the row-count facts (5
profiles / 5 auth_links / revision + event counts) reference a Neon snapshot unreachable from the
repo; the draft's *DDL* half was verified byte-faithful to its cited migrations, the *live-diff* half
must be re-confirmed against the snapshot when the collapse is staged.

---

## Sequencing (surfaces stay functional throughout)

This is the macro arc across the whole endgame; the **§"Executable collapse sequence"** above details
the DDL run order *within* step 3, and the **§"Coincident code changes"** the edits that ship with it.

1. **Snapshot the live (`temper_next`) state** — the rollback target (NOT `public`).
2. **Disentangle** substrate from scaffolding (the table above; **audit resolved** in
   `2026-06-22-ws6-disentanglement-audit.md`), landing substrate so it can run against the canonical
   schema. Concretely: re-home the survivors (`key_fate.rs`, `slugify`, the `parity` body-helpers)
   *before* deleting `synthesis/`; rewrite the two search_path hooks (`substrate.rs:20`,
   `writes.rs:83`) + the qualified `writes.rs`/`next_backend.rs` resolver SQL. (Overlaps shim-exit.)
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
- **Disentanglement audit** (per-symbol KEEP/DELETE/RE-HOME; survivor table; collapse-rewrite inventory): `2026-06-22-ws6-disentanglement-audit.md`
- [[project_ws6_flip_already_executed]] (live-on-`temper_next` timeline; read-path defect)
- Findings task `019eefbe` (raw-pool reads serve stale `public`)
- `crates/temper-api/src/backend/{read_selector,next_backend,selection,db_backend}.rs`
- Flip runbook: `docs/guides/ws6-flip-runbook.md`
- Convergence/§9 floor: `docs/superpowers/specs/2026-06-12-ws6-convergence-delta-adjudication-design.md`
