# Managed-meta boundary — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `managed_meta`'s closed, temper-owned vocabulary enforced at the type boundary and legible across every caller surface — reject caller-invented managed keys, point them at `open_meta`, and document slug precedence.

**Architecture:** `ManagedMeta` becomes a closed serde type (`deny_unknown_fields`, no `extra` catch-all). Because that one type is shared by the MCP tool inputs (schemars), the API payloads (utoipa), and the CLI's typed construction, the rejection and the guidance propagate to all surfaces from a small set of edits. No wire-shape restructuring — that is Phase 2.

**Tech Stack:** Rust (serde, schemars, utoipa, clap, axum, rmcp), cargo-make, cargo-nextest.

## Global Constraints

- **No backward compatibility.** Cloud-native only; the local vault is read-only ripgrep scratch. No shims, no deprecation windows, no dual-read paths.
- **Full surface parity.** MCP + CLI + API move together; skill docs update in-arc. One shared logic layer.
- **Design source of truth:** `docs/superpowers/specs/2026-07-07-managed-open-meta-boundary-reshape-design.md`. This plan implements its **Phase 1** only (P1.1 reject, P1.2 discoverability, P1.3 slug docs). Phase 2 (shrinking `ManagedMeta` to Property-only, promoting identity to wire fields) is a **separate** plan — do not start it here.
- **The invariant (Phase 2's end state, stated so Phase 1 doesn't fight it):** `managed_meta` = exactly the `KeyFate::Property` keys. Phase 1 does **not** remove `temper-title`/`temper-slug` from `ManagedMeta` yet; it only closes the type and documents the precedence.
- **Verification:** `cargo make check` (fmt + clippy + docs + machete + TS) must pass before every commit. Tests run against real Postgres (Docker on :5437). Rebuild the CLI bin after CLI changes (`cargo build -p temper-cli --bin temper`) — e2e does not rebuild it.
- **Commit style:** `fix(meta):` / `docs(meta):` / `test:` scoped prefixes.

---

## Task 1: Close the `ManagedMeta` vocabulary at the type boundary

Delete the `extra` catch-all and mark `ManagedMeta` `#[serde(deny_unknown_fields)]`, making a caller-invented managed key a deserialization error everywhere the type is parsed. Verified-safe: readback (`substrate_read.rs:253`) only ever reconstructs the closed 10-key property vocabulary, and `split_managed_open` only ever routes `temper-`-prefixed (typed) keys into the managed tier, so no live read/projection path carries an unknown key into `ManagedMeta`.

**Files:**
- Modify: `crates/temper-workflow/src/types/managed_meta.rs:9-110` (struct doc-comment, derive attrs, remove `extra` field) + its test module (`:311-334`)
- Modify: `crates/temper-workflow/src/operations/actions.rs:310-313` (drop `extra` merge in `merge_managed_meta`) + test `:665-678`
- Modify: `crates/temper-workflow/src/frontmatter/document.rs:246,304-307` (drop `extra` render loop in `set_managed_meta` + doc-comment)
- Test: `crates/temper-workflow/src/types/managed_meta.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `ManagedMeta` with no `extra` field and `#[serde(deny_unknown_fields)]`. `serde_json::from_value::<ManagedMeta>(v)` returns `Err` iff `v` contains any key not named by a typed field. Consumed by Tasks 2 and 3.

- [ ] **Step 1: Write the failing test — unknown key is rejected**

Replace the stale round-trip test at `managed_meta.rs:311-334` (`managed_meta_extras_bucket_round_trips_unknown_fields`) with a rejection test:

```rust
#[test]
fn managed_meta_rejects_unknown_keys() {
    // `managed_meta` is a closed, temper-owned vocabulary. A key the typed
    // struct does not name (e.g. `date`, or a caller-invented tag) is not a
    // managed key and must be rejected, not silently absorbed.
    let json = r#"{"temper-type":"session","temper-title":"test","date":"2026-04-13"}"#;
    let err = serde_json::from_str::<ManagedMeta>(json).unwrap_err();
    assert!(
        err.to_string().contains("date"),
        "rejection must name the offending key, got: {err}"
    );

    // A caller-invented key is likewise rejected.
    assert!(
        serde_json::from_str::<ManagedMeta>(r#"{"my-tag":"x"}"#).is_err(),
        "arbitrary caller keys belong in open_meta, not managed_meta"
    );
}

#[test]
fn managed_meta_accepts_the_closed_vocabulary() {
    // Every typed managed key deserializes cleanly — the readback/projection
    // shape (only vocabulary keys) still round-trips.
    let json = r#"{"temper-type":"task","temper-stage":"backlog","temper-mode":"build",
        "temper-effort":"small","temper-seq":3,"temper-branch":"b","temper-pr":"p",
        "temper-status":"active","temper-provenance":"llm-discovered",
        "temper-llm-model":"claude","temper-llm-run":"01947b5c-0000-0000-0000-000000000000",
        "temper-title":"T","temper-slug":"t"}"#;
    let parsed: ManagedMeta = serde_json::from_str(json).expect("closed vocabulary must parse");
    assert_eq!(parsed.stage.as_deref(), Some("backlog"));
    assert_eq!(parsed.slug.as_deref(), Some("t"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run -p temper-workflow managed_meta_rejects_unknown_keys managed_meta_accepts_the_closed_vocabulary`
Expected: `managed_meta_rejects_unknown_keys` FAILS (today `date` lands in `extra`, no error). Compile may also fail once Step 3 lands — that's fine; the point is red-before-green.

- [ ] **Step 3: Remove the `extra` field and close the struct**

In `managed_meta.rs`, replace the struct doc-comment (`:9-23`) and derive block, and delete the `extra` field (`:100-109`):

```rust
/// Temper-governed frontmatter fields for a vault resource.
///
/// This is a **closed, temper-owned vocabulary**: every managed key has a
/// typed field below. There is no catch-all — a key not named here is not a
/// managed key. Caller-defined ("bring-your-own") fields belong in `open_meta`,
/// the free-form tier. Deserialization rejects unknown keys
/// (`#[serde(deny_unknown_fields)]`) so a mis-filed key fails loudly at the
/// wire boundary instead of silently migrating tiers.
///
/// All fields use `temper-*` YAML/JSON key names via `serde(rename)`.
/// `None` fields are omitted from serialized output.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "managed_meta.ts"))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ManagedMeta {
    // ... all existing typed fields unchanged ...
```

Delete the `extra` field and its doc-comment entirely (the `#[serde(flatten)] pub extra: HashMap<String, Value>` block at `:100-109`). Remove the now-unused `use std::collections::HashMap;` (`:1`) **only if** no other code in the file needs it (the test module uses it at `:215` — keep the file-level import if tests still reference it; if clippy flags it, scope the import into the test module).

> Note: `deny_unknown_fields` is legal here only because removing `extra` removes the struct's sole `#[serde(flatten)]`. serde forbids the two together. No parent struct flattens `ManagedMeta` (it is always a named field), so nothing else conflicts.

- [ ] **Step 4: Fix the two `extra` consumers**

In `actions.rs`, delete the `extra` merge tail of `merge_managed_meta` (`:310-313`):

```rust
    if patch.slug.is_some() {
        existing.slug = patch.slug;
    }
}
```

(Remove the `// Merge extra HashMap key-by-key.` comment and the `for (k, v) in patch.extra { existing.extra.insert(k, v); }` loop.)

In the `merge_managed_meta` test (`actions.rs:665-678`), delete the `extra`-manipulating lines (`existing.extra.insert(...)`, `patch.extra.insert(...)`, and the three `assert_eq!(existing.extra.get(...))` assertions). Keep the rest of the test asserting typed-field `Some`-wins merge.

In `document.rs`, delete the `extra` render loop in `set_managed_meta` (`:304-307`):

```rust
        if let Some(ref v) = meta.slug {
            self.set_raw_field("temper-slug", serde_json::Value::String(v.clone()));
        }
    }
```

(Remove the `// Apply extra bucket last` comment and the `for (key, value) in &meta.extra { ... }` loop.) Update the method doc-comment at `:246` to drop the "Fields in `meta.extra` are ..." sentence.

- [ ] **Step 5: Run the workflow crate suite to verify green**

Run: `cargo nextest run -p temper-workflow`
Expected: PASS — including the two new tests and the updated merge test. Then `cargo make check` (clippy will catch any dangling `HashMap`/`Value` imports).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-workflow/src/types/managed_meta.rs \
        crates/temper-workflow/src/operations/actions.rs \
        crates/temper-workflow/src/frontmatter/document.rs
git commit -m "fix(meta): close ManagedMeta vocabulary (deny_unknown_fields, drop extra bucket)

