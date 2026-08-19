# Rationalization-Comments Audit Sweep — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve the still-actionable findings from the 2026-05-08 rationalization-comments audit (A.2, A.3, B.2), refresh the audit task body, and close the deprecate-resource-service sub-task — all against ground truth post-Wave 1 Phase 3b/3c.

**Architecture:** Five commits across two crates and one vault refresh. Phase A fixes A.3 (env-var auth doc + expiry warning) and A.2 (graph cross-owner doc nudge). Phase B threads owner through `build_vault_path` and its two callers (B.2). Phase C refreshes the vault audit task body and closes the deprecate sub-task. No new tests gated on `test-db` or `test-embed`.

**Tech Stack:** Rust (temper-client, temper-cli, temper-api), `cargo nextest`, `chrono::DateTime<Utc>` for expiry math.

**B.2 cut-line:** Phase B is fully isolated. If Pete decides to defer B.2, run **Phase A + Phase C** in this session and skip Phase B; the audit-task-body refresh in Phase C will note B.2 as deferred and link to a follow-up task. Phase B can then be picked up as its own session by reading this plan and executing Tasks 5–6 only.

---

## Spec reference

`docs/superpowers/specs/2026-05-09-rationalization-comments-audit-sweep-design.md` (committed in `035e014`).

## Pre-flight (one-time)

- [ ] **Verify branch.** Should be on `jct/audit-rationalization-sweep`. Run `git branch --show-current`. Expected output: `jct/audit-rationalization-sweep`.
- [ ] **Verify clean tree.** Run `git status`. Expected: `working tree clean` aside from possible untracked files unrelated to this work.
- [ ] **Confirm baseline tests pass.** Run `cargo nextest run -p temper-client -p temper-cli` to confirm the baseline is green before any change. If not, stop and report.

---

## Phase A — A.3 doc rewrite, expiry helper, expiry warning, A.2 nudge

### Task 1: A.3 doc rewrite (no test — pure documentation)

**Files:**
- Modify: `crates/temper-client/src/auth.rs:553-569` (docstring on `pub fn stored_auth_from_env`)

- [ ] **Step 1: Replace the docstring.**

Replace lines 553-569 (the existing docstring on `pub fn stored_auth_from_env`) with:

```rust
/// If `TEMPER_TOKEN` is set, build an in-memory [`StoredAuth`] from it.
///
/// Returns `Ok(None)` when `TEMPER_TOKEN` is unset or empty — the caller
/// then falls back to disk-backed auth. Returns `Err(_)` when the env var
/// is set but malformed.
///
/// Provider defaults to `"auth0"` when `TEMPER_PROVIDER` is unset (matches
/// the out-of-box config default). Device id is taken from `TEMPER_DEVICE_ID`
/// when set; otherwise a fresh UUIDv7 is generated for this session — per
/// the cloud-mode design, the session is ephemeral and a fresh device id
/// is acceptable.
///
/// **Refresh-less by design.** The returned [`StoredAuth`] has
/// `refresh_token: None` because env-var auth deliberately does not carry
/// a refresh token. Per Unit B.4 §Q1/W1
/// (`docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md`),
/// exporting a refresh token to a cloud session would entangle it with the
/// user's local Auth0 grant under refresh-token rotation: the first side
/// to refresh invalidates the other's RT, and the next refresh triggers
/// reuse-detection that kills the entire grant family. The only safe
/// contract is access-token-only export with the Auth0-default 24h AT TTL
/// as the ceiling. Users re-export via `temper auth export-token` to renew.
///
/// **Where this fits.** This is the parsing primitive shared by
/// [`MemoryTokenStore::from_env`] (the user-facing cloud-mode API) and
/// [`DiskTokenStore::load`]'s env fallback. The cloud-mode bootstrap in
/// `temper-cli/src/actions/runtime.rs` emits a stderr warning when an
/// env-var token is within an hour of expiry; on actual expiry, callers
/// receive [`ClientError::TokenExpired`] and must re-export.
pub fn stored_auth_from_env() -> Result<Option<StoredAuth>> {
```

- [ ] **Step 2: Run check.**

```bash
cargo make check
```

