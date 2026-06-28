# L0 Telos Charter Delivery — Design

**Date:** 2026-06-28
**Status:** Design / spec
**Context:** Workstream 7 (Agent surface) under goal `substrate-kernel-to-cognitive-map`.
**Task:** `019f0368` — "Deliver L0 telos charter content (statement + questions + framing)".
**Companions:**
- Charter content design: [2026-06-25-l0-orientation-kernel-charter-design.md](2026-06-25-l0-orientation-kernel-charter-design.md) (§1 statement, §2 six questions-with-context, §3 framing — the content this delivers).
- Landmark delivery (landed, #177): [2026-06-25-l0-delivery-and-lifecycle-design.md](2026-06-25-l0-delivery-and-lifecycle-design.md) + plan [../plans/2026-06-25-l0-delivery-and-lifecycle.md](../plans/2026-06-25-l0-delivery-and-lifecycle.md). This design **extends** that reconciler.
- Architecture: [2026-06-25-cognitive-map-agent-invocation-architecture-design.md](2026-06-25-cognitive-map-agent-invocation-architecture-design.md) (L0 = deterministic kernel tier; L0 evolves via additive shipped content + operator-directed runs).

---

## Problem

Every cogmap is born via `cogmap_genesis` with a **telos resource** (`kb_cogmaps.telos_resource_id`)
holding its charter — a statement + questions-with-context + framing, stored as real role-tagged content
blocks. The L0 birth migration (`20260625000001_l0_kernel_cogmap.sql`) created L0's telos resource
(`00000000-0000-0000-0005-000000000002`) **empty** (`blocks: []`, title "What Temper Is") to dodge the
embeddings-in-a-migration wall (content chunks carry bge-768 embeddings; SQL can't run ONNX).

The landmark reconciler (#177) delivers L0's **22 landmark resources** to the live `system-default`
cogmap via `temper cogmap reconcile`, but it deliberately does **not** touch the telos: the telos is the
cogmap's identity (referenced by `kb_cogmaps.telos_resource_id`), not a `provenance: kernel` homed
landmark, so `kernel_slice` excludes it and the reconciler never sees it.

**Result: L0's purpose statement is blank in production.** The rich authored charter exists only in the
workbench seed `crates/temper-substrate/tests/fixtures/seeds/l0-kernel.yaml` under `cogmap.telos`
(statement + 6 questions-with-context + 5 framing lines).

## Why this matters beyond display

The telos is not cosmetic. `kb_cogmap_regions.telos_alignment = cosine(centroid, telos_resource.embedding)`
(canonical schema, the salience decomposition). The charter's bge-768 embeddings feed the region-salience
readout that the just-shipped analytics surface (`cogmap_region_metrics` / `cogmap_analytics`, #196/#197)
reads. An empty telos leaves `telos_alignment` degenerate. Delivering the charter **with real embeddings**
brings that salience signal — and semantic search over the charter prose — online.

---

## Grounding (verified against the tree, 2026-06-28)

- **Telos = role-tagged content blocks.** `_project_cogmap_seeded` (canonical_functions.sql:677) builds the
  telos via the shared `_project_blocks` path: block-0 statement, blocks 1..n questions-with-context, then
  framing; `doc_type='cogmap_charter'`. Each chunk carries `v_side->'embedding'` from the content sidecar.
- **Charter read = `resource_blocks(telos, …, p_role)`** (canonical_functions.sql:371) — a generic
  property-filtered block read; `statement`/`questions`/`framing` are all this one function with a
  `block_role` filter. `cogmap_telos(cogmap)` (canonical_functions.sql:396) resolves cogmap →
  telos_resource_id. So delivery must produce **role-tagged blocks** (`block_role` ∈
  `statement`/`question`/`framing`), each with embedded chunks.
- **Charter → (role, prose) translation already exists.** `TelosDef::block_specs` (substrate
  scenario/model.rs:120) maps `{statement, questions, framing}` → positional
  `[("statement", S), ("question", q + "\n\n" + ctx), …, ("framing", f), …]`; `content::prepare_blocks`
  (substrate content.rs) embeds them Rust-side. This is the exact path genesis uses; the CLI mirrors it
  client-side.
- **No post-birth "populate an empty telos" function exists.** `block_mutate` (canonical_functions.sql:957)
  is **revise-only** — it requires an existing block (and rejects an empty chunk set). Genesis left the
  telos with **zero** blocks, so nothing today can create the initial charter blocks. A new substrate
  function is required.
- **Embedding column is nullable; FTS is text-derived** (`search_vector tsvector` built from chunk content,
  not embeddings). So a text-only migration is *mechanically* possible but rejected — see Decision 1.
- **The reconciler is a single SERIALIZABLE tx inside an `admin_reconcile` `kb_invocations` envelope**
  (commit 0ce9172), admin-gated by `require_cogmap_write_admin`, diffing on the **server-recomputed**
  chunk-merkle (`body_hash_from_chunk_hashes`, commit e3609a0), **not** the advisory wire `content_hash`.
- **Live wire shape** (temper-core `types/reconcile.rs`): `ReconcileEntry` carries `chunks_packed` (the
  `compute_body_chunks` packed blob) as the SOLE body; `content_hash` is advisory. `ReconcileCogmapRequest
  { entries, fold_resources, fold_edges }`; `ReconcileOutcome { created, updated, folded, unchanged }`.

---

## Decisions

### Decision 1 — Deliver with **real embeddings** via the operator-CLI path (not a text-only migration)

The telos embedding feeds `telos_alignment` salience and enables semantic search over the charter. A
migration can't embed (no ONNX in SQL), and a text-only charter would leave `telos_alignment` degenerate
and the charter semantically unsearchable. The operator reconcile path already embeds client-side
(`compute_body_chunks`), so it is the delivery vehicle. (User-confirmed 2026-06-28.)

### Decision 2 — Extend the **existing landmark reconciler** with a `telos:` section (Approach 1)

The reconciler already *is* "converge L0 to its committed desired state." The charter is part of that
state, living in the same `system-default` cogmap the manifest already targets. A single operator command
(`temper cogmap reconcile`) delivers the **whole** L0 — charter + landmarks — atomically, reusing the
admin gate, SERIALIZABLE tx, `admin_reconcile` envelope, idempotent diff, and client-side embed with no
new surface.

**Considered and rejected — Approach 2 (separate `temper cogmap set-charter` command + endpoint):** cleaner
identity/content separation, but a whole second vertical (command + endpoint + backend command + client
method) and a second release step, fragmenting "what is L0" across two artifacts. The isolation it buys is
mostly conceptual. We accept a modest overload of the reconcile command because it is *self-consistent with
the command's purpose* (converge L0's desired state). (User-confirmed 2026-06-28.)

### Decision 3 — New substrate function `cogmap_charter_set` + `charter_set` event (one additive migration)

Neither existing primitive can populate an empty telos (`block_mutate` is revise-only; genesis is
once-guarded). The new function replaces the telos's full block set uniformly (handles both 0→N first
delivery and N→M re-delivery) via **fold-then-reproject**: fold all current non-folded telos blocks, then
`_project_blocks(telos, ev, blocks, sidecar)`, then `_recompute_resource_body_hash(telos)`, emitting one
`charter_set` event anchored on the cogmap. Block ids churn on each content change — acceptable because
**idempotency keys on content (`body_hash`), never on block identity**, exactly like the landmark diff.

**Considered and rejected — a generic `set_resource_blocks` primitive:** YAGNI. The telos is the only
caller; a charter-anchored function + `charter_set` event reads honestly in the ledger and can enforce the
cogmap-home anchor + telos resolution in one place. If a second "replace a resource's whole block set" use
appears, generalize then.

### Decision 4 — `charter` is a distinct outcome field, not folded into landmark counts

`ReconcileOutcome` gains a `charter: CharterDisposition` (`Unchanged | Created | Updated | Absent`). The
telos is a different grain from the `provenance: kernel` landmark slice; conflating it into
`created/updated/unchanged` would make the outcome (and `kb_invocations.outcome`) ambiguous. `Absent` is
the disposition when a request carries no `telos:` (landmark-only reconcile — backward compatible).

---

## Architecture

### Data flow

```
schema-artifact/manifests/l0-kernel.yaml          temper cogmap reconcile <l0-ref> --manifest <f>
  telos:                                     CLI:  parse telos → block_specs → [(role, prose)…]
    statement: …                                   compute_body_chunks(prose) per block  (client embed)
    questions: [{question, context}…]              → ReconcileTelos { blocks:[{role, chunks_packed}…] }
    framing: […]                                   ──PUT /api/cognitive-maps/{id}──►
  entries: [ …22 landmarks unchanged… ]
                                             API:  require_cogmap_write_admin → reconcile_cognitive_map
                                                     (existing SERIALIZABLE tx + admin_reconcile envelope)
                                                     ├─ landmark diff/apply        (UNCHANGED)
                                                     └─ telos branch:
                                                          cogmap_telos(id) → telos_resource_id
                                                          recompute charter body-merkle from chunks_packed
                                                          diff vs live telos body_hash
                                                          if changed → cogmap_charter_set(payload, sidecar, emitter)
                                                     → ReconcileOutcome { …landmark counts…, charter }
```

### Components

**1. Manifest (`schema-artifact/manifests/l0-kernel.yaml`)** — gains a top-level `telos:` section copied
verbatim from the workbench seed's `cogmap.telos` (statement + `questions:[{question, context}]` +
`framing:[…]`). The existing 22 `entries:` stay byte-identical. *What it does:* the committed desired state
for all of L0. *Depends on:* the manifest model (CLI parse).

**2. Wire types (`temper-core/src/types/reconcile.rs`)** — Rust-only CLI↔API, mirroring the existing
derive stack (no ts-rs/schemars; `web-api` utoipa derive; `PartialEq` for round-trip tests):
```rust
pub struct ReconcileTelos { pub blocks: Vec<ReconcileTelosBlock> }
pub struct ReconcileTelosBlock { pub role: String, pub chunks_packed: String }
// role ∈ {"statement","question","framing"}; chunks_packed = compute_body_chunks(block prose)

pub enum CharterDisposition { Unchanged, Created, Updated, Absent }   // serde-tagged like other small enums

// ReconcileCogmapRequest gains:  #[serde(default)] pub telos: Option<ReconcileTelos>,
// ReconcileOutcome gains:        pub charter: CharterDisposition,   (Default = Absent)
```
*What it does:* the typed contract for the telos extension. *Depends on:* nothing new.

**3. CLI charter-embed step (`temper-cli` reconcile action)** — after building landmark entries, if the
manifest has a `telos:`, translate `{statement, questions, framing}` → `[(role, prose)]` via the shared
`block_specs` logic, `compute_body_chunks(prose)` per block, and attach `ReconcileTelos`. *What it does:*
client-side embed of the charter. *Depends on:* `compute_body_chunks` (`feature = "embed"`), the
`block_specs` mapping. **The `{statement, questions, framing}` → `[(role, prose)]` mapping is lifted to a
shared location** (temper-core) so the CLI and the substrate `TelosDef::block_specs` cannot drift — the
prose-assembly rule (`question + "\n\n" + context`, empty-context → bare question) is defined once.

**4. Substrate function (new additive migration `migrations/20260629000001_cogmap_charter_set.sql`)** —
`cogmap_charter_set(p_payload, p_content, p_emitter)` + the `charter_set` event type (insert into
`kb_event_types` with its JSON schema). *What it does:* replace the telos's block set with the charter,
idempotently re-projectable. *Depends on:* `cogmap_telos`, `_project_blocks`, `_recompute_resource_body_hash`,
`_event_append`. **Payload shape** (identity-as-input, matching the genesis/`block_mutate` convention):
`{cogmap_id, blocks:[{block_id, seq, role, chunks:[{chunk_id, chunk_index, content_hash}]}]}`; **sidecar**
`{chunk_id → {content, embedding, header_path, heading_depth}}` (unpacked from `chunks_packed`).

**5. Backend telos branch (`temper-api` `reconcile_cognitive_map`)** — after the landmark diff, if
`request.telos` is `Some`: resolve `cogmap_telos(cogmap_id)`; recompute the charter's resource-level
body-merkle from the per-block `chunks_packed` (the same `body_hash_from_chunk_hashes` the substrate
stores); compare to the live telos `body_hash`; if equal → `charter = Unchanged` (currently-empty telos has
no body_hash → treated as changed → `Created`); else fire `cogmap_charter_set` and set
`charter = Created|Updated`. Runs inside the existing SERIALIZABLE tx + envelope. *What it does:* the
diff/apply decision for the telos. *Depends on:* components 2 & 4; reuses the landmark path's chunk-unpack
and body-merkle helpers.

### Idempotency

Same manifest + same live state ⇒ the recomputed charter body-merkle equals the live telos `body_hash` ⇒
**no `cogmap_charter_set` call ⇒ zero events** ⇒ `charter: Unchanged`. This is the landmark diff
philosophy applied to the telos at the resource-merkle grain (one resource, all its blocks). Block-id
churn on genuine change does not break idempotency because the diff keys on content, not identity.

### Error handling

- Reuses the endpoint's existing failure path: any error inside the SERIALIZABLE tx aborts the whole run
  (landmarks + telos) and closes the `admin_reconcile` envelope with `Disposition::Failed` — atomic.
- `cogmap_charter_set` raises if the cogmap has no telos (cannot happen for a genesis'd cogmap; fail-loud
  guards a malformed call) or if a chunk's sidecar entry is missing (inherited from `_project_blocks`).
- A request with no `telos:` is valid (landmark-only) → `charter: Absent`, no telos work — backward
  compatible with the shipped reconciler and its tests.

### Testing

Mirror the reconcile invariants (the `test-db` backend test + the e2e embed path):
1. **First delivery:** empty telos + a `telos:` request → `charter: Created`; the telos now has N
   role-tagged blocks readable via `resource_blocks(telos, principal, role)` for each of
   `statement`/`question`/`framing`.
2. **Idempotency:** re-run the identical request → `charter: Unchanged`, and **no new `kb_events`** rows
   fired between runs (snapshot the event count).
3. **Update:** change one framing line → `charter: Updated`; the other roles' read-shape intact; the live
   telos `body_hash` now matches the new charter merkle.
4. **Salience payoff (embed path):** after delivery + a materialize, `telos_alignment` is non-null in the
   region readout (the embeddings landed). Runs under the embed feature (e2e embed job).
5. **Landmark isolation:** the landmark diff/outcome is unaffected by the presence/absence of the telos
   branch (existing reconcile tests stay green; add one asserting a `telos:`-bearing request still produces
   the correct landmark counts).

---

## Scope

**In scope:** the manifest `telos:` section; the two wire types + outcome field; the shared
charter→(role,prose) mapping; the CLI charter-embed step; the `cogmap_charter_set` substrate function +
`charter_set` event (one additive migration); the backend telos diff/apply branch; the tests above.

**Rejected (load-bearing — resist scope creep):**
- A text-only migration delivery (Decision 1) — degenerate `telos_alignment`, unsearchable charter.
- A separate `set-charter` command/endpoint (Decision 2) — second vertical, fragmented L0 identity.
- A generic `set_resource_blocks` primitive (Decision 3) — YAGNI; telos is the sole caller.

**Deferred (in scope elsewhere or later):**
- Editing the immutable birth migration to seed the charter — forbidden (shipped-migration immutability);
  this delivery is the additive successor.
- L0 lifecycle cadence (when/who re-runs the reconcile per release) — the architecture spec states the
  principle (additive shipped content + operator-directed runs; ambient steward wake = never); this design
  makes the charter ride that same cadence, but the operational cadence itself is a separate thread.
- Tuning the charter prose — the content is settled in the charter design spec; this delivers it verbatim.

---

## Self-review notes

- **Spec coverage:** delivers task `019f0368` (charter statement + questions + framing onto live L0) and
  closes the "production delivery of L0 content" deferred thread from the charter design spec for the telos
  half (the landmark half shipped in #177).
- **Consistency:** the four decisions align — real embeddings (1) forces the operator path, which is the
  reconciler (2); the reconciler needs a populate-the-empty-telos primitive (3); the telos's distinct grain
  forces a distinct outcome field (4). Wire types, payload shape, and read function (`resource_blocks` with
  `block_role`) are consistent end to end.
- **Grounded:** every named function/column/commit was verified against the tree on 2026-06-28 (genesis
  `_project_cogmap_seeded:677`, `block_mutate:957`, `resource_blocks:371`, `cogmap_telos:396`,
  `TelosDef::block_specs` substrate model.rs:120, live `reconcile.rs` wire shape, reconciler commits
  0ce9172/e3609a0). The plan must re-verify the exact `body_hash_from_chunk_hashes` helper name and the
  chunks_packed unpack path against the landmark reconciler before authoring the telos branch.
- **Isolation:** the landmark slice stays literally unchanged; the telos branch is one gated block inside
  `reconcile_cognitive_map` calling one new substrate function; the manifest gains one section; the wire
  contract gains two types + one field. Each unit is independently testable.
