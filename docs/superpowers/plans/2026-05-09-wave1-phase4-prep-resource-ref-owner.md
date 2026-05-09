# Wave 1 Phase 4-prep — `ResourceRef::Scoped` gains `owner` field

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend `ResourceRef::Scoped` with an `owner: String` field so the canonical addressing (`kb://owner/context/doctype/<ident>`) is fully type-bound, and prepare for Phase 4a's `ManifestManager` API which addresses every record through `&ResourceRef`.

**Architecture:** Single change to a `temper-core::operations` enum variant rippled through every call site. Today every `ResourceRef::Scoped` destructure hardcodes `owner: "@me".to_string()` at the lookup boundary; after this PR the value flows from the ref. Constructor argument order matches `Vault::canonical_uri(owner, context, doc_type, ident)` for symmetry. No behavioral change — every constructor passes `"@me"` (preserving today's solo-mode convention), every destructure uses the ref's `owner` field. Future callers (teams) supply real owner handles without any further type changes.

**Tech Stack:** Rust 1.x, `serde`, `cargo-make`, `cargo-nextest`. Workspace crates touched: `temper-core`, `temper-api`. (`temper-mcp`, `temper-cli`, e2e tests use `ResourceRef::Uuid` exclusively today; not touched.)

---

## Spec reference

[`docs/superpowers/specs/2026-05-09-wave1-phase4-vault-backend-design.md`](../specs/2026-05-09-wave1-phase4-vault-backend-design.md), section "Phase 4-prep — ResourceRef::Scoped gains owner field".

## Scope

**In scope:**
- `ResourceRef::Scoped` gains `owner: String` field.
- `ResourceRef::scoped` constructor signature updated to `(owner, context, doctype, slug)` matching `Vault::canonical_uri` order.
- All call sites — production code and tests — pass `"@me"` for the owner argument so behavior is identity-preserving.
- All destructures replace hardcoded `owner: "@me".to_string()` with the ref's `owner` field.
- New unit test asserting non-`@me` owner flows through to `ResolveByUriParams.owner` (forward-compat coverage).

**Out of scope:**
- `temper-cli` does not currently use `ResourceRef`; touched only if compilation requires.
- `ManifestManager` work (lands in Phase 4a).
- VaultBackend / RemoteBackend impls (Phase 4a).
- Real team-owner usage (still implicit `@me`).

## File map

| File | Change |
|---|---|
| `crates/temper-core/src/operations/resource_ref.rs` | Add `owner: String` to `Scoped` variant; update `scoped(...)` constructor; update unit tests; add new `+team-acme` round-trip test. |
| `crates/temper-core/src/operations/commands.rs` | Update two test fixtures (lines ~105, ~115) to pass owner. |
| `crates/temper-core/src/operations/actions.rs` | Update destructure in `validate_update` (line ~256) to match the new variant shape; update one test fixture (line ~602). |
| `crates/temper-api/src/backend/db_backend.rs` | Update `show_resource` Scoped destructure (line ~79) — replace `owner: "@me".to_string()` with the ref's `owner`. |
| `crates/temper-api/src/backend/translators.rs` | Update `resolve_to_id` Scoped destructure (line ~179) — same pattern. |
| `crates/temper-api/src/backend/tests.rs` | Update test fixture (line ~152) to pass owner; add new test asserting non-`@me` owner round-trips through resolve. |

## Verification suites

The "regression target" from the spec — every PR runs all four:
```bash
cargo make check
cargo nextest run --workspace
cargo nextest run -p temper-api --features test-db
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed
```

## Commit strategy

This is a type-extension refactor. Touching the type breaks compilation in every consumer until each is updated, so the workspace must be green at every commit boundary. Strategy: **one atomic commit** containing the type change + every caller migration + the new tests. The pre-commit hook (clippy + format + biome) gates this commit and validates the whole-workspace state. Verification (Task 2) runs additional test suites; if any reveal a fix, the fix lands as a separate small commit.

---

## Task 1: Atomic `ResourceRef::Scoped` extension and caller migration

**Files:**
- Modify: `crates/temper-core/src/operations/resource_ref.rs`
- Modify: `crates/temper-core/src/operations/actions.rs`
- Modify: `crates/temper-core/src/operations/commands.rs`
- Modify: `crates/temper-api/src/backend/db_backend.rs`
- Modify: `crates/temper-api/src/backend/translators.rs`
- Modify: `crates/temper-api/src/backend/tests.rs`

The single commit at the end of this task is the **only** code commit in the PR. Every step before the commit edits files; the workspace is intentionally non-compiling between Step 1 and Step 6, so do **not** run `cargo check` until Step 7. Run it at the end as the gate.

### Step 1: Update the `Scoped` variant and constructor

**File:** `crates/temper-core/src/operations/resource_ref.rs`

Replace the existing enum and constructor block. The full new content of the production type code (everything before `#[cfg(test)] mod tests`) is:

```rust
//! ResourceRef — identifier for resource-action commands.
//!
//! Slug uniqueness is scoped to (owner, context, doctype); UUID is globally
//! unique. Every resource-action command (`Show`, `Update`, `Delete`, sync
//! variants) accepts either form. The enum shape (rather than two `Option`
//! fields) makes "exactly one form populated" a compile-time guarantee.

use serde::{Deserialize, Serialize};

use crate::types::ids::ResourceId;

/// Identifies a resource for a command that targets an existing resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ResourceRef {
    /// Globally-unique reference. Resolves directly without scoping fields.
    Uuid {
        #[serde(rename = "resource_id")]
        id: ResourceId,
    },
    /// Owner-qualified slug-based reference. Maps to the canonical
    /// `kb://<owner>/<context>/<doctype>/<slug>` URI form.
    Scoped {
        owner: String,
        context: String,
        doctype: String,
        slug: String,
    },
}