Expected: clean. If clippy complains about the doc link `[`ClientError::TokenExpired`]`, replace with the bare path `ClientError::TokenExpired` (no brackets) — the rest of the file uses both shapes, so either is acceptable.

- [ ] **Step 3: Commit.**

```bash
git add crates/temper-client/src/auth.rs
git commit -m "$(cat <<'EOF'
doc(client): rewrite stored_auth_from_env docstring to reflect W1 contract

The previous docstring framed env-var auth as "intentionally refresh-less
in this pass" with a deferral pointer to "Unit B.4 research work, not B.1."
B.1 has long shipped and the B.4 spec exists; the W1 recommendation —
access-token-only export, no refresh-token export — is the architectural
answer, not a deferral. Refresh-less is the deliberate contract.

Also notes that this function is the parsing primitive shared by
MemoryTokenStore::from_env() and DiskTokenStore::load()'s env fallback,
and points at the cloud-mode bootstrap warning surface added in a
follow-up commit.

Resolves audit finding A.3 (doc portion).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: A.3 expiry helper in `temper-client`

**Files:**
- Modify: `crates/temper-client/src/auth.rs` (add `time_until_expiry` helper near `StoredAuth`-adjacent helpers)
- Modify: `crates/temper-client/src/auth.rs` tests module (add 2 unit tests)

- [ ] **Step 1: Write the failing tests.**

Add the following tests to the existing `#[cfg(test)] mod tests` block in `crates/temper-client/src/auth.rs` (the module already exists; append at the end before the closing brace). Use the existing `make_auth` helper at line 738 as a reference for `StoredAuth` construction.

```rust
#[test]
fn time_until_expiry_positive_for_future() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-05-09T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let auth = make_auth(
        chrono::DateTime::parse_from_rfc3339("2026-05-09T14:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
    );
    assert_eq!(time_until_expiry(&auth, now).num_minutes(), 150);
}

#[test]
fn time_until_expiry_negative_for_expired() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-05-09T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let auth = make_auth(
        chrono::DateTime::parse_from_rfc3339("2026-05-09T11:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
    );
    assert!(time_until_expiry(&auth, now).num_seconds() < 0);
}
```

- [ ] **Step 2: Run the tests — verify they fail with "function not defined."**

```bash
cargo nextest run -p temper-client -E 'test(time_until_expiry)'
```

Expected: compile error (`time_until_expiry` not in scope) or test failure. Either fails the assertion that the symbol exists.

- [ ] **Step 3: Implement the helper.**

Add this function to `crates/temper-client/src/auth.rs`. Insert it immediately after the existing `needs_refresh` helper (around line 634 — find the function that returns `bool` for "auth approaching expiry"; place the new helper directly below it for proximity to other expiry math):

```rust
/// Returns the duration until `stored`'s token expires, relative to `now`.
///
/// Negative when the token has already expired. Use the sign to distinguish
/// "expired" from "expiring soon" at call sites — see
/// `temper-cli/src/actions/runtime.rs` for the cloud-mode bootstrap warning
/// that consumes this.
pub fn time_until_expiry(
    stored: &StoredAuth,
    now: chrono::DateTime<chrono::Utc>,
) -> chrono::Duration {
    stored.expires_at - now
}
```

- [ ] **Step 4: Run the tests — verify they pass.**

```bash
cargo nextest run -p temper-client -E 'test(time_until_expiry)'
```

Expected: 2 tests pass.

- [ ] **Step 5: Run full crate suite.**

```bash
cargo nextest run -p temper-client
```

Expected: all pass. (Per `feedback_plan_regression_guard_after_filter_test`, follow filter-by-name with full suite before commit.)

- [ ] **Step 6: Run check.**

```bash
cargo make check
```

Expected: clean.

- [ ] **Step 7: Commit.**

