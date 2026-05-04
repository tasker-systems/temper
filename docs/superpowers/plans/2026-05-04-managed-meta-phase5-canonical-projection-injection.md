# Schema-Driven Managed-Meta — Phase 5: Canonical Projection-Key Injection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Execution status (final, 2026-05-04):** Phase 5 complete.
>
> | Task | Commit | Notes |
> |---|---|---|
> | 1 — Spec update (Q3 + dual-write framing) | `ee1599e` | clean; `### Phase 5 helper contract` placed after the dogfood-gate paragraph rather than inside the migration table (markdown subsections can't live inside a table) |
> | 2 — `ensure_managed_identity_keys` helper + tests | `84747e2` | clean; 4 unit tests committed; signature later refined in `2be0629` |
> | 3 — CLI send-side wiring | `d7170f9` | option 2 chosen for cloud-mode update path (defer to receive-side defense), comment in commands/resource.rs:1417 |
> | 4 — MCP send-side wiring | `2992551` | 3 sites: helper-on-Value × 2 + typed-direct via `ManagedMeta { title, slug, ..Default::default() }` for the title/slug PATCH path; the typed-direct route also flips a server-gated condition so identity-only PATCHes refresh the manifest |
> | 5 — API receive-side wiring + integration tests | `329b5c0` | wired in `ingest_service::ingest` + `resource_service::update`; the gate at line 583 of resource_service was extended to fire on `req.title.is_some()` / `req.slug.is_some()` so identity-only PATCHes rewrite the JSONB; new test file `crates/temper-api/tests/managed_hash_invariant_test.rs` lands 3 of the planned 4 tests |
> | (review fixup) | `2be0629` | code-review pass on Tasks 3-5 surfaced two real issues — the integration test was a tautology (rewrote to compute hash client-side BEFORE sending, then assert against post-storage server hash) and `slug.unwrap_or("")` would write empty-string `temper-slug` into the JSONB for slugless rows (changed helper to `Option<&str>` so column-NULL and key-absent agree); 6 helper unit tests now |
> | 6 — E2E hash-equality acceptance gate | `2f92b7f` | new test `phase5_local_canonical_hash_matches_server_managed_hash` in `tests/e2e/tests/show_cache_e2e_test.rs`; the spec/plan referred to "restoring a previously-removed test" but git history showed no such test was ever committed — the spec was capturing forward intent, this commit lands the actual gate |
> | 7 — Full quality bar | this commit | `cargo make check`, `cargo make test` (1015 / 1015), `cargo make test-db` (163 / 163 in temper-api), `cargo make test-e2e` (114 / 114), `cargo sqlx prepare --workspace --check` clean |
>
> **Plan deviation: meta_service preservation test dropped.** Plan listed 4 integration tests; only 3 landed. The fourth (`meta_service_update_meta_preserves_temper_title_in_jsonb`) would have asserted that a PATCH-without-title via PUT `/api/resources/{id}/meta` preserves the stored `temper-title` key. The current `meta_service::update_meta` writes the full payload-supplied `ManagedMeta` to the JSONB (line 137 of `meta_service.rs`) — if the caller's typed `ManagedMeta` has `title: None`, the JSONB drops the `temper-title` key. The test would have failed exposing a real defense-in-depth gap on the meta_service path. Closing the gap properly requires server-side hash recomputation (the path currently trusts caller-computed `payload.managed_hash`), which is bigger than Phase 5's scope. Captured as Phase-5 follow-up: `meta_service::update_meta` does not have receive-side defense; in practice the CLI sync flush always sends typed `ManagedMeta` with title/slug Some, so the canonical CLI path is correct. A non-CLI client could regress; revisit if observed.
>
> **Branch state:** `jct/wave1-shared-execution-paths-and-cloud-first-reframe`, 25 commits ahead of main. 8 commits this phase (`ee1599e`, `84747e2`, `d7170f9`, `2992551`, `329b5c0`, `2be0629`, `2f92b7f`, plus this finalization). Working tree clean.
>
> Phase 6 (DB migration to rewrite legacy stored JSONB), Phase 8 (re-enable show_cache tier-2), and Phase 9 (doctor fix legacy vault rewrite) are now unblocked.

**Goal:** Close the load-bearing hash-invariant gap that prevents show-cache tier-2 from working. After Phase 1, the typed `ManagedMeta` serializes `title` and `slug` as `temper-title` and `temper-slug` — but `IngestPayload` and `ResourceUpdateRequest` still carry top-level `title`/`slug` fields, and the CLI populates those rather than putting the keys inside `managed_meta`. Result: the local canonical-form `managed_hash` (computed over a JSONB containing `temper-title`/`temper-slug`) cannot match the server-computed `managed_hash` (computed over a JSONB without them). This plan introduces a shared `temper-core::operations` helper that injects `temper-title`/`temper-slug` into the `managed_meta` JSONB from the top-level identity fields, runs it on both the send side (CLI / MCP build paths) and the receive side (API ingest + update services), and adds the regression coverage the spec calls out.

**Architecture:** A pure, shared action `ensure_managed_identity_keys` lives in `crates/temper-core/src/operations/actions.rs` alongside `apply_defaults` and `merge_managed_meta`. It takes a `&mut serde_json::Value` (the `managed_meta` JSONB), a `title: &str`, and a `slug: &str`, and overwrites `temper-title` and `temper-slug` with the supplied values. Idempotent: running it twice produces the same output. Send-side callers run it before `compute_managed_hash` (so the local hash sees the canonical shape); receive-side callers run it before persisting and hashing (so a non-CLI client that didn't pre-canonicalize still produces a valid stored row). Top-level wire fields (`IngestPayload.title`, `IngestPayload.slug`, `ResourceUpdateRequest.title`, `ResourceUpdateRequest.slug`) and `kb_resources.title` / `kb_resources.slug` columns are unchanged — they are projections of the JSONB source by construction now.

**Tech Stack:** Rust 2021, serde_json, sqlx (for DB-backed integration tests).

**Specs:**
- `docs/superpowers/specs/2026-05-03-schema-driven-managed-meta-design.md` — Phase 5 row in the migration table.
- Updated as Task 1 of this plan to resolve open question Q3 (server-side title/slug column-extraction site) and to record the wire-vs-DB dual-write framing the user landed on 2026-05-04.

**Predecessor state (verified by direct read 2026-05-04):**
- `IngestPayload` (`crates/temper-core/src/types/ingest.rs:13-36`) carries `title: String` (line 14) and `slug: String` (line 21) as top-level fields, separate from `managed_meta: Option<serde_json::Value>` (line 28).
- `ResourceUpdateRequest` (`crates/temper-core/src/types/resource.rs:136-161`) carries `title: Option<String>` (line 138) and `slug: Option<String>` (line 140) alongside `managed_meta: Option<ManagedMeta>` (line 146).
- `ingest_service::ingest` (`crates/temper-api/src/services/ingest_service.rs:384-515`) writes `kb_resources.title`/`slug` from `payload.title`/`payload.slug` (lines 463-464) and stores `payload.managed_meta` JSONB unchanged after `strip_system_managed_fields` (lines 99-120). `compute_managed_hash` runs on the unmodified JSONB at line 309.
- `resource_service::update` (`crates/temper-api/src/services/resource_service.rs:524-721`) writes `kb_resources.title`/`slug` from `req.title`/`req.slug` (lines 552-568) and merges `req.managed_meta` via `apply_managed_meta_partial` (line 610). `compute_managed_hash` runs on the merged JSONB at line 619.
- `meta_service::update_meta` (`crates/temper-api/src/services/meta_service.rs:86-266`) operates on a typed `ManagedMeta` whose `title`/`slug` fields already serialize as `temper-title`/`temper-slug` after Phase 1 — no injection needed there. Cascade to columns at lines 155-172 already handles dual-write correctly. **Out of scope for code change**, but covered by a regression test in Task 5.
- `compute_managed_hash` (`crates/temper-core/src/hash.rs`) is the single hash function used by all three paths and by the CLI's local canonical-form hash. After Phase 1, the local form includes `temper-title`/`temper-slug`; this plan brings the server-stored form into agreement.

**Out of scope for this plan:**
- Wire-shape collapse (dropping top-level `title`/`slug` from `IngestPayload` and `ResourceUpdateRequest`). Decided 2026-05-04: keep the wire ergonomics; correctness is closed by injection. A follow-up issue captures the principled cleanup if drift is observed.
- DB migration to rewrite existing rows' `managed_meta` JSONB (Phase 6, blocked on this).
- `temper doctor fix` rewrite of legacy vault files (Phase 9).
- Re-enabling tier-2 in show_cache (Phase 8, blocked on this).
- The `date` field's relocation to open_meta (Phase 1 dropped it from managed-tier schemas; the DB rewrite is Phase 6).

This plan ends with: server-stored `managed_meta` JSONB always contains `temper-title` and `temper-slug` keys; local `managed_hash` and server `managed_hash` are byte-identical for any new resource; the test suite is green at every level.

---

## File Structure

**New files:** none.

**Modified files:**

| File | Change |
|---|---|
| `docs/superpowers/specs/2026-05-03-schema-driven-managed-meta-design.md` | Resolve open question Q3 with wire-vs-DB dual-write framing; refine Phase 5 row |
| `crates/temper-core/src/operations/actions.rs` | Add `ensure_managed_identity_keys` function + unit tests |
| `crates/temper-core/src/operations/mod.rs` | Re-export `ensure_managed_identity_keys` if needed for ergonomic callers |
| `crates/temper-cli/src/actions/ingest.rs` | Call helper inside `build_ingest_payload` after serializing typed `ManagedMeta` |
| `crates/temper-cli/src/commands/resource.rs` | Call helper inside the cloud-mode update path before `client.update(...)` (around line 1417) |
| `crates/temper-mcp/src/tools/resources.rs` | Call helper at the two `IngestPayload` build sites (lines 262, 477) and at the `ResourceUpdateRequest` build site (line 455) |
| `crates/temper-api/src/services/ingest_service.rs` | Call helper inside `ingest()` after `strip_system_managed_fields` and before `compute_managed_hash` |
| `crates/temper-api/src/services/resource_service.rs` | Call helper inside `update()` after `apply_managed_meta_partial` and before `compute_managed_hash` |
| `crates/temper-api/tests/managed_hash_invariant_test.rs` | New integration test: ingest a resource, recompute local hash from response, assert equality with stored `managed_hash` |
| `tests/e2e/tests/show_cache_e2e_test.rs` | Restore the previously-removed `tier2_hits_when_local_hashes_match_server_hashes` regression (the spec's primary acceptance gate). Note: full tier-2 re-enable lives in Phase 8; this restoration verifies only that hashes match end-to-end, with the tier-2 short-circuit code still in its current state |

**Conventions:**
- The helper signature is `pub fn ensure_managed_identity_keys(meta: &mut serde_json::Value, title: &str, slug: &str)`. It mutates in place. Empty-string title or slug is permitted (the schema layer rejects empty values; this helper does not gate on shape).
- Unit tests for the helper live in the existing `#[cfg(test)] mod tests` at the bottom of `actions.rs`.
- Each task that touches a `.rs` file ends with a per-crate `cargo nextest run -p <crate>` step. The final task runs the full quality gate (`cargo make check` + `cargo make test` + `cargo make test-db` + `cargo make test-e2e`).
- Per `feedback_plan_regression_guard_after_filter_test.md`: every step that runs a filtered test name pairs it with a full per-crate suite run before commit.

---

## Task 1: Update spec — resolve Q3 and clarify wire-vs-DB dual-write

**Files:**
- Modify: `docs/superpowers/specs/2026-05-03-schema-driven-managed-meta-design.md`

This task is documentation-only; no code changes, no test changes. Lands as its own commit so the spec edit is reviewable independently from the implementation.

- [ ] **Step 1: Update the "Open Questions" section**

In `docs/superpowers/specs/2026-05-03-schema-driven-managed-meta-design.md`, replace the body of open question Q3 with the resolution:

```markdown
3. **Server-side title/slug column extraction site (RESOLVED 2026-05-04):** Investigation showed `kb_resources.title`/`slug` are NOT extracted from `managed_meta` JSONB — they are populated from top-level `IngestPayload.title`/`slug` and `ResourceUpdateRequest.title`/`slug` fields. The asymmetry is the actual hash-invariant gap: the JSONB never had `temper-title`/`temper-slug` server-side, while the local canonical form does. Resolution: a shared `temper-core::operations::ensure_managed_identity_keys` helper injects the keys into the JSONB from the top-level fields, run on both the send side (CLI / MCP) and the receive side (`ingest_service::ingest`, `resource_service::update`) for defense in depth. See Phase 5 plan: `docs/superpowers/plans/2026-05-04-managed-meta-phase5-canonical-projection-injection.md`.
```

- [ ] **Step 2: Update the "Why dual-write not generated columns" section**

Replace the existing paragraph with one that distinguishes wire-level from DB-level dual-write:

```markdown
### Why dual-write not generated columns

Two layers of "dual-write" need separating:

1. **DB-level (intentional, retained):** `kb_resources.title`/`slug` columns AND `kb_resource_manifests.managed_meta` JSONB both carry the values. The columns are kept for query ergonomics — search facets, list ordering, sync replay all read them directly, and several SQL views depend on them. Replacing them with `GENERATED ALWAYS AS (managed_meta->>'temper-title') STORED` would close drift by construction but require a re-audit of every read path. Dual-write keeps reads semantically identical and confines the change to writes; revisitable as a one-shot additive change later if drift is observed.

2. **Wire-level (acceptable as ergonomic sugar):** `IngestPayload` and `ResourceUpdateRequest` carry top-level `title`/`slug` fields alongside `managed_meta`. After this work, the canonical source of truth is the JSONB; the top-level fields are a convenience for CLI/MCP callers who already have title and slug as scalars and don't want to re-pack them into JSONB themselves. The shared `ensure_managed_identity_keys` helper makes the wire dual-write impossible to skew: send-side and receive-side both run it, and the values come from a single in-memory source on each side. Wire collapse (dropping the top-level fields entirely) was considered and deferred — its blast radius (~50 sites including ts-rs codegen, MCP JsonSchema input shapes, OpenAPI spec, e2e tests) is disproportionate to the correctness gain when the helper already closes the gap.
```

- [ ] **Step 3: Update Phase 5 row in the migration table**

Replace the existing Phase 5 row with:

```markdown
| 5 | Server-side: canonical projection-key injection | `[controller]` | `temper-core::operations::ensure_managed_identity_keys` injects `temper-title`/`temper-slug` into `managed_meta` JSONB from the top-level identity fields. Called on both send-side (CLI/MCP build paths) and receive-side (`ingest_service::ingest`, `resource_service::update`) for defense in depth. New service test verifies `managed_meta` JSONB contains `temper-title`/`temper-slug` post-ingest; new integration test asserts `local_managed_hash == server.managed_hash` for any newly-ingested resource. `kb_resources.title`/`slug` columns continue to be populated from top-level fields. |
```

- [ ] **Step 4: Add a "Phase 5 helper contract" subsection**

After the Phase 6 row in the migration table, add a new subsection:

```markdown
### Phase 5 helper contract

`ensure_managed_identity_keys(meta: &mut serde_json::Value, title: &str, slug: &str)`:

- Coerces `meta` to a JSON object if it is not one already (replacing it with `{}` on a non-object value, since the alternative is silently dropping the data).
- Inserts or overwrites `meta["temper-title"] = title` and `meta["temper-slug"] = slug`.
- Idempotent: running twice with the same inputs produces the same output.
- Pure: no I/O, no dependencies beyond `serde_json`.

Callers: send-side runs it after serializing `ManagedMeta → Value` and before `compute_managed_hash` (so the local hash sees what the server will see); receive-side runs it after `strip_system_managed_fields` / `apply_managed_meta_partial` and before the server's own `compute_managed_hash` (so a non-CLI client that skipped injection still produces a canonical row).
```

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-05-03-schema-driven-managed-meta-design.md
git commit -m "$(cat <<'EOF'
docs(spec): resolve managed-meta Q3 — projection-key injection

Spec open question Q3 hypothesized server-side title/slug column extraction from
managed_meta JSONB. Investigation showed extraction does not happen — columns are
populated from top-level wire fields, while the JSONB never carried the
temper-prefixed keys server-side. The actual hash-invariant gap is the asymmetry
between local canonical form (includes temper-title/temper-slug) and server-stored
JSONB (does not).

Resolution: a shared temper-core::operations helper injects the keys into the JSONB
from the top-level identity fields, run on both send and receive sides. Wire-level
dual-write (top-level title/slug + JSONB) is acceptable as ergonomic sugar; DB-level
dual-write (columns + JSONB) is intentional for query ergonomics. Wire-collapse is
deferred as a follow-up cleanup if drift is observed.

Phase 5 plan in docs/superpowers/plans/2026-05-04-managed-meta-phase5-canonical-projection-injection.md.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add `ensure_managed_identity_keys` helper to `temper-core::operations::actions`

**Files:**
- Modify: `crates/temper-core/src/operations/actions.rs`

- [ ] **Step 1: Write the failing tests**

Append to the existing `#[cfg(test)] mod tests` at the bottom of `crates/temper-core/src/operations/actions.rs`:

```rust
#[test]
fn ensure_managed_identity_keys_inserts_when_absent() {
    let mut meta = serde_json::json!({"temper-stage": "backlog"});
    ensure_managed_identity_keys(&mut meta, "My Title", "my-slug");
    assert_eq!(meta["temper-title"], "My Title");
    assert_eq!(meta["temper-slug"], "my-slug");
    assert_eq!(meta["temper-stage"], "backlog");
}

#[test]
fn ensure_managed_identity_keys_overwrites_existing() {
    let mut meta = serde_json::json!({
        "temper-title": "Stale",
        "temper-slug": "stale-slug",
    });
    ensure_managed_identity_keys(&mut meta, "Fresh", "fresh-slug");
    assert_eq!(meta["temper-title"], "Fresh");
    assert_eq!(meta["temper-slug"], "fresh-slug");
}

#[test]
fn ensure_managed_identity_keys_is_idempotent() {
    let mut meta = serde_json::json!({});
    ensure_managed_identity_keys(&mut meta, "T", "s");
    let after_first = meta.clone();
    ensure_managed_identity_keys(&mut meta, "T", "s");
    assert_eq!(meta, after_first);
}

#[test]
fn ensure_managed_identity_keys_replaces_non_object_with_object() {
    let mut meta = serde_json::Value::Null;
    ensure_managed_identity_keys(&mut meta, "T", "s");
    assert!(meta.is_object());
    assert_eq!(meta["temper-title"], "T");
    assert_eq!(meta["temper-slug"], "s");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo nextest run -p temper-core ensure_managed_identity_keys
```

Expected: FAIL — function does not yet exist; tests do not compile.

- [ ] **Step 3: Implement the helper**

Add to `crates/temper-core/src/operations/actions.rs`, alongside `apply_defaults` (which lives at line 34):

```rust
/// Inject canonical identity keys (`temper-title`, `temper-slug`) into a
/// `managed_meta` JSONB value.
///
/// Called on both the send side (CLI / MCP build paths) before `compute_managed_hash`,
/// and on the receive side (server ingest / update services) before persisting and
/// hashing. Idempotent: running twice with the same inputs produces the same output.
///
/// If `meta` is not a JSON object, it is replaced with a fresh object containing
/// only the two identity keys. This handles the (unusual) case of a caller passing
/// `Value::Null` or a primitive; downstream validation (`schema::validate_frontmatter`)
/// will reject it on shape grounds, but the helper does not silently drop the data.
///
/// Empty-string title or slug is permitted; the schema layer is responsible for
/// rejecting empty identity values.
pub fn ensure_managed_identity_keys(meta: &mut Value, title: &str, slug: &str) {
    if !meta.is_object() {
        *meta = Value::Object(serde_json::Map::new());
    }
    let obj = meta.as_object_mut().expect("just-coerced to object");
    obj.insert("temper-title".to_owned(), Value::String(title.to_owned()));
    obj.insert("temper-slug".to_owned(), Value::String(slug.to_owned()));
}
```

- [ ] **Step 4: Re-export from `mod.rs` if needed**

Check `crates/temper-core/src/operations/mod.rs` for re-exports of other action helpers; if `apply_defaults` is re-exported there, add `ensure_managed_identity_keys` next to it. Otherwise leave the function importable as `temper_core::operations::actions::ensure_managed_identity_keys`.

- [ ] **Step 5: Run filtered tests to verify they pass**

```
cargo nextest run -p temper-core ensure_managed_identity_keys
```

Expected: 4 tests pass.

- [ ] **Step 6: Run full crate suite (regression guard)**

```
cargo nextest run -p temper-core
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-core/src/operations/actions.rs crates/temper-core/src/operations/mod.rs
git commit -m "$(cat <<'EOF'
feat(operations): add ensure_managed_identity_keys helper

Pure shared action that injects temper-title/temper-slug into a managed_meta
JSONB from the top-level identity fields. Idempotent and used on both send
side (CLI/MCP) and receive side (API services) to keep the local and
server-stored canonical forms byte-identical. Closes the hash-invariant gap
that has prevented show-cache tier-2 from working since inception.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Wire helper into CLI send-side build paths

**Files:**
- Modify: `crates/temper-cli/src/actions/ingest.rs`
- Modify: `crates/temper-cli/src/commands/resource.rs` (cloud-mode update path)

The CLI build paths are the canonical place to inject — local-mode `compute_managed_hash` runs over the same JSONB the cloud client sends, so injection here makes both consistent in one shot.

- [ ] **Step 1: Add a CLI integration test (or extend an existing one)**

Find the existing test in `crates/temper-cli/src/actions/ingest.rs` that exercises `build_ingest_payload` (or add one if absent). Append:

```rust
#[cfg(feature = "embed")]
#[test]
fn build_ingest_payload_injects_temper_title_and_slug_into_managed_meta() {
    let payload = build_ingest_payload(
        "# Hello\n\nbody",
        "Hello World",
        "test-ctx",
        "task",
        None,
        Some(temper_core::types::ManagedMeta {
            stage: Some("backlog".to_owned()),
            ..Default::default()
        }),
        None,
    )
    .unwrap();
    let managed = payload.managed_meta.expect("managed_meta set");
    assert_eq!(managed["temper-title"], "Hello World");
    assert_eq!(managed["temper-slug"], "hello-world");
    assert_eq!(managed["temper-stage"], "backlog");
}
```

- [ ] **Step 2: Run filtered test to verify failure**

```
cargo nextest run -p temper-cli --features embed build_ingest_payload_injects_temper_title_and_slug
```

Expected: FAIL — current code stores `managed_meta` without the identity keys.

- [ ] **Step 3: Wire `ensure_managed_identity_keys` into `build_ingest_payload`**

In `crates/temper-cli/src/actions/ingest.rs::build_ingest_payload`, after the `managed_meta_value` is built (line 163-166) and before constructing the `IngestPayload`, inject:

```rust
let mut managed_meta_value = managed_meta_value.unwrap_or_else(|| serde_json::json!({}));
temper_core::operations::actions::ensure_managed_identity_keys(
    &mut managed_meta_value,
    title,
    &slug,
);
let managed_meta_value = Some(managed_meta_value);
```

The `unwrap_or_else` handles the case where the caller passes `None` for `managed_meta` — we still need an object to inject into, so the post-injection state always has an object with at minimum `temper-title` and `temper-slug`.

- [ ] **Step 4: Run filtered test to verify pass**

```
cargo nextest run -p temper-cli --features embed build_ingest_payload_injects_temper_title_and_slug
```

Expected: PASS.

- [ ] **Step 5: Wire into the cloud-mode update path in `commands/resource.rs`**

Read `crates/temper-cli/src/commands/resource.rs` around line 1417 (the `ResourceUpdateRequest` construction site). Inject before the `client.update(...)` call. Two cases to handle:

1. The request includes `req.managed_meta = Some(managed)`: serialize to JSON, run the helper with `req.title.unwrap_or(&current_title)` and `req.slug.unwrap_or(&current_slug)`, deserialize back, attach.
2. The request omits `managed_meta` entirely: leave it as `None` — the server-side helper (Task 4) will inject defensively from the top-level fields.

The exact splice point depends on the current shape of that function — read it first; if the current code path already has access to the merged `ManagedMeta` and the resolved title/slug, pass them through. If not, the simpler change is option 2 (leave it to server-side defense) and add a comment noting the partial-update case relies on the receive-side helper.

- [ ] **Step 6: Add a regression test for the cloud-mode update path**

In `crates/temper-cli/src/commands/resource.rs` (or its test module), add a unit test that builds the cloud-mode update request with a `managed_meta` partial and asserts the JSONB contains `temper-title` / `temper-slug` after injection. If the test path is awkward (the function may not be easily callable in isolation), defer the regression coverage to the e2e test in Task 6 and document the deferral with an inline comment.

- [ ] **Step 7: Run full crate suite (regression guard)**

```
cargo nextest run -p temper-cli
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/actions/ingest.rs crates/temper-cli/src/commands/resource.rs
git commit -m "$(cat <<'EOF'
feat(cli): inject temper-title/temper-slug into managed_meta on send

Wires ensure_managed_identity_keys into build_ingest_payload and the cloud-mode
update path in commands/resource.rs. Local compute_managed_hash now runs over a
JSONB shape that matches what the server will store and hash, closing the local
side of the invariant.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Wire helper into MCP send-side build paths

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs`

Three sites (lines 262, 477, 455 from the grep). Each constructs a wire payload that flows to the same in-process services as the API; pre-injecting on the send side keeps the MCP path symmetric with the CLI.

- [ ] **Step 1: Add MCP-side test fixtures or piggy-back on integration coverage**

The MCP tool functions are async and require a `TemperMcpService` context, so unit-testing in isolation is awkward. Defer regression coverage to the integration tests added in Task 5 — they cover the receive side, which is the actual correctness contract. The MCP-side injection is defense in depth; if it skipped, Task 4 (server-side) catches it.

- [ ] **Step 2: Inject at each `IngestPayload` build site**

For each of `crates/temper-mcp/src/tools/resources.rs:262` and `:477`, after the `managed_meta` JSON is built and before constructing the `IngestPayload`:

```rust
let mut managed_meta_value: serde_json::Value =
    serde_json::to_value(&managed_meta_struct).unwrap_or_else(|_| serde_json::json!({}));
temper_core::operations::actions::ensure_managed_identity_keys(
    &mut managed_meta_value,
    &input.title,
    &input.slug,  // or wherever the slug is sourced
);
```

then attach `Some(managed_meta_value)` to the payload.

- [ ] **Step 3: Inject at the `ResourceUpdateRequest` build site (line 455)**

Same pattern: serialize the typed `ManagedMeta` partial to a `Value`, inject from the request's title/slug or the resolved current values, deserialize back to `ManagedMeta` if the request expects the typed shape — or attach as `Value` if the wire shape allows. Read the exact call site to determine which form is expected.

- [ ] **Step 4: Run full crate suite (regression guard)**

```
cargo nextest run -p temper-mcp
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-mcp/src/tools/resources.rs
git commit -m "$(cat <<'EOF'
feat(mcp): inject temper-title/temper-slug into managed_meta on send

Symmetric with the CLI send-side wiring. MCP tool calls now produce canonical
managed_meta JSONB before reaching the in-process ingest/update services. The
receive-side helper (next commit) is the defense-in-depth backstop.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Wire helper into API receive-side services + add regression tests

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs`
- Modify: `crates/temper-api/src/services/resource_service.rs`
- New: `crates/temper-api/tests/managed_hash_invariant_test.rs`

This is the load-bearing receive-side wiring. After this lands, even a non-CLI / non-MCP client that POSTs to `/api/ingest` with bare `title`/`slug` and an empty `managed_meta` will end up with a canonical row.

- [ ] **Step 1: Write the failing integration tests**

Create `crates/temper-api/tests/managed_hash_invariant_test.rs`:

```rust
//! Hash-invariant test — local canonical-form managed_hash must equal
//! server-stored managed_hash for any resource ingested through the API.
//! This is the spec's primary acceptance gate for Phase 5 and the
//! prerequisite for re-enabling show-cache tier-2 in Phase 8.

#![cfg(feature = "test-db")]

use temper_api::services::ingest_service;
use temper_core::hash::compute_managed_hash;
use temper_core::types::ingest::IngestPayload;

mod common;
use common::TestDb;

#[tokio::test]
async fn ingest_stores_temper_title_and_temper_slug_in_managed_meta_jsonb() {
    let db = TestDb::new().await;
    // ... seed profile + context, then ingest a task ...
    // Assert: the stored managed_meta JSONB has temper-title and temper-slug keys.
    todo!("seed + ingest + fetch manifest + assert keys present");
}

#[tokio::test]
async fn server_managed_hash_equals_local_canonical_hash_post_ingest() {
    let db = TestDb::new().await;
    // ... ingest a task with managed_meta = {temper-stage: backlog} ...
    // Fetch stored managed_meta JSONB from manifest.
    // Compute local hash using compute_managed_hash with the SAME doc_type and
    // the JSONB read from the manifest.
    // Assert byte-equality with the stored managed_hash column.
    todo!("invariant assertion");
}

#[tokio::test]
async fn partial_patch_with_top_level_title_change_updates_jsonb_temper_title() {
    let db = TestDb::new().await;
    // ... ingest a task, then PATCH with req.title = Some("Renamed") ...
    // Fetch managed_meta JSONB.
    // Assert: managed_meta["temper-title"] == "Renamed".
    todo!("partial-update injection verification");
}

#[tokio::test]
async fn meta_service_update_meta_preserves_temper_title_in_jsonb() {
    let db = TestDb::new().await;
    // ... ingest a task with title "Original" ...
    // Build a MetaUpdatePayload that touches temper-stage but leaves title None.
    // Call meta_service::update_meta.
    // Fetch managed_meta JSONB.
    // Assert: managed_meta["temper-title"] == "Original" (still there).
    todo!("meta-service path regression");
}
```

The `todo!()` placeholders become real bodies in the implementation step. Reuse the existing `common` test harness in `crates/temper-api/tests/common/` (look at `crates/temper-api/tests/resources_test.rs` for the established seeding pattern).

- [ ] **Step 2: Run filtered tests to verify failure (compile error from `todo!`)**

```
cargo nextest run -p temper-api --features test-db managed_hash_invariant
```

Expected: tests panic with `not yet implemented` — the bodies are stubbed.

- [ ] **Step 3: Implement the test bodies**

Flesh out each test. Use `ingest_service::ingest` directly for the seed; use `resource_service::update` and `meta_service::update_meta` for the PATCH/PUT paths; use `sqlx::query_scalar!` to read the stored `managed_meta` and `managed_hash` columns from `kb_resource_manifests`.

- [ ] **Step 4: Run filtered tests to verify failure (real assertions now)**

```
cargo nextest run -p temper-api --features test-db managed_hash_invariant
```

Expected: tests fail because the receive-side helper is not yet wired.

- [ ] **Step 5: Wire `ensure_managed_identity_keys` into `ingest_service::ingest`**

In `crates/temper-api/src/services/ingest_service.rs::ingest` (line 384), after `apply_doc_type_defaults` runs at line 403 and before `validate_managed_meta` at line 411, inject:

```rust
temper_core::operations::actions::ensure_managed_identity_keys(
    &mut managed,
    &payload.title,
    &payload.slug,
);
```

The `managed` value here is the `serde_json::Value` returned from `strip_system_managed_fields`. Order rationale: defaults run first (so we don't clobber doctype-supplied defaults), then injection (so identity keys are present for validation), then validation (which sees the canonical shape).

- [ ] **Step 6: Wire into `resource_service::update`**

In `crates/temper-api/src/services/resource_service.rs::update` (line 524), after `apply_managed_meta_partial` runs at line 610 and before `serde_json::to_value` at line 618, inject:

```rust
let mut managed_value = serde_json::to_value(&merged_managed)?;
temper_core::operations::actions::ensure_managed_identity_keys(
    &mut managed_value,
    new_title,
    new_slug.unwrap_or(""),
);
```

`new_title` and `new_slug` are already computed at lines 552-553. Note: `new_slug` is `Option<&str>`; the `.unwrap_or("")` keeps the helper non-fallible — empty slug is a schema-validation concern, not a helper concern.

- [ ] **Step 7: Run filtered tests to verify pass**

```
cargo nextest run -p temper-api --features test-db managed_hash_invariant
```

Expected: 4 tests pass.

- [ ] **Step 8: Run full per-crate suites (regression guard)**

```
cargo nextest run -p temper-api
cargo nextest run -p temper-api --features test-db
```

Expected: all tests pass.

- [ ] **Step 9: Regenerate sqlx prepare cache if needed**

```
cargo sqlx prepare --workspace -- --all-features
```

If `.sqlx/` has changes, stage and include in this commit.

- [ ] **Step 10: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs \
        crates/temper-api/src/services/resource_service.rs \
        crates/temper-api/tests/managed_hash_invariant_test.rs \
        .sqlx/
git commit -m "$(cat <<'EOF'
feat(api): inject temper-title/temper-slug on receive + invariant tests

Wires ensure_managed_identity_keys into ingest_service::ingest and
resource_service::update as the receive-side defense. Adds the integration
test that asserts local_managed_hash == server.managed_hash for any newly-
ingested resource — the primary acceptance gate for Phase 5 and the
prerequisite for re-enabling show-cache tier-2 in Phase 8.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Restore the e2e tier-2 hash-equality regression

**Files:**
- Modify: `tests/e2e/tests/show_cache_e2e_test.rs`

The previously-removed `tier2_hits_when_local_hashes_match_server_hashes` regression is the spec's primary acceptance gate. Restore the body of the test, but assert only the hash-equality precondition — not the full tier-2 short-circuit, which Phase 8 will re-enable. This separates "hashes can match" (Phase 5) from "show_cache trusts that they match" (Phase 8).

- [ ] **Step 1: Locate the removed test in git history**

```
git log --all --oneline -- tests/e2e/tests/show_cache_e2e_test.rs | head -20
git log -p --all -S 'tier2_hits_when_local_hashes_match_server_hashes' -- tests/e2e/
```

Find the commit that removed the test; copy the test body as a starting point.

- [ ] **Step 2: Adapt the test for the Phase 5 acceptance contract**

The restored test should:
1. `temper resource create` against a real Axum + Postgres harness.
2. Fetch the resource via `/api/resources/{id}/meta`.
3. Compute local canonical-form `managed_hash` from the response's `managed_meta`.
4. Assert byte-equality with the response's `managed_hash` field.

Name the test `phase5_local_canonical_hash_matches_server_managed_hash` to signal that this is the Phase 5 gate, distinct from the Phase 8 `tier2_hits_when_local_hashes_match_server_hashes` (which Phase 8 will add when it re-enables the tier-2 short-circuit).

- [ ] **Step 3: Run the e2e test in isolation**

```
cargo make test-e2e -- phase5_local_canonical_hash_matches_server_managed_hash
```

Expected: PASS.

- [ ] **Step 4: Run full e2e suite (regression guard)**

```
cargo make test-e2e
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/tests/show_cache_e2e_test.rs
git commit -m "$(cat <<'EOF'
test(e2e): restore Phase 5 hash-equality regression

Asserts local canonical-form managed_hash equals server-stored managed_hash
end-to-end, via a real Axum + Postgres harness. This is the precondition
for Phase 8's tier-2 re-enable; the Phase 8 commit will add the tier-2
short-circuit assertion on top of this.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Full quality gate

**Files:**
- None (verification only)

Per `feedback_subagent_check_before_commit.md`: this gate runs before any subagent commits and before the controller signs off on the phase as a whole. The pre-commit hook is the backstop, not the first line.

- [ ] **Step 1: Run cargo make fix to normalize formatting**

```
cargo make fix
```

If anything changed, stage and commit as a separate `chore(fmt): cargo make fix` commit — do not bundle with implementation work.

- [ ] **Step 2: Run cargo make check**

```
cargo make check
```

Expected: fmt + clippy + machete + ts-typecheck + biome all green. Any failure → STOP and report. Do not soften, refactor, or bypass.

- [ ] **Step 3: Run cargo make test (workspace unit + integration)**

```
cargo make test
```

Expected: all tests pass.

- [ ] **Step 4: Run cargo make test-db**

```
cargo make docker-up
cargo make test-db
```

Expected: all DB-backed tests pass, including the new `managed_hash_invariant_test.rs`.

- [ ] **Step 5: Run cargo make test-e2e**

```
cargo make test-e2e
```

Expected: all e2e tests pass, including the restored hash-equality regression.

- [ ] **Step 6: Verify sqlx prepare cache is clean**

```
cargo sqlx prepare --workspace --check -- --all-features
```

Expected: clean. If dirty, regenerate and commit as `chore(sqlx): refresh prepare cache`.

- [ ] **Step 7: Manual dogfood**

Per the spec's "Dogfood gate before merge":

```bash
# Against a fresh-ingested vault on the dev server:
temper resource show <some-real-slug>
temper resource show <same-slug>  # second call
```

Verify the second call's local-form `managed_hash` matches the server's. (Tier-2 short-circuit is still off — that's Phase 8 — but the underlying invariant must hold.)

- [ ] **Step 8: Update plan execution status**

Edit the top of this plan document to add an "Execution status (final)" callout block similar to the Phase 1 plan, with one row per task and the landing commit SHA.

- [ ] **Step 9: Commit the plan-status update**

```bash
git add docs/superpowers/plans/2026-05-04-managed-meta-phase5-canonical-projection-injection.md
git commit -m "$(cat <<'EOF'
docs(plans): finalize managed-meta phase 5 — projection-key injection landed

All 7 tasks landed; full quality gate green. Phase 8 (re-enable tier-2 in
show_cache) and Phase 6 (DB migration to rewrite legacy stored JSONB) are
now unblocked.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Validation-Agent-Pass Checklist

A subagent dispatched against any task in this plan MUST run this checklist before reporting completion. Failure of any step → BLOCKED, not workaround.

```
1. cargo make check                                                      # fmt + clippy + machete + ts-typecheck + biome
2. cargo nextest run -p <crate>                                          # full per-crate suite for any crate touched
3. cargo make test                                                       # workspace unit + integration tests
4. cargo make test-db                                                    # DB-backed tests (only if the task touches API/services)
5. cargo make test-e2e                                                   # e2e tests (only for Task 6 and Task 7)
6. grep -n 'todo!\|unimplemented!' crates/temper-core/src/operations/    # → expected zero in production code (test-only todo! in Task 5 step 1 is intentional and resolved in step 3)
7. Read the diff for the task. Confirm: NO "for now" comments. NO "until X reconciled" comments. NO new TODOs without ticket links. (See feedback_no_ship_for_now_workarounds.md.)
```

Per `feedback_subagent_escalate_not_soften.md`: if a test fails because the contract under test is wrong, STOP and report BLOCKED. Do not silently relax assertions or swallow errors.

## Open Questions (resolve during implementation)

1. **MCP `update_resource_meta` shape:** the MCP tool's `MetaUpdatePayload` is constructed from typed `ManagedMeta` (resources.rs:531), not from a raw `Value`. The serde renames from Phase 1 already produce canonical keys here. Defer regression coverage to the integration tests in Task 5 that exercise `meta_service::update_meta`.

2. **`test-embed` feature flag for CLI test in Task 3:** `build_ingest_payload` is gated on `embed`; the new test is too. If the local toolchain lacks ONNX Runtime, the test will skip. Confirm CI's "Embed" job runs it.

3. **Phase 8 boundary check:** confirm at the end of this plan that show_cache tier-2 is still in its currently-disabled state (commenting out the hash-comparison short-circuit). Phase 8 turns it back on; Phase 5 only ensures the precondition holds.