impl ResourceRef {
    /// Construct a UUID-based reference.
    pub fn uuid(id: ResourceId) -> Self {
        Self::Uuid { id }
    }

    /// Construct an owner-scoped reference. Argument order matches
    /// `Vault::canonical_uri(owner, context, doc_type, ident)`.
    pub fn scoped(
        owner: impl Into<String>,
        context: impl Into<String>,
        doctype: impl Into<String>,
        slug: impl Into<String>,
    ) -> Self {
        Self::Scoped {
            owner: owner.into(),
            context: context.into(),
            doctype: doctype.into(),
            slug: slug.into(),
        }
    }
}
```

### Step 2: Update existing tests in `resource_ref.rs` and add the new team-owner test

**File:** `crates/temper-core/src/operations/resource_ref.rs` (the `mod tests` block)

Replace the entire `#[cfg(test)] mod tests { ... }` block with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn scoped_constructor_sets_fields() {
        let r = ResourceRef::scoped("@me", "temper", "task", "hello-world");
        match r {
            ResourceRef::Scoped {
                owner,
                context,
                doctype,
                slug,
            } => {
                assert_eq!(owner, "@me");
                assert_eq!(context, "temper");
                assert_eq!(doctype, "task");
                assert_eq!(slug, "hello-world");
            }
            ResourceRef::Uuid { .. } => panic!("expected Scoped variant"),
        }
    }

    #[test]
    fn uuid_constructor_sets_id() {
        let id = ResourceId(Uuid::nil());
        let r = ResourceRef::uuid(id);
        match r {
            ResourceRef::Uuid { id: got } => assert_eq!(got, id),
            ResourceRef::Scoped { .. } => panic!("expected Uuid variant"),
        }
    }

    #[test]
    fn scoped_round_trips_via_serde() {
        let r = ResourceRef::scoped("@me", "temper", "task", "foo");
        let s = serde_json::to_string(&r).unwrap();
        let back: ResourceRef = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn uuid_round_trips_via_serde() {
        let r = ResourceRef::uuid(ResourceId(Uuid::nil()));
        let s = serde_json::to_string(&r).unwrap();
        let back: ResourceRef = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn scoped_carries_team_owner() {
        let r = ResourceRef::scoped("+team-acme", "engineering", "doc", "design-spec");
        match &r {
            ResourceRef::Scoped {
                owner,
                context,
                doctype,
                slug,
            } => {
                assert_eq!(owner, "+team-acme");
                assert_eq!(context, "engineering");
                assert_eq!(doctype, "doc");
                assert_eq!(slug, "design-spec");
            }
            ResourceRef::Uuid { .. } => panic!("expected Scoped variant"),
        }

        // serde wire form must include owner
        let s = serde_json::to_string(&r).unwrap();
        assert!(
            s.contains("\"owner\":\"+team-acme\""),
            "serde body did not include owner: {s}"
        );
    }
}
```

### Step 3: Update `validate_update` and its test in `operations/actions.rs`

**File:** `crates/temper-core/src/operations/actions.rs`

Find the `validate_update` function (around line 250) and replace its body's match arm for `Scoped`. The Scoped arm doesn't *use* owner for validation — slug/doctype/context-emptiness checks are unchanged. Use `..` so the destructure stays exhaustive without binding owner:

```rust
pub fn validate_update(cmd: &UpdateResource) -> Result<(), ActionError> {
    match &cmd.resource {
        ResourceRef::Uuid { .. } => Ok(()),
        ResourceRef::Scoped {
            slug,
            doctype,
            context,
            ..
        } => {
            validate_slug(slug)?;
            validate_doctype(doctype)?;
            if context.is_empty() {
                return Err(ActionError::MissingRequiredField("context".to_string()));
            }
            Ok(())
        }
    }
}
```

Find the `validate_update_validates_scoped_ref` test (around line 600). Replace the `ResourceRef::scoped(...)` line:

```rust
#[test]
fn validate_update_validates_scoped_ref() {
    let cmd = UpdateResource {
        resource: ResourceRef::scoped("@me", "temper", "task", "INVALID"),
        body: None,
        managed_meta: None,
        open_meta: None,
        origin: super::super::Surface::CliCloud,
    };
    assert!(matches!(
        validate_update(&cmd),
        Err(ActionError::InvalidSlug(_))
    ));
}
```

(Argument order: owner, context, doctype, slug. The "INVALID" slug stays; the test asserts `InvalidSlug`.)

### Step 4: Update test fixtures in `operations/commands.rs`

**File:** `crates/temper-core/src/operations/commands.rs`

Find the two `ResourceRef::scoped(...)` constructor calls inside `mod tests` (around lines 105 and 115). Update each to the new arg order:

- The call at line ~105 (currently `ResourceRef::scoped("hello", "task", "temper")`) becomes:
  ```rust
  resource: ResourceRef::scoped("@me", "temper", "task", "hello"),
  ```
- The call at line ~115 (currently `ResourceRef::scoped("x", "task", "temper")`) becomes:
  ```rust
  resource: ResourceRef::scoped("@me", "temper", "task", "x"),
  ```

If the surrounding test code matches against `ResourceRef::Scoped { .. }` it still works (the `..` covers the new field). No other changes in this file.

### Step 5: Update `db_backend.rs::show_resource` Scoped match arm

**File:** `crates/temper-api/src/backend/db_backend.rs`

Find the `ResourceRef::Scoped` match arm in `show_resource` (around line 79). Replace:

```rust
ResourceRef::Scoped {
    slug,
    doctype,
    context,
} => {
    let params = resource_service::ResolveByUriParams {
        owner: "@me".to_string(),
        context,
        doc_type: doctype,
        ident: slug,
    };
    resource_service::resolve_by_uri(self.pool(), *self.profile_id(), &params)
        .await
        .map_err(TemperError::from)?
}
```

with:

```rust
ResourceRef::Scoped {
    owner,
    context,
    doctype,
    slug,
} => {
    let params = resource_service::ResolveByUriParams {
        owner,
        context,
        doc_type: doctype,
        ident: slug,
    };
    resource_service::resolve_by_uri(self.pool(), *self.profile_id(), &params)
        .await
        .map_err(TemperError::from)?
}
```

(The hardcoded `owner: "@me".to_string(),` line is gone. Field order on the destructure now mirrors URI order.)

### Step 6: Update `translators.rs::resolve_to_id` Scoped match arm + `tests.rs` fixture

**File:** `crates/temper-api/src/backend/translators.rs`

Find the matching `ResourceRef::Scoped` block in `resolve_to_id` (around line 179). Replace:

```rust
ResourceRef::Scoped {
    slug,
    doctype,
    context,
} => {
    let params = resource_service::ResolveByUriParams {
        owner: "@me".to_string(),
        context,
        doc_type: doctype,
        ident: slug,
    };
    let row = resource_service::resolve_by_uri(pool, *profile_id, &params)
        .await
        .map_err(TemperError::from)?;
    Ok(row.id)
}
```

with:

```rust
ResourceRef::Scoped {
    owner,
    context,
    doctype,
    slug,
} => {
    let params = resource_service::ResolveByUriParams {
        owner,
        context,
        doc_type: doctype,
        ident: slug,
    };
    let row = resource_service::resolve_by_uri(pool, *profile_id, &params)
        .await
        .map_err(TemperError::from)?;
    Ok(row.id)
}
```

**File:** `crates/temper-api/src/backend/tests.rs`

Find the `ResourceRef::scoped(...)` call near line 152. Update:

```rust
resource: ResourceRef::scoped("@me", TEMPER_CONTEXT_NAME, "task", "show-by-slug"),
```

(`TEMPER_CONTEXT_NAME` is the existing constant in this file. Argument order: owner, context, doctype, slug.)

### Step 7: Add a new test asserting non-`@me` owner reaches the resolver

**File:** `crates/temper-api/src/backend/tests.rs`

Append to the existing `mod tests` block. **Before writing this test, re-read the existing tests in the file** to understand what helpers (`test_pool()`, profile-seeding, `Surface::HttpApi` construction etc.) are already there and use the same patterns. The intent is: build a Scoped ref with a `+team-acme` owner, dispatch through `show_resource`, assert that `resolve_by_uri` was called with `owner = "+team-acme"`.

Two acceptable shapes for this test depending on what's available in the file:

**Shape A — error-path assertion (simpler, no DB seeding required):**
Build a backend, dispatch `show_resource` with the team-owner Scoped ref, expect a `NotFound` error (because no row was seeded under that owner). The fact that the call returns `NotFound` instead of erroring at lookup-construction time proves the team owner reached `ResolveByUriParams`.

```rust
#[tokio::test]
#[cfg(feature = "test-db")]
async fn show_resource_threads_team_owner_into_resolve_params() {
    let pool = /* use whatever helper builds the test pool — e.g. test_pool().await */;
    let profile_id = /* use whatever helper seeds a profile and returns its id */;

    let backend = DbBackend::new(
        pool.clone(),
        profile_id,
        "test-device".to_string(),
        Surface::HttpApi,
    );
    let cmd = ShowResource {
        resource: ResourceRef::scoped("+team-acme", "engineering", "doc", "design-spec"),
        origin: Surface::HttpApi,
    };

    let err = backend.show_resource(cmd).await.unwrap_err();
    match err {
        TemperError::NotFound { .. } => { /* expected */ }
        other => panic!("expected NotFound for unseeded team resource; got {other:?}"),
    }
}
```

**Shape B — seeded happy-path assertion (stronger, more setup):**
Seed a row with `owner = "+team-acme"` via direct sqlx, then assert `show_resource` returns it. Pick this shape only if the file already has a precedent for direct row-seeding in tests; otherwise Shape A is sufficient.

Use Shape A unless the existing fixture file makes Shape B obvious and one-line.

### Step 8: Run all tests at the new green state

Run the workspace + temper-api tests to confirm nothing is broken before committing:

```bash
cargo make check
cargo nextest run --workspace
cargo nextest run -p temper-api --features test-db
```

Expected:
- `cargo make check` → all green (format, clippy, docs, biome).
- Workspace tests → all pass; the 5 tests in `resource_ref.rs` and the new test in `backend/tests.rs` are present.
- `temper-api` with `test-db` → all pass including the new `show_resource_threads_team_owner_into_resolve_params` test.

If anything fails, re-read the relevant edit. Common failure modes:
- A `ResourceRef::scoped(...)` call site missed in Steps 4 or 6 — `cargo check` will name the file and line.
- A destructure that didn't add `..` or didn't pull in `owner` — clippy or rustc will flag it.

### Step 9: Commit

```bash
git add crates/temper-core/src/operations/resource_ref.rs \
        crates/temper-core/src/operations/actions.rs \
        crates/temper-core/src/operations/commands.rs \
        crates/temper-api/src/backend/db_backend.rs \
        crates/temper-api/src/backend/translators.rs \
        crates/temper-api/src/backend/tests.rs

git commit -m "$(cat <<'EOF'
feat(operations): add owner field to ResourceRef::Scoped

Constructor signature is scoped(owner, context, doctype, slug),
matching Vault::canonical_uri argument order. DbBackend::show_resource
and translators::resolve_to_id previously hardcoded owner: "@me" at
the resolve boundary; now the value flows from the ResourceRef itself.

Identity-preserving: every existing call site passes "@me", so today's
solo-mode behavior is unchanged. Adds a +team-acme test asserting the
team-owner case routes through ResolveByUriParams correctly. Strict
prerequisite for Phase 4a's ManifestManager API which addresses every
record through &ResourceRef.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Workspace + e2e regression sweep, then push

**Files:** none (verification + push only).

### Step 1: Run the e2e suite (`test-db`)

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db`
Expected: all e2e `test-db` tests pass. No new failures.

### Step 2: Run the e2e suite (`test-db,test-embed`)

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`
Expected: all e2e `test-db,test-embed` tests pass. This is the embed-gated tier that catches workspace-feature-unification surprises (per the lesson from session `wave-1-phase-3c-mcp-migration-complete-3b-regressions-fixed-ready-for-pr`).

### Step 3: Verify diff size

Run: `git diff main --stat`

Expected: a small, focused diff. Approximate sizes:
- `crates/temper-core/src/operations/resource_ref.rs`: ~+30 / -10 lines (type change + 5 tests)
- `crates/temper-core/src/operations/actions.rs`: ~+1 / -1 lines (the `..` plus reordered test)
- `crates/temper-core/src/operations/commands.rs`: ~+0 / -0 lines net (just arg-order swaps)
- `crates/temper-api/src/backend/db_backend.rs`: ~-1 line (drop hardcoded owner)
- `crates/temper-api/src/backend/translators.rs`: ~-1 line (drop hardcoded owner)
- `crates/temper-api/src/backend/tests.rs`: ~+30 / -1 lines (new test + arg reorder)

If anything is significantly larger, something accidental crept in. Investigate before pushing.

### Step 4: Fix-up commit if needed

If Step 1 or Step 2 surfaced a failure, fix it and commit separately with a clear message:

```bash
git add <fixed-files>
git commit -m "$(cat <<'EOF'
fix(<crate>): <one-line reason>

<context — why the fix was needed and what it does>

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

If Steps 1 and 2 passed cleanly, no commit needed in this task.

### Step 5: Push the branch

Run: `git push -u origin jct/wave1-phase4a-vault-backend-foundation`

The branch already contains the two spec commits (`67772f6`, `e6db02d`) plus the Task 1 commit from this plan. After this push it's ready for PR.

### Step 6: Open the PR

Run:

```bash
gh pr create --title "Wave 1 Phase 4-prep: ResourceRef::Scoped gains owner field" --body "$(cat <<'EOF'
## Summary
- Add `owner: String` to `ResourceRef::Scoped`; constructor signature `scoped(owner, context, doctype, slug)` matches `Vault::canonical_uri` order.
- Replace hardcoded `owner: "@me".to_string()` in `DbBackend::show_resource` and `translators::resolve_to_id` with the ref's `owner`.
- Identity-preserving: every caller passes `"@me"` so today's solo-mode behavior is unchanged. Adds a `+team-acme` test that proves the team-owner case routes correctly.

This is Phase 4-prep per spec `docs/superpowers/specs/2026-05-09-wave1-phase4-vault-backend-design.md`. Strict prerequisite for Phase 4a's `ManifestManager` API.

## Test plan
- [x] `cargo make check`
- [x] `cargo nextest run --workspace`
- [x] `cargo nextest run -p temper-api --features test-db`
- [x] `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db`
- [x] `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Return the PR URL when done.

---

## Self-review

**Spec coverage:** The spec's Phase 4-prep section requires:
- [x] `ResourceRef::Scoped` carries `owner: String` — Task 1 Step 1
- [x] Every existing call site passes the caller's owner handle — Task 1 Steps 3–6 (each site passes `"@me"`, preserving today's behavior)
- [x] Workspace + four test suites green — Task 1 Step 8 (workspace + temper-api) + Task 2 Steps 1–2 (e2e suites)
- [x] No behavioral change — guaranteed by the identity-substitution pattern; verified by running the existing test suite which has not been adjusted in any way that changes behavior

**Type consistency:** `ResourceRef::scoped(owner, context, doctype, slug)` arg order matches `Vault::canonical_uri(owner, context, doc_type, ident)`. The `Scoped` variant fields appear in the same URI order in the type definition. Every destructure uses field-name shorthand, so reordering fields in the variant definition is invisible at call sites.

**Placeholder scan:** No TBDs, TODOs, "implement later" markers, or vague handwave language. Every step has exact file paths, exact code (or exact constructor arg order), and exact commands.

**Single-file ambiguity acknowledgment:** Task 1 Step 7's new test references helpers (`test_pool()`, profile-seeding) whose exact names depend on what `crates/temper-api/src/backend/tests.rs` already provides. The step explicitly instructs to read the existing tests first and reuse those helpers, with a Shape A vs Shape B fallback so the implementer always has a workable approach.

**Commit boundary check:** Only one code commit (Task 1 Step 9). The pre-commit hook runs clippy on the entire workspace; intermediate commits would fail the hook because the workspace is non-compiling between Step 1 and Step 6. Verifying this is intentional: the type extension cannot be split across commits without breaking the green-tree invariant.