```bash
git add crates/temper-client/src/auth.rs
git commit -m "$(cat <<'EOF'
feat(client): add time_until_expiry helper for env-var token UX

Pure helper returning the chrono::Duration between StoredAuth.expires_at
and a caller-supplied `now`. Negative when expired so call sites can
distinguish "expired" (error-shaped warning) from "expiring soon"
(advisory warning) without re-deriving the math.

Consumed by the cloud-mode bootstrap warning in temper-cli's runtime.rs
in the next commit.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: A.3 expiry warning at cloud-mode bootstrap

**Files:**
- Modify: `crates/temper-cli/src/actions/runtime.rs` (add `token_expiry_warning` + `humanize_duration` private helpers, wire warning into `resolve_token_store`'s `Cloud` branch, add 3 unit tests)

- [ ] **Step 1: Write the failing tests.**

Append the following test module to `crates/temper-cli/src/actions/runtime.rs` (or extend the existing `#[cfg(test)] mod tests` block — check the file for an existing tests block first; if absent, add a new one at the bottom of the file, before any closing braces):

```rust
#[cfg(test)]
mod expiry_warning_tests {
    use super::*;
    use chrono::{DateTime, Duration, Utc};
    use temper_client::auth::{Provider, StoredAuth};

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-09T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn auth_expiring_at(when: DateTime<Utc>) -> StoredAuth {
        StoredAuth {
            provider: Provider::auth0("temperkb.us.auth0.com"),
            access_token: "tok".to_string().into(),
            refresh_token: None,
            expires_at: when,
            profile_id: None,
            device_id: None,
        }
    }

    #[test]
    fn warns_when_token_within_threshold() {
        let now = fixed_now();
        let auth = auth_expiring_at(now + Duration::minutes(30));
        let msg = token_expiry_warning(&auth, now, Duration::hours(1))
            .expect("expected warning for 30-min-out token");
        assert!(msg.starts_with("warning:"), "got: {msg}");
        assert!(msg.contains("30m"), "got: {msg}");
        assert!(msg.contains("temper auth export-token"), "got: {msg}");
    }

    #[test]
    fn silent_when_token_healthy() {
        let now = fixed_now();
        let auth = auth_expiring_at(now + Duration::hours(2));
        assert!(token_expiry_warning(&auth, now, Duration::hours(1)).is_none());
    }

    #[test]
    fn errors_when_token_expired() {
        let now = fixed_now();
        let auth = auth_expiring_at(now - Duration::minutes(30));
        let msg = token_expiry_warning(&auth, now, Duration::hours(1))
            .expect("expected error-shaped warning for expired token");
        assert!(msg.starts_with("error:"), "got: {msg}");
        assert!(msg.contains("expired"), "got: {msg}");
    }
}
```

- [ ] **Step 2: Run the tests — verify they fail.**

```bash
cargo nextest run -p temper-cli -E 'test(expiry_warning_tests)'
```

Expected: compile error (`token_expiry_warning` not in scope).

- [ ] **Step 3: Implement the helpers.**

Add these private helpers to `crates/temper-cli/src/actions/runtime.rs`. Place them after `client_err_to_temper` (which ends around line 34) and before `resolve_token_store`:

```rust
/// Returns a stderr-shaped warning message when `stored`'s token is at or
/// past `threshold` of expiry, or already expired. Returns `None` for
/// healthy tokens.
///
/// Pure function; takes `now` explicitly so tests don't depend on
/// wall-clock time. Used by `resolve_token_store`'s `Cloud` branch.
fn token_expiry_warning(
    stored: &temper_client::auth::StoredAuth,
    now: chrono::DateTime<chrono::Utc>,
    threshold: chrono::Duration,
) -> Option<String> {
    let remaining = temper_client::auth::time_until_expiry(stored, now);
    if remaining < chrono::Duration::zero() {
        Some(format!(
            "error: TEMPER_TOKEN expired at {} (~{} ago). \
             Re-run `temper auth export-token` and re-set TEMPER_TOKEN to renew.",
            stored.expires_at.to_rfc3339(),
            humanize_duration(-remaining),
        ))
    } else if remaining <= threshold {
        Some(format!(
            "warning: TEMPER_TOKEN expires at {} (~{} from now). \
             Re-run `temper auth export-token` and re-set TEMPER_TOKEN to renew.",
            stored.expires_at.to_rfc3339(),
            humanize_duration(remaining),
        ))
    } else {
        None
    }
}

/// Render a non-negative `chrono::Duration` in `<N>h<M>m` or `<N>m` form.
/// Sub-minute durations clamp to `0m` — the warning is human-scale, not
/// stopwatch-precise.
fn humanize_duration(d: chrono::Duration) -> String {
    let total_mins = d.num_minutes().max(0);
    if total_mins >= 60 {
        format!("{}h{}m", total_mins / 60, total_mins % 60)
    } else {
        format!("{}m", total_mins)
    }
}
```