managed_meta is a closed, temper-owned vocabulary; a key the typed struct
does not name is not a managed key and is now rejected at deserialize
instead of silently migrating to open_meta via the extra catch-all.
Safe: readback and split_managed_open only ever carry vocabulary keys.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Enforce + explain the rejection at the MCP and API surfaces

The type change from Task 1 already makes both surfaces reject unknown keys (MCP at the rmcp input parse; API at the handler `from_value`). This task makes the rejection *legible*: the shared `ManagedMeta` doc-comment (Task 1) already propagates the closed-vocab framing into the MCP schemars schema and the utoipa OpenAPI schema; here we (a) enrich the API's runtime error to point at `open_meta`, (b) add the same pointer to the MCP input field docs, and (c) prove rejection end-to-end on both surfaces.

**Files:**
- Modify: `crates/temper-api/src/handlers/ingest.rs:75,138` (enrich the `invalid managed_meta` error on create + update)
- Modify: `crates/temper-mcp/src/tools/resources.rs:58-63,137-145,170-172` (field doc-comments on the three inputs)
- Test: `tests/e2e/tests/` (new: API ingest rejection) + `crates/temper-mcp/src/tools/resources.rs` test module (MCP input parse rejection)

**Interfaces:**
- Consumes: `ManagedMeta` closed type (Task 1).
- Produces: a stable error substring — `"caller-defined keys belong in open_meta"` — used by the e2e assertion.

- [ ] **Step 1: Write the failing MCP unit test — unknown managed key rejected at input parse**

In the `#[cfg(test)] mod tests` of `crates/temper-mcp/src/tools/resources.rs`, add (fill any additional required fields by consulting `CreateResourceInput` at `resources.rs:26-70` — as of this writing `title`, `doc_type_name`, and a home are the hard requirements):

```rust
#[test]
fn create_input_rejects_unknown_managed_key() {
    let json = serde_json::json!({
        "title": "T",
        "doc_type_name": "task",
        "context_ref": "@me/temper",
        "managed_meta": { "my-tag": "x" }
    });
    let err = serde_json::from_value::<CreateResourceInput>(json).unwrap_err();
    assert!(
        err.to_string().contains("my-tag"),
        "unknown managed key must be rejected at input parse, got: {err}"
    );
}
```

- [ ] **Step 2: Run to verify it fails (before Task 1 is merged) / passes shape**

Run: `cargo nextest run -p temper-mcp create_input_rejects_unknown_managed_key`
Expected: PASS once Task 1 is in (the shared type now denies unknown fields). If run in isolation before Task 1, it FAILS (key absorbed into `extra`). This test is the MCP-surface proof that the closed type reaches the tool boundary.

- [ ] **Step 3: Enrich the API runtime error**

In `crates/temper-api/src/handlers/ingest.rs`, at both deserialize sites (create `:75`, update `:138`), change the error wrap to point at `open_meta`:

```rust
        .map_err(|e| ApiError::BadRequest(format!(
            "invalid managed_meta: {e}. managed_meta is a closed vocabulary; \
             caller-defined keys belong in open_meta"
        )))?,
```

Apply the identical message at both sites (create and the update/PATCH path) so the two paths stay in lockstep.

- [ ] **Step 4: Add the pointer to the MCP input field docs**

In `crates/temper-mcp/src/tools/resources.rs`, extend the `managed_meta` field doc-comment on all three input structs (`CreateResourceInput:58-63`, `UpdateResourceInput:137-145`, `UpdateResourceMetaInput:170-172`). schemars renders these into the tool schema:

```rust
    /// Managed (temper-*) frontmatter — a **closed, temper-owned vocabulary**.
    /// Only the typed temper-* keys are accepted; an unknown key is rejected.
    /// Caller-defined ("bring-your-own") fields belong in `open_meta`, the
    /// free-form tier.
    #[serde(default)]
    pub managed_meta: Option<ManagedMeta>,
```

(For `UpdateResourceMetaInput` the field is non-`Option` — keep its shape, update only the doc text.)

- [ ] **Step 5: Write the failing e2e API rejection test**

Add an e2e test in `tests/e2e/tests/` (a new file, e.g. `managed_meta_reject_test.rs`, or the nearest existing ingest test file). It drives the real Axum server + Postgres:

```rust
#![cfg(feature = "test-db")]
// ... use the common harness (see tests/e2e/tests/common) ...

#[tokio::test]
async fn ingest_rejects_unknown_managed_key() {
    let ctx = TestContext::new().await; // per the common harness pattern
    let body = serde_json::json!({
        "context_ref": ctx.context_ref(),
        "doc_type_name": "task",
        "title": "reject me",
        "slug": "reject-me",
        "content": "body",
        "managed_meta": { "not-a-managed-key": "boom" },
        "open_meta": {}
    });
    let resp = ctx.post_json("/api/ingest", &body).await;
    assert_eq!(resp.status(), 400, "unknown managed key must 400");
    let text = resp.text().await;
    assert!(
        text.contains("caller-defined keys belong in open_meta"),
        "error must point the caller at open_meta, got: {text}"
    );
}
```

> Match the actual harness constructor/method names in `tests/e2e/tests/common/` — the exact `TestContext` API is established there; mirror a neighboring ingest test rather than inventing helpers.

- [ ] **Step 6: Run to verify it fails, then implement is already done — verify it passes**

Run: `cargo make prepare-e2e` (only if SQL changed — it did not here) then
`cargo make docker-up && cargo make test-e2e -- ingest_rejects_unknown_managed_key`
Expected: FAIL before Step 3 (today the key is absorbed → 200), PASS after Steps 1/3 land (Task 1 + the enriched wrap).

- [ ] **Step 7: Run the surface suites + check**

Run: `cargo nextest run -p temper-mcp` and `cargo nextest run -p temper-api --features test-db --test <ingest test target>`, then `cargo make check`.
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-api/src/handlers/ingest.rs \
        crates/temper-mcp/src/tools/resources.rs \
        tests/e2e/tests/managed_meta_reject_test.rs
git commit -m "fix(meta): reject unknown managed keys at MCP + API with open_meta hint

Closed-vocab framing propagates to both schemas via the shared ManagedMeta
doc-comment; API runtime error now points callers at open_meta. e2e proves
the ingest boundary rejects with a 400 + hint.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Discoverability + slug-precedence documentation sweep

Weave the closed-vocabulary framing and slug precedence into the surfaces callers actually read: CLI `--help`, MCP `describe_doc_type`, and the `temper` skill doc. Most of the vocabulary is *already* discoverable (CLI enum flags list values in prose; `describe_doc_type` returns the full schema + `enum_fields`), so this is a targeted legibility pass, not new machinery.

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:298-486` (Create + Update `--slug`/managed-flag help text)
- Modify: `crates/temper-mcp/src/tools/doc_types.rs` (a `describe_doc_type` assertion + optional framing note)
- Modify: `.claude/skills/temper/reference.md` (managed-vs-open + slug precedence section — locate the actual skill path; it is the `temper` skill's reference file)
- Test: `crates/temper-mcp/src/tools/doc_types.rs` test module

**Interfaces:**
- Consumes: `describe_doc_type_impl` (`doc_types.rs`), the closed `ManagedMeta` (Task 1).

- [ ] **Step 1: Write the failing `describe_doc_type` discoverability test**

In the `doc_types.rs` test module, assert that describing a task surfaces the managed vocabulary (so a caller can discover valid keys):

```rust
#[test]
fn describe_task_surfaces_managed_vocabulary() {
    let d = describe_doc_type_impl("task").expect("task is a known doc type");
    let props = d.schema.get("properties").and_then(|p| p.as_object()).unwrap();
    for key in ["temper-stage", "temper-mode", "temper-effort"] {
        assert!(props.contains_key(key), "managed key {key} must be discoverable");
    }
    // enum-valued managed keys expose their allowed values.
    assert!(
        d.enum_fields.get("temper-stage").is_some_and(|v| v.contains(&"backlog".to_string())),
        "temper-stage enum values must be discoverable"
    );
}
```

- [ ] **Step 2: Run to verify it passes (guards existing behavior)**

Run: `cargo nextest run -p temper-mcp describe_task_surfaces_managed_vocabulary`
Expected: PASS (this pins the existing discoverability so the sweep can't silently regress it). If it fails, `describe_doc_type_impl` regressed — fix before continuing.

- [ ] **Step 3: Add slug-precedence + managed-vs-open help to the CLI**

In `crates/temper-cli/src/cli.rs`, update the `--slug` help on both Create (`:298-350`) and Update, and the managed-field grouping comment. For `--slug`:

```rust
        /// URL-safe slug. Optional — derived from the title when omitted.
        /// This top-level flag is the only way to set the slug; a slug in
        /// managed frontmatter is inert (title-derived).
        #[arg(long)]
        slug: Option<String>,
