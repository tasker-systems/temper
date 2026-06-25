# WS6 Shim-Exit — native read shape (supersedes 2026-06-22)

Design spec for retiring `reconstruct_resource_row` and the §9 invariant-floor *reconstruction*,
so api/cli/mcp/ui read the **native** schema-derived shape instead of a fabricated reproduction of
the old production `ResourceRow`.

Backlog task `019ee5a4-710a`. **Supersedes**
[`2026-06-22-ws6-shim-exit-design.md`](2026-06-22-ws6-shim-exit-design.md), which predates the WS6
re-home (executed 2026-06-25, PR #168) and the #166 collapse: its file references are stale
(`next_backend.rs` → the code now lives in `db_backend.rs`), its "sequenced after the endgame
collapse" framing is satisfied (the endgame is **done**), and — critically — its timestamp premise
was wrong (it assumed timestamps were already event-derived and merely discarded; they are not; see
§3).

This is **Spec A** of a pair. **Spec B** (the `temper-substrate` / `temper-workflow` crate split) is
a sibling, sequenced immediately after — out of scope here (§7).

---

## Problem

`reconstruct_resource_row` (`crates/temper-api/src/backend/db_backend.rs:141-174`) answers reads in
the OLD production `ResourceRow` shape by reading the migration-invariant fields from
`readback::resource_row` (returning `ResourceRowParity`,
`crates/temper-next/src/readback/mod.rs:454-485`) and **fabricating the rest**:

| `ResourceRow` field | Shim fill today (`db_backend.rs`) | Native target |
|---|---|---|
| `created` / `updated` | `Utc::now()` at read time — **wrong every read** (`:161-162`) | the real `kb_resources.created` / `updated` columns |
| `kb_doc_type_id` | `Uuid::nil()` — §7 dissolved the typed id (`:154`) | **drop the field**; `doc_type_name` is authoritative |
| `slug` | `None` — §7-dissolved (`:157`) | **drop the field**; addressing is ref-by-id, decorated slug is presentation |
| `managed_hash` / `open_hash` | `None` (`:171-172`) | **drop both fields** |
| ids, profiles, title, context_name, doc_type_name, stage/mode/effort/seq, body_hash | read natively from `readback` | unchanged — already native |

Functional, but "shimmed, not evolved." Without this spec the shim — and the four dead fields on
`ResourceRow` it serves — are permanent, and temper-ui/cli keep consuming a fresh `now()` on every
read plus a nil id.

---

## Decisions (locked in brainstorming, 2026-06-25)

1. **Timestamps: surface the existing real columns.** `readback` SELECTs `kb_resources.created` /
   `updated` (already real insert/mutation timestamps). Not event-derived — that is explicitly
   deferred (§7). Cheapest path to genuinely-real timestamps with no schema change.
2. **Scope: native read shape only.** The `temper-substrate` / `temper-workflow` crate extraction is
   a sibling spec (Spec B), sequenced immediately after.
3. **Native type: evolve `ResourceRow` in place.** Remove the four dropped fields from the existing
   struct, keep the name. No new type, no dual-type migration. (`ResourceRow` re-homes to
   `temper-workflow` in Spec B regardless.)
4. **Doc type: name-only.** `doc_type_name` from `kb_properties` (`property_key='doc_type'`); drop
   `kb_doc_type_id`.

---

## Native surface shape

The evolved `ResourceRow` (`crates/temper-core/src/types/resource.rs:18-54`) after this spec:

- **Removed:** `kb_doc_type_id`, `slug`, `managed_hash`, `open_hash`.
- **Now real:** `created`, `updated` — sourced from `kb_resources`, not fabricated per-read.
- **Unchanged:** `id`, `kb_context_id`, `origin_uri`, `title`, `originator_profile_id`,
  `owner_profile_id`, `is_active`, `context_name`, `doc_type_name`, `owner_handle`,
  `stage`, `seq`, `mode`, `effort`, `body_hash`.

The type stays in `temper-core` with its `ts-rs` derive; one `generate-ts-types` run updates
`packages/temper-ui/src/lib/types/generated/resource.ts`, and temper-ui consumes the slimmer type.
Slug-less addressing is already the norm (trailing-UUID resolution; the CLI synthesizes the
decorated `sluggify(title)-<uuid>` form for display from `title` + `id`, never from a stored slug).

---

## Data flow

**Today:**

```
read_selector arm / db_backend method
  → readback::resource_row(pool, principal, id) → ResourceRowParity   (no timestamps)
  → reconstruct_resource_row(): Parity → ResourceRow, fabricating
      created/updated = Utc::now(), slug = None, kb_doc_type_id = nil, hashes = None
```

**Target:**

```
read_selector arm / db_backend method
  → readback::resource_row(...) → ResourceRow directly
      created/updated = real kb_resources columns; no fabricated fields
```

`readback::resource_row` is extended to SELECT `kb_resources.created` / `updated` and to build the
evolved `ResourceRow` directly (the intermediate `ResourceRowParity` collapses into it). With no
fields left to fabricate, **`reconstruct_resource_row` is deleted** — the literal "retire
`reconstruct_resource_row`" deliverable.

### Implementation gates (resolve during plan, not re-decisions)

- **chrono on temper-next sqlx.** `temper-next` sqlx deliberately carries no `chrono` feature, so its
  query macros cannot bind `timestamptz` today (`readback/mod.rs:490` comment). Add the `chrono`
  feature to `temper-next`'s sqlx dependency, then regenerate the per-crate offline cache with
  `cargo make prepare-next` (never `cargo sqlx prepare --workspace` — it clobbers per-crate caches).
- **Dependency direction.** Confirm `temper-next` may depend on the `temper-core` `ResourceRow` type
  so `readback` can construct it directly. If that dependency is awkward (cycle / feature-gating),
  fall back to a thin pure mapper in `temper-api` (`ResourceRowParity` + real timestamps →
  `ResourceRow`, still zero fabrication). Either way `reconstruct_resource_row` and its `Utc::now()`
  disappear.
- **`updated` maintenance.** Verify the resource mutation functions in
  `migrations/20260624000002_canonical_functions.sql` bump `kb_resources.updated = now()` on update.
  If they do not, the surfaced `updated` is meaningless — include the function fix in this spec's
  scope.

---

## Retiring the shim

- Delete `reconstruct_resource_row`; route its callers
  (`show_resource`, `create_resource` dedup + echo-back, `update_resource` pre-fetch,
  `delete_resource` pre-fetch, `reassert_relationship`, and the `read_selector`
  `list`/`show`/`search` arms) to the native `readback` read.
- Remove the §9 invariant-floor **reconstruction** path. The §9 floor remains the migration-time
  *correctness property* (proven once, historical); it is no longer a runtime read shape. Scrub the
  "reconstructing the production-shaped types at the §9 floor" doc-comments.

---

## Surface migration

Most of the ~17 read call sites route through the two central points
(`reconstruct_resource_row` and the `read_selector` arms), so changing the center covers them. The
per-surface residue:

- **ts-rs regen** (`cargo make generate-ts-types`) → updated `resource.ts`.
- **temper-ui** — grep for `kb_doc_type_id` / `slug` / `managed_hash` / `open_hash` on resource rows;
  drop any reference to the removed fields.
- **temper-mcp** — drop the stale claim that `managed_hash` / `open_hash` are "required"
  (`crates/temper-mcp/src/service.rs:177`); the tool already sets them `None`.
- **temper-cli** — the producer sites that set the dropped fields to `None`
  (`actions/show_cache.rs`, `actions/ingest.rs`, `commands/resource.rs`,
  `cloud_backend/translators.rs`) lose those fields with the struct change; the dead `show_cache`
  tier-2 hash-match branch (`actions/show_cache.rs:188-207`, already always
  `ServerHashesMissing`) is removed.

**Sequence:** temper-core type → readback/backend center (delete shim) → ts-rs regen → temper-ui →
delete dead code.

### Cosmetic cleanups folded in (opportunistic, low priority)

Since every `read_selector` arm is touched anyway: rename the `read_selector` misnomer to
`substrate_read` (it dispatches reads, it does not select a backend — the backend-switch machinery
was removed by #166), and scrub stale `NextBackend` doc-comments. Flagged **optional** so they never
block the core deliverable; drop them if they add review noise.

---

## Rejected / Deferred

**Rejected — event-sourcing the timestamps by adding `resource_id` to `kb_events`.** Events
*generate* mutations and not all are resource-bound; the `resource_id` carried in `kb_events.payload`
is for **replay only**, not a foreign key. We must not re-home resources *into* the event ledger. The
correct future direction for event-derived timestamps is a **resource/content-block → originating-event
pointer** (`kb_content_blocks` already carries `genesis_event_id` / `last_event_id`,
`canonical_schema.sql:551-552`), read in the resource→event direction.

**Deferred:**

- **True event-derived timestamps** via that correct pointer direction → the cogmap/event arc. This
  spec's surfaced `kb_resources.created`/`updated` are real and sufficient until then.
- **`temper-substrate` / `temper-workflow` crate split** → sibling **Spec B**, sequenced immediately
  after, with crate boundaries drawn around the now-native types.

---

## Risks / gates

- **F3 usability floor.** Each surface stays green through the migration. Extend the endgame's
  end-to-end surface-parity test to assert native fields: real timestamps (≠ read-time `now()`),
  absence of `kb_doc_type_id`, presence of `doc_type_name`.
- **Timestamp semantics now observable.** UI sort/filter sees real values instead of a per-read
  `now()` — intended. Gated on the `updated`-maintenance verification above; if mutation functions
  don't bump `updated`, sort-by-recent is wrong and the fix is in scope.
- **Hash drop blast radius — already safe.** The only reader, `show_cache` tier-2
  (`actions/show_cache.rs:188-207`), has been dead since the collapse (always
  `ServerHashesMissing` because the backend always emitted `None`); tests already assert the drop
  (`commands/resource.rs:1561,1616`; `tests/managed_hash_invariant_test.rs`). `body_hash` is
  unaffected — it is native on `kb_resources` and stays.

---

## Testing

- Extend the surface-parity e2e to the native assertions above (real timestamps, name-only doc type,
  dropped fields absent).
- Add a timestamp test: `created` stable and `updated` advances across a resource update (real, not
  `now()`-per-read).
- Keep `managed_hash_invariant_test`.
- After readback SQL changes: `cargo make prepare-next` then `cargo make test-next`; full gate via
  `cargo make check` (honest offline probe of the committed caches) + `cargo make test-e2e`.

---

## Sequencing (within the WS6 arc)

endgame collapse (done) → re-home to `public` (done, #168) → **shim-exit (this spec)** → crate split
(Spec B) → event-derived timestamps (cogmap/event arc).

Within shim-exit: evolve `ResourceRow` → extend `readback` (chrono + real timestamps, build
`ResourceRow`) → delete `reconstruct_resource_row` + route callers → ts-rs regen → temper-ui →
delete dead code → (optional) `read_selector` rename.

---

## References

- `crates/temper-api/src/backend/db_backend.rs:141-174` (`reconstruct_resource_row`)
- `crates/temper-api/src/backend/read_selector.rs` (the read arms; the misnomer)
- `crates/temper-next/src/readback/mod.rs:454-485` (`ResourceRowParity`, the SQL, the no-chrono note)
- `crates/temper-core/src/types/resource.rs:18-54` (`ResourceRow`)
- `crates/temper-mcp/src/service.rs:177` (stale hash-required claim)
- `migrations/20260624000001_canonical_schema.sql` (`kb_resources`, `kb_events`, `kb_content_blocks`,
  `kb_properties`), `…02_canonical_functions.sql` (mutation functions — `updated` maintenance)
- Superseded: `docs/superpowers/specs/2026-06-22-ws6-shim-exit-design.md`
- [[project_shared_kernel_two_domains]], [[project_neutral_api_temper_workflow]],
  [[project_ws6_flip_already_executed]]