- [ ] **Step 4: Wire the warning into the Cloud branch.**

Replace lines 44-49 in `crates/temper-cli/src/actions/runtime.rs` (the `VaultState::Cloud` arm of `resolve_token_store`) with:

```rust
        VaultState::Cloud => {
            let mem = MemoryTokenStore::from_env_required()
                .map_err(|e| TemperError::Config(e.to_string()))?;
            // Cloud-mode AT is refresh-less by design (see
            // `stored_auth_from_env` docstring). Warn early when the token
            // is approaching expiry so users have time to re-export.
            if let Ok(Some(stored)) = mem.load() {
                if let Some(msg) = token_expiry_warning(
                    &stored,
                    chrono::Utc::now(),
                    chrono::Duration::hours(1),
                ) {
                    eprintln!("{msg}");
                }
            }
            Ok(Arc::new(mem))
        }
```

- [ ] **Step 5: Run the tests — verify they pass.**

```bash
cargo nextest run -p temper-cli -E 'test(expiry_warning_tests)'
```

Expected: 3 tests pass.

- [ ] **Step 6: Run full crate suite.**

```bash
cargo nextest run -p temper-cli
```

Expected: all pass.

- [ ] **Step 7: Run check.**

```bash
cargo make check
```

Expected: clean.

- [ ] **Step 8: Commit.**

```bash
git add crates/temper-cli/src/actions/runtime.rs
git commit -m "$(cat <<'EOF'
feat(cli): warn when env-var TEMPER_TOKEN approaches expiry

Cloud-mode auth is refresh-less by design (Auth0 RT-rotation security).
The 24h Auth0-default AT TTL is the contract, but mid-task surprise
expiry produces a confusing TokenExpired error without warning.

After MemoryTokenStore::from_env_required() builds the store, peek at
the stored auth and emit a stderr advisory when the AT is within 1h of
expiry, or an error-shaped advisory when already expired. Disk-backed
auth auto-refreshes via the RT and doesn't need this surface.

Threshold is constant (Duration::hours(1)). Tunable later if telemetry
shows it's wrong; 1h is "long enough that ignoring is plausible, short
enough to be actionable mid-task."

Resolves audit finding A.3 (runtime portion).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: A.2 graph cross-owner doc nudge

**Files:**
- Modify: `crates/temper-api/src/handlers/graph.rs:39-49`

- [ ] **Step 1: Replace the comment block.**

Find the comment block at `crates/temper-api/src/handlers/graph.rs:40-43` (the four-line block immediately before the `if query.owner != "@me"` check) and replace it with:

```rust
    // Resolve `owner` — v1 only supports "@me" (caller's own vault).
    // Multi-owner queries are a v1-scope boundary: expanding requires a
    // permission-model design that defines who can read whose graph and
    // how handles resolve to profile IDs. Treat handles other than "@me"
    // as an invalid query parameter and return 400 (rather than 404).
```

(The change drops the "Cross-owner querying is deferred; handles are left as a later migration" sentence and replaces the framing with a v1-scope-boundary statement that names the missing design dependency.)

- [ ] **Step 2: Run check.**

```bash
cargo make check
```

Expected: clean.

- [ ] **Step 3: Run the API crate tests.**

```bash
cargo nextest run -p temper-api
```

Expected: all pass (no behavior changed; this catches accidental breakage).

- [ ] **Step 4: Commit.**

```bash
git add crates/temper-api/src/handlers/graph.rs
git commit -m "$(cat <<'EOF'
doc(api): tighten graph subgraph cross-owner comment

The previous comment framed cross-owner support as "deferred; handles
are left as a later migration" with no tracking link — the exact soft-
fail rationalization shape the 2026-05-08 audit was hunting.

Replace with a v1-scope-boundary statement that names the missing design
dependency (permission model + handle resolution) so future readers see
this as a deliberate scope decision, not a TODO without a home.