```

Update the `// --- Task-specific fields ---` grouping comment (`cli.rs:445`) to name the tier:

```rust
        // --- Managed (temper-*) fields: a closed vocabulary; caller-defined
        //     tags/relationships are open-tier (see --tags/--relates-to) ---
```

- [ ] **Step 4: Add the managed-vs-open + slug section to the skill doc**

Locate the `temper` skill's `reference.md` (the SKILL.md router points at it; it is under the temper skill directory). Add a short section:

```markdown
## Managed vs open frontmatter

`managed_meta` is a **closed, temper-owned vocabulary** — the `temper-*`
workflow/provenance keys (stage, mode, effort, status, seq, branch, pr,
llm-model, llm-run, provenance). It is optional metadata with smart defaults;
you never *have* to send it. An unknown key under `managed_meta` is rejected —
put caller-defined ("bring-your-own") fields in `open_meta`, the free-form tier.

**Slug precedence:** the slug is derived from the title. To override it, pass
the top-level `slug` (CLI `--slug`, MCP `slug`). A slug placed in managed
frontmatter is inert.
```

- [ ] **Step 5: Run + check**

Run: `cargo nextest run -p temper-mcp` and `cargo build -p temper-cli --bin temper && temper resource create --help | head -40` (eyeball that `--slug` help reads correctly), then `cargo make check`.
Expected: tests PASS; `--help` shows the new slug guidance.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/cli.rs \
        crates/temper-mcp/src/tools/doc_types.rs \
        .claude/skills/temper/reference.md
git commit -m "docs(meta): document closed managed vocabulary + slug precedence across surfaces

CLI --help, describe_doc_type, and the temper skill now state that
managed_meta is a closed vocabulary (caller keys go to open_meta) and that
slug is title-derived / overridden only via the top-level slug.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage (Phase 1 items):**
- P1.1 reject unknown managed keys → Task 1 (type) + Task 2 (surface enforcement + errors). ✓
- P1.2 discoverability → Task 2 (schema descriptions via shared doc-comment) + Task 3 (CLI help, describe_doc_type). ✓
- P1.3 slug precedence documented → Task 3 (CLI, skill doc) + Task 2 (MCP field docs) + Task 1 (struct doc-comment). ✓
- Blast-radius surfaces (MCP descriptions, CLI shapes, utoipa, skill docs) → all touched. ✓
- **Deliberate deviation from spec Open Question #1:** the spec floated wrapping the *runtime* error at both MCP and API. Phase 1 wraps the **API** runtime error (cheap — the handler owns the `from_value`) and delivers the MCP-side "→ open_meta" guidance via the **schema description** (the doc-comment schemars renders), because the MCP input parse rejects at the rmcp framework boundary where the message is not ours to shape without a raw-`Value` field. Since **Phase 2 restructures these exact input structs**, the dedicated strict input type + custom MCP error belongs there, not here. Net Phase 1 behavior: callers on both surfaces are rejected; both surfaces carry the open_meta pointer (API at runtime + schema; MCP in-schema).

**Placeholder scan:** No TBD/TODO. Two spots defer to the live code deliberately and say exactly what to mirror: the e2e `TestContext` harness API (Task 2 Step 5) and any extra required `CreateResourceInput` fields (Task 2 Step 1) — both name the file to consult rather than leaving a blank.

**Type consistency:** `ManagedMeta` (no `extra`, `deny_unknown_fields`) is defined in Task 1 and consumed unchanged in Tasks 2–3. The error substring `"caller-defined keys belong in open_meta"` is produced in Task 2 Step 3 and asserted in Task 2 Step 5. `describe_doc_type_impl` signature used in Task 3 matches `doc_types.rs`.

## Out of scope (Phase 2 — do NOT do here)

- Removing `temper-title`/`temper-slug` (and `-type`/`-context`) from `ManagedMeta`.
- Promoting identity/home/type to required first-class wire fields; retiring `ensure_managed_identity_keys`.
- Single-sourcing the Property vocabulary (`ManagedMeta` ⟺ `MANAGED_PROPERTY_KEYS` ⟺ `key_fate`) with a drift-guard test.
- A dedicated strict `ManagedMetaInput` type with a custom MCP error message.
- The open-key kebab-vs-snake case inconsistency (`relates-to` vs `relates_to`) — a separate open_meta wart noted in the spec.