Resolves audit finding A.2.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase B — B.2 owner threading

> **Cut-line:** Phase B is fully isolated from Phase A. If deferring B.2 to a separate session, skip directly to Phase C and note the deferral in Task 6 Step 1.

### Task 5: Thread `owner` through `build_vault_path`, `dedup_vault_slug`, `write_vault_file_and_register`

**Files (atomic commit — all changes in one commit because the type system links them):**
- Modify: `crates/temper-cli/src/actions/ingest.rs:391-419` (helpers + TODO removal)
- Modify: `crates/temper-cli/src/actions/ingest.rs:579-593` (write_vault_file_and_register signature + body)
- Modify: `crates/temper-cli/src/actions/ingest.rs:786-810` (existing tests + new test)
- Modify: `crates/temper-cli/src/actions/sync.rs:3271` (test caller)
- Modify: `crates/temper-cli/src/commands/add.rs` — 12 caller sites (lines 148, 154, 259, 268, 356, 358, 421, 423, 719, 725, 910, 918)

- [ ] **Step 1: Add the failing test for non-@me owner.**

Add this test to `crates/temper-cli/src/actions/ingest.rs` inside the existing `#[cfg(test)] mod tests` block (it's the block containing `build_vault_path_produces_correct_path` at line 792):

```rust
    #[test]
    fn build_vault_path_threads_non_me_owner() {
        let root = std::path::Path::new("/vault");
        let path = build_vault_path(root, "@petetaylor", "work", "note", "my-document");
        assert_eq!(
            path,
            std::path::PathBuf::from("/vault/@petetaylor/work/note/my-document.md")
        );
    }
```

- [ ] **Step 2: Run the test — verify it fails (compile error).**

```bash
cargo nextest run -p temper-cli -E 'test(build_vault_path_threads_non_me_owner)'
```

Expected: compile error (`build_vault_path` takes 4 args, this passes 5).

- [ ] **Step 3: Update `build_vault_path` signature and body.**

Replace lines 391-401 in `crates/temper-cli/src/actions/ingest.rs` (the existing `build_vault_path` definition and its docstring) with:

```rust
/// Canonical vault path for a managed resource.
///
/// `{vault_root}/{owner}/{context}/{doc_type}/{slug}.md`
///
/// `owner` is the profile-handle component (e.g. `"@me"` or `"@alice"`).
/// Callers should resolve via `Config::owner_for_context(context)` when
/// they have a `Config` in scope; the literal `"@me"` is appropriate for
/// tests and contexts without a configured subscription.
pub fn build_vault_path(
    vault_root: &Path,
    owner: &str,
    context: &str,
    doc_type: &str,
    slug: &str,
) -> PathBuf {
    Vault::new(vault_root).doc_file(owner, context, doc_type, slug)
}
```

(The old `// TODO(owner-scoped): thread owner through when subscriptions sync lands.` comment is removed by this replacement.)

- [ ] **Step 4: Update `dedup_vault_slug` signature and body.**

Replace lines 405-419 in `crates/temper-cli/src/actions/ingest.rs` with:

```rust
/// De-duplicate a vault slug by appending `-2`, `-3`, etc. when the target
/// path already exists.
pub fn dedup_vault_slug(
    vault_root: &Path,
    owner: &str,
    context: &str,
    doc_type: &str,
    slug: &str,
) -> String {
    let base_path = build_vault_path(vault_root, owner, context, doc_type, slug);
    if !base_path.exists() {
        return slug.to_string();
    }
    for i in 2..1000 {
        let candidate = format!("{slug}-{i}");
        let path = build_vault_path(vault_root, owner, context, doc_type, &candidate);
        if !path.exists() {
            return candidate;
        }
    }
    // Extremely unlikely — fall back to UUID-suffixed slug.
    format!("{slug}-{}", Uuid::now_v7())
}
```

- [ ] **Step 5: Update `write_vault_file_and_register` signature and body.**

Replace lines 579-593 in `crates/temper-cli/src/actions/ingest.rs` with:

```rust
/// Write a vault file and register the resource in the manifest.
///
/// `slug` determines the vault filename (`{slug}.md`). Pass
/// `slug_from_title(&resource.title)` when no better slug is available.
/// `owner` is the profile-handle directory component — resolve via
/// `Config::owner_for_context(context)`.
///
/// Returns the absolute vault path.
#[expect(
    clippy::too_many_arguments,
    reason = "vault write needs owner, context, slug, resource, content, source, and extra fields; \
              candidate for VaultWritePlan params struct (see audit-followups task follow-ups)."
)]
pub fn write_vault_file_and_register(
    vault_root: &Path,
    owner: &str,
    context: &str,
    doc_type: &str,
    slug: &str,
    resource: &temper_core::types::ResourceRow,
    content: &str,
    ingestion_source: Option<&str>,
    extra_fields: Option<&[(&str, &str)]>,
) -> Result<PathBuf> {
    let vault_path = build_vault_path(vault_root, owner, context, doc_type, slug);
```

(Body below `let vault_path = …` stays unchanged.)

- [ ] **Step 6: Update existing in-file tests.**

Find the existing tests at lines 792 and 799 in `crates/temper-cli/src/actions/ingest.rs`. Update each call:

- Line 792 area (`build_vault_path_produces_correct_path`): replace `build_vault_path(root, "work", "note", "my-document")` with `build_vault_path(root, "@me", "work", "note", "my-document")`.
- Line 799 area (`build_vault_path_nested_context`): replace `build_vault_path(root, "personal", "resource", "research-paper")` with `build_vault_path(root, "@me", "personal", "resource", "research-paper")`.

If the asserted-against expected paths in either test hardcode the result (e.g. `"/vault/@me/work/note/my-document.md"`), they may already include `@me` from the inner `Vault::doc_file` call — keep them as-is; only the call-site argument changes.

- [ ] **Step 7: Update the `sync.rs` test caller.**

In `crates/temper-cli/src/actions/sync.rs:3271`, replace `ingest::dedup_vault_slug(vault, "temper", "task", "my-document")` with `ingest::dedup_vault_slug(vault, "@me", "temper", "task", "my-document")`.

- [ ] **Step 8: Update all 12 caller sites in `commands/add.rs`.**

For each pair below, the `dedup_vault_slug` call is followed within ~5 lines by a `write_vault_file_and_register` call. At each pair, before the `dedup_vault_slug` call (within the same enclosing function block), add or reuse a binding `let owner = config.owner_for_context(<the-context-var>);` — the local variable used for context is named `context`, `ctx`, or `resolved_context` depending on the site, so substitute accordingly.

Then update both calls to insert `&owner` as the second positional argument to `dedup_vault_slug` and the second positional argument to `write_vault_file_and_register`.

Caller pairs (line numbers from the current file; verify with `grep -n "dedup_vault_slug\|write_vault_file_and_register" crates/temper-cli/src/commands/add.rs`):

| dedup line | write line | context var name | notes |
|------------|------------|------------------|-------|
| 148 | 154 | `context` | |
| 259 | 268 | `context` | |
| 356 | 358 | `resolved_context` | |
| 421 | 423 | `context` | |
| 719 | 725 | `context` | inside a closure/match arm; check scope |
| 910 | 918 | `context` | |

For each row: insert `let owner = config.owner_for_context(<ctx-var>);` at the top of the smallest enclosing block (function, match arm, or loop body) that holds both calls, then thread `&owner` into both. If the smallest enclosing block already has a different `owner` binding (from existing `Config::owner_for_context` usage near line 277, etc.), reuse it instead of shadowing.

Verification after each pair:
```bash
cargo check -p temper-cli
```

If `Config` is not in scope at any site, surface as a blocker — do not stub `"@me"` to make it compile.

- [ ] **Step 9: Run the new test — verify it passes.**

```bash
cargo nextest run -p temper-cli -E 'test(build_vault_path_threads_non_me_owner)'
```

Expected: pass.

- [ ] **Step 10: Run full crate suite.**

```bash
cargo nextest run -p temper-cli
```

Expected: all pass. Per `feedback_plan_regression_guard_after_filter_test`, this catches caller-update mistakes.

- [ ] **Step 11: Run workspace test sweep.**

```bash
cargo nextest run --workspace
```

Expected: all pass.

- [ ] **Step 12: Run check.**

```bash
cargo make check
```

Expected: clean. If clippy flags the `#[expect(clippy::too_many_arguments)]` reason text or the new `owner` parameter shape, surface and triage rather than suppress.

- [ ] **Step 13: Commit.**

```bash
git add crates/temper-cli/src/actions/ingest.rs crates/temper-cli/src/actions/sync.rs crates/temper-cli/src/commands/add.rs
git commit -m "$(cat <<'EOF'
refactor(cli): thread owner through vault-path helpers

build_vault_path, dedup_vault_slug, and write_vault_file_and_register
now accept owner as their second parameter. Callers in commands/add.rs
resolve owner via Config::owner_for_context(context) at each site;
test callers pass "@me" literal.

Removes the // TODO(owner-scoped): ... comment at the old
build_vault_path definition. Subscriptions sync (Config::owner_for_context)
has been first-class for several PRs; the TODO is no longer load-bearing.

Note: write_vault_file_and_register's #[expect(too_many_arguments)] is
now reason-tagged as a candidate for a VaultWritePlan params-struct
refactor; tracked in the audit-followups task follow-ups, not done here.

Resolves audit finding B.2.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase C — Audit refresh + sub-task closure

### Task 6: Vault audit-task body refresh + close deprecate-resource-service

This task touches the vault, not the codebase. It runs as `temper resource update` invocations, not git commits.

- [ ] **Step 1: Refresh the audit-followups task body.**

Compose new task body content reflecting current state. Pipe it into `temper resource update` via stdin (per CLAUDE.md cloud-mode operations and the temper skill's body-stdin idiom):

```bash
cat <<'EOF' | temper resource update audit-followups--rationalization-comments-hiding-incomplete-implementations --type task --context temper
# audit followups: rationalization comments hiding incomplete implementations

## Why this exists

While fixing the `temper sync run` orphan-UUID-files bug (PR for `fix/sync-pull-snapshot-canonical-layout`, task `2026-05-07-fix-sync-pull-snapshot-branch-...`), we found that the bug originated from a code comment that called the broken behavior an "acceptable simplification." That comment was rationalizing a shortcut, the shortcut shipped through review, and it became a real cross-device data-placement defect months later.

We then ran a repo-wide grep for similar shapes — `acceptable tradeoff`, `for now`, `rare case`, `once X lands`, `future work will`, `intentionally`, `deferred`, `not handled yet`, etc. — to surface other places where deliberate-but-undocumented incomplete code might be lying in wait.

## Status (refreshed 2026-05-09)

The 2026-05-09 sweep (spec: `docs/superpowers/specs/2026-05-09-rationalization-comments-audit-sweep-design.md`) verified each finding against current code post-Wave 1 Phase 3b/3c and resolved the actionable ones.

### Findings — current state

- **A.1** (`crates/temper-api/src/backend/translators.rs:54-57`, dark-launch comment) — ✅ resolved by Phase 3b Task 10 (commit `f483453`). The translator now describes a forward-looking short-circuit for caller-supplied chunks, not a deferral.

- **A.2** (`crates/temper-api/src/handlers/graph.rs`, cross-owner deferral) — ✅ resolved 2026-05-09. Comment rewritten to a v1-scope-boundary statement naming the missing design dependency (permission model + handle resolution).

- **A.3** (`crates/temper-client/src/auth.rs`, env-var refresh-less auth) — ✅ resolved 2026-05-09 as **doc-not-architecture**. The B.4 spec (`docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md`) recommends W1 (access-token-only export); refresh-less is the deliberate contract, not a deferral. Doc rewrite + 1h-pre-expiry stderr warning at the cloud-mode bootstrap site. `stored_auth_from_env()` confirmed load-bearing as the parsing primitive shared by `MemoryTokenStore::from_env()` and `DiskTokenStore::load()`'s env fallback.

- **B.1** (`packages/temper-ui/src/lib/components/graph/KnowledgeGraph.svelte:46-49`, meta-doc mode stub) — ⏸ still UI-deferred per `feedback_ui_last`. Re-triage when UI work resumes.

- **B.2** (`crates/temper-cli/src/actions/ingest.rs:398-400`, `@me` hardcode TODO) — **<STATUS>**.

### Follow-ups surfaced

- `write_vault_file_and_register` has 9 args after B.2 (or 8 if B.2 not yet executed); candidate for `VaultWritePlan` params-struct refactor per CLAUDE.md "Params structs" rule. Not done in the 2026-05-09 sweep to keep B.2 mechanical.

- New rationalization-shape sweep across 3b/3c-introduced code (commits in PR #71) returned **zero new soft-fail comments**. The two `intentionally`-tagged comments at `handlers/resources.rs:230` and `tools/resources.rs:102` document deliberate API contract decisions (server-as-source-of-truth for body trio; open_meta is by-design free-form), not deferrals. The audit's pattern detection held up cleanly.

## Original audit-prompt scope (preserved for reference)

This task is **read-as-context** for any agent picking up adjacent work so they don't extend code paths that have known soft-fail rationalizations underneath. Each finding is triaged individually.

Full original report: `/tmp/temper-rationalization-audit.md` (regenerate via the audit prompt in this task's history).
EOF
```

**`<STATUS>` placeholder:** at execution time, replace with one of:
- `✅ resolved 2026-05-09` (signature change + caller updates + TODO removal) — if Phase B was executed.
- `⏸ deferred to follow-up task <slug>; size grew during 2026-05-09 sweep (~12 caller sites in commands/add.rs + write_vault_file_and_register signature)` — if Phase B was deferred. Create a follow-up task before this step using `temper resource create --type task --title "thread owner through build_vault_path (audit B.2)" --context temper --mode build --effort small`, then substitute the new slug.

- [ ] **Step 2: Close the deprecate-resource-service sub-task.**

```bash
temper resource update deprecate-resource-service--create-after-phase-3b --type task --context temper --stage done
```

Expected: confirmation that the task moved to `done`.

- [ ] **Step 3: Verify both vault changes via list.**

```bash
temper resource list --type task --context temper
```

Expected: `deprecate-resource-service--create-after-phase-3b` shows `done`; `audit-followups--...` body reflects the refreshed text (re-run `temper resource show audit-followups--...` to spot-check).

- [ ] **Step 4: No git commit — vault edits are tracked by the cloud manifest, not the project repo.**

---

## Final verification

After all phases complete (or after Phase A + Phase C if B.2 is deferred):

- [ ] **Workspace tests.**
  ```bash
  cargo nextest run --workspace
  ```
  Expected: all pass.

- [ ] **API crate with test-db.**
  ```bash
  cargo nextest run -p temper-api --features test-db
  ```
  Expected: all pass. (Per the embed-gated e2e recipe in CLAUDE.md, this confirms no integration regressions even though we didn't touch test-db code paths.)

- [ ] **`cargo make check`.** Expected: clean.

- [ ] **Branch state.** Run `git log --oneline jct/audit-rationalization-sweep ^main` — should list the spec commit (`035e014`) plus 4 commits if Phase A+C only, or 5 commits if Phase A+B+C.

---

## Self-review

Spec coverage check:
- Component 1 (A.3 doc + warning) → Tasks 1, 2, 3.
- Component 2 (A.2 nudge) → Task 4.
- Component 3 (B.2 owner threading) → Task 5.
- Component 4 (audit refresh + closure) → Task 6.
- Out-of-scope items (refresh-token wiring, cross-owner impl, B.1, VaultWritePlan refactor) → not in plan, captured as follow-ups in Task 6 Step 1's audit refresh body.

Type consistency check:
- `time_until_expiry(stored, now) -> chrono::Duration` — defined Task 2, consumed Task 3. ✅
- `token_expiry_warning(stored, now, threshold) -> Option<String>` — defined Task 3, only call site is `resolve_token_store`. ✅
- `build_vault_path(vault_root, owner, context, doc_type, slug)` — signature consistent across Tasks 5 Steps 3, 4, 5, 6, 7, 8. ✅

Placeholder scan: only one intentional placeholder (`<STATUS>` in Task 6 Step 1, with explicit substitution rules). No "TBD" / "implement later" / "similar to Task N" shapes.
