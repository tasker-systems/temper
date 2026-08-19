# Cloud-Only Vault — Chunk 2: Staleness Check + Per-Resource Refresh — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the two consumer-facing halves of the local projection — `temper resource show` materializes the canonical projection file (per-resource refresh), and a non-blocking staleness pre-flight warns when a pulled context has fallen behind the server. Purely additive: removes nothing, dark-launched alongside the existing local-vault machinery.

**Architecture:** The Chunk 1 `projection` module gains a pure write-from-parts helper (`write_resource_file_from_parts`) and a staleness API (`StalenessOutcome` + `evaluate_staleness` + `check_context_staleness` + `warn_if_context_stale`). Cloud-mode `temper resource show` — which already fetches both the resource row and its content — calls the write-from-parts helper as a best-effort tail action. `temper resource list` and `temper search` call the staleness pre-flight before their cloud query; the check reads the per-context cursor sidecar, and only when one exists does it resolve the context id and compare against the server's latest event id.

**Tech Stack:** Rust, `temper-client` (HTTP client), `cargo-nextest` (tests), `tempfile` (test fixtures). The e2e harness spawns a real Axum server + Postgres via `#[sqlx::test]`.

**Source spec:** `docs/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md` (Chunk 2).

**Predecessor:** Chunk 1 (`docs/superpowers/plans/2026-05-21-cloud-only-vault-chunk1-projection-foundation.md`) — landed. It built `projection.rs` (`ProjectionCursor`, `read_cursor`, `write_cursor`, `prune_context`, `write_resource_file`, `pull_context`, `PullSummary`), the `GET /api/events/cursor` endpoint, `EventClient::latest_for_context`, and repurposed `temper pull`.

---

## Conventions for the implementer

These are repository rules (`tasker-systems/temper/CLAUDE.md`). Follow them in every task:

- **Typed structs over inline JSON.** Never `serde_json::json!()` for data with a known shape — define a struct.
- **Shared wire types live in `temper-core`.** A type sent between the API and a client is defined once in `temper-core` and imported by both sides.
- **Service layer owns SQL.** All `sqlx::query!()` lives in `temper-api/src/services/`. (No SQL in this chunk.)
- **Functions decomposed / single-responsibility.** The staleness *decision* (`evaluate_staleness`) is a pure function separate from the network fetch (`check_context_staleness`) and the output (`warn_if_context_stale`).
- **All public types implement `Debug`.**
- **`#[expect(lint, reason = "...")]`** instead of `#[allow]` if a lint must be suppressed.
- **Before writing anything, read the file you're modifying AND a sibling.** Match the style you find — naming, imports, error handling.
- Commit messages end with the `Co-Authored-By` trailer used elsewhere in this repo.

**Environment:** Docker Postgres must be running (`cargo make docker-up`). `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`.

**Branch:** all work lands on `jct/cloud-only-vault-deprecation-spec` (Chunk 1 is already committed there) — do not create a new branch.

**Additivity constraint:** Chunk 2 must remain strictly additive. The `VaultState::Local` branch of `show_generic` is untouched. The `show_cache` three-tier ladder is untouched. No `VaultState`/`VaultBackend`/sync-engine code is removed. The mode flip and deletions are Chunks 3–8.

---

## File Structure

**Modified:**
- `crates/temper-cli/src/projection.rs` — gains `write_resource_file_from_parts` (extracted pure-write half of `write_resource_file`), `StalenessOutcome`, `evaluate_staleness`, `resolve_context_id`, `check_context_staleness`, `warn_if_context_stale`. New unit tests inline under `#[cfg(test)]`.
- `crates/temper-cli/src/commands/resource.rs` — `show_generic`'s `VaultState::Cloud` branch writes the projection file; `list` calls the staleness pre-flight.
- `crates/temper-cli/src/commands/search_cmd.rs` — `search` calls the staleness pre-flight.
- `tests/e2e/tests/projection_pull_test.rs` — new e2e tests appended (created in Chunk 1).

**Created:** none — Chunk 2 extends existing files.

---

## Task 1: Extract `write_resource_file_from_parts` — the pure-write half

**Files:**
- Modify: `crates/temper-cli/src/projection.rs`
- Test: `tests/e2e/tests/projection_pull_test.rs`

`write_resource_file` (Chunk 1) does two things: fetch a resource's `content` from the API, then assemble + write the markdown file. `temper resource show` already holds both a `ResourceRow` and a `ContentResponse` in hand (its cloud branch fetches both), so it needs only the *write* half — no second fetch. Split the pure-write half into `write_resource_file_from_parts`; `write_resource_file` becomes the fetch plus a call to it. The signature of `write_resource_file` is unchanged, so `pull_context` and the Chunk 1 tests are unaffected.

- [ ] **Step 1: Write the failing e2e test**

Add to `tests/e2e/tests/projection_pull_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn write_resource_file_from_parts_materializes_a_document(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client.contexts().create("fpctx").await.expect("ctx");
    seed_resource(&app, "fpctx", "research", "Parts Doc").await;

    let listed = app
        .client
        .resources()
        .list(&temper_core::types::resource::ResourceListParams {
            context_name: Some("fpctx".to_string()),
            ..Default::default()
        })
        .await
        .expect("list");
    let row = listed.rows.first().expect("one row");
    let content = app
        .client
        .resources()
        .content(uuid::Uuid::from(row.id))
        .await
        .expect("content");

    let vault_root = app.vault_dir.path();
    let path = temper_cli::projection::write_resource_file_from_parts(vault_root, row, &content)
        .expect("write_resource_file_from_parts");

    let expected = vault_root
        .join("@me")
        .join("fpctx")
        .join("research")
        .join("parts-doc.md");
    assert_eq!(path, expected);
    assert!(path.exists(), "file written at canonical path");

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.starts_with("---\n"), "has frontmatter fence");
    assert!(on_disk.contains("temper-id:"), "has identity frontmatter");
    assert!(on_disk.contains("Body text for Parts Doc"), "has body");
}
```

> `uuid::Uuid::from(row.id)` — `row.id` is a `ResourceId` newtype; Chunk 1's `write_resource_file` already converts it this way. If the conversion form differs, match Chunk 1's existing code in `projection.rs`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db write_resource_file_from_parts_materializes`
Expected: FAIL — `no function write_resource_file_from_parts in projection`.

- [ ] **Step 3: Refactor `write_resource_file`**

In `crates/temper-cli/src/projection.rs`, the current `write_resource_file` is one async function that fetches content then assembles and writes. Replace it with the two functions below. Add `use temper_core::types::resource::ContentResponse;` to the top-of-file imports (if `ContentResponse` is also re-exported at `temper_core::types::ContentResponse`, match whichever path Chunk 1 used for `ResourceRow`).

```rust
/// Assemble and write a resource's projection file from an already-fetched
/// row and content. The pure-write half of [`write_resource_file`] — it
/// makes no network call. `pull_context` reaches it via `write_resource_file`
/// (which fetches first); `temper resource show` calls it directly, because
/// its cloud branch already holds both the row and the content.
///
/// Frontmatter assembly reuses `actions::ingest::build_frontmatter_from_resource`
/// so projected files are byte-identical to sync-pulled ones. Returns the
/// absolute path written.
pub fn write_resource_file_from_parts(
    vault_root: &Path,
    row: &ResourceRow,
    content: &ContentResponse,
) -> Result<PathBuf> {
    use crate::actions::ingest;

    // `owner_handle` is literal "@me" for the requester's own resources and
    // "+team-slug" for team contexts — both are canonical vault directory
    // components. Empty handle defends against a sparse server row.
    let owner: &str = if row.owner_handle.is_empty() {
        "@me"
    } else {
        &row.owner_handle
    };
    let context = row.context_name.as_str();
    let doc_type = row.doc_type_name.as_str();

    let slug_owned;
    let slug: &str = match row.slug.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => {
            slug_owned = ingest::slug_from_title(&row.title);
            slug_owned.as_str()
        }
    };

    let managed_value = content
        .managed_meta
        .as_ref()
        .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null));

    let fm = ingest::build_frontmatter_from_resource(
        row,
        context,
        doc_type,
        owner,
        ingest::normalize_body_for_vault(&content.markdown),
        managed_value.as_ref(),
        content.open_meta.as_ref(),
    )?;

    let path = Vault::new(vault_root).doc_file(owner, context, doc_type, slug);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    fm.write_to(&path)
        .map_err(|e| TemperError::Config(format!("projection write {}: {e}", path.display())))?;
    Ok(path)
}

/// Fetch a resource's content and write it as a complete markdown file at
/// its canonical vault path. Returns the absolute path written.
///
/// `row` is a resource summary already obtained from a `list` call; this
/// makes one further API call (`content`) for the body + frontmatter meta,
/// then delegates the assembly + write to [`write_resource_file_from_parts`].
pub async fn write_resource_file(
    client: &TemperClient,
    vault_root: &Path,
    row: &ResourceRow,
) -> Result<PathBuf> {
    let content = client
        .resources()
        .content(Uuid::from(row.id))
        .await
        .map_err(crate::commands::client_err)?;
    write_resource_file_from_parts(vault_root, row, &content)
}
```

> This is a pure extraction — the body of `write_resource_file_from_parts` is exactly the post-fetch half of Chunk 1's `write_resource_file`. Do not change behavior. The `use crate::actions::ingest;` line moves into `write_resource_file_from_parts` (where the `ingest::` calls now live).

- [ ] **Step 4: Run the new test and the Chunk 1 regression tests**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db projection_pull`
Expected: PASS — the new `write_resource_file_from_parts_materializes_a_document` plus every Chunk 1 test (`write_resource_file_materializes_a_document`, `pull_context_materializes_tree_and_writes_cursor`, `pull_prunes_resources_deleted_on_server`, `pull_is_idempotent`, `pull_empty_context_writes_cursor_with_no_event_id`, `events_cursor_returns_latest_event_for_context`). The refactor must not regress any of them.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/projection.rs tests/e2e/tests/projection_pull_test.rs
git commit -m "$(cat <<'EOF'
refactor(cli): extract write_resource_file_from_parts

Splits the pure assemble-and-write half out of write_resource_file so a
caller that already holds a ResourceRow + ContentResponse (temper resource
show) can write a projection file without a second content fetch.
write_resource_file keeps its signature; pull_context is unaffected.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Cloud-mode `temper resource show` writes the projection file

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs`

`show_generic`'s `VaultState::Cloud` branch (around `resource.rs:1058-1092`) already fetches a `ResourceRow` via `resolve_by_uri` and a `ContentResponse` via `content`. Add a best-effort tail action: write the resource to its canonical projection path via `write_resource_file_from_parts`. This is the per-resource projection refresh and the *read* step of the read-before-write discipline. The `VaultState::Local` branch is left exactly as-is.

The projection write is **best-effort**: `show`'s guaranteed job is to display the resource. If the projection write fails (disk full, permissions), `show` prints a warning to stderr and still displays the content — it does not error.

- [ ] **Step 1: Read the current cloud branch**

Read `show_generic` in `crates/temper-cli/src/commands/resource.rs` (the function starts near line 1041). The `VaultState::Cloud` arm's async closure currently ends:

```rust
                    let resp = client
                        .resources()
                        .content(*row.id.as_uuid())
                        .await
                        .map_err(crate::actions::runtime::client_err_to_temper)?;
                    Ok(resp.markdown)
```

`config_clone` (a `Config` clone) and `slug_inner` (the slug `String`) are already bound in the enclosing scope.

- [ ] **Step 2: Add the projection write**

Replace those final two lines of the `VaultState::Cloud` closure with:

```rust
                    let resp = client
                        .resources()
                        .content(*row.id.as_uuid())
                        .await
                        .map_err(crate::actions::runtime::client_err_to_temper)?;

                    // Per-resource projection refresh: write the fetched
                    // resource to its canonical projection path. Best-effort
                    // — a write failure must not stop `show` from displaying.
                    if let Err(e) = crate::projection::write_resource_file_from_parts(
                        &config_clone.vault_root,
                        &row,
                        &resp,
                    ) {
                        crate::output::warning(format!(
                            "could not refresh projection file for '{slug_inner}': {e}"
                        ));
                    }

                    Ok(resp.markdown)
```

> Verify field/method names against the real code: `config_clone.vault_root` (the `Config` projection root — Chunk 1's `pull_context` uses `config.vault_root`), `row` is the `ResourceRow` from `resolve_by_uri`, `resp` is the `ContentResponse`. `crate::output::warning` is the stderr warning helper. If `slug_inner` is named differently in the actual closure scope, use the actual slug binding.

- [ ] **Step 3: Verify it compiles**

Run: `SQLX_OFFLINE=true cargo build -p temper-cli`
Expected: clean build.

> This task has no dedicated automated test. The projection-write logic itself (`write_resource_file_from_parts`) is covered by Task 1's e2e test; the cloud-`show` change is a thin best-effort wiring of it (the same rationale Chunk 1 Task 9 used for the `pull` command rewrite). Correctness here is compilation plus the existing `show` test suite staying green — `show_cache_e2e_test` and `cloud_only_show_fallback_test` exercise the untouched `VaultState::Local` path and must still pass (Task 6).

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "$(cat <<'EOF'
feat(cli): cloud-mode `resource show` refreshes the projection file

show_generic's Cloud branch already fetches the resource row + content;
it now also writes them to the canonical projection path via
write_resource_file_from_parts. This is the per-resource projection
refresh and the read step of the read-before-write discipline. The write
is best-effort — a failure warns but does not stop show from displaying.
The local-mode branch is untouched.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `StalenessOutcome` + `evaluate_staleness` — the pure decision

**Files:**
- Modify: `crates/temper-cli/src/projection.rs`

The staleness check has three separable concerns: the *decision* (does a cursor match the server?), the *network fetch* (Task 4), and the *output* (Task 4). This task builds the decision as a pure, fully unit-tested function with no client and no IO.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `crates/temper-cli/src/projection.rs`:

```rust
    #[test]
    fn evaluate_staleness_equal_ids_is_fresh() {
        let cursor = ProjectionCursor {
            last_event_id: Some(Uuid::nil()),
            pulled_at: Utc::now(),
        };
        assert_eq!(
            evaluate_staleness(&cursor, Some(Uuid::nil())),
            StalenessOutcome::Fresh
        );
    }

    #[test]
    fn evaluate_staleness_differing_ids_is_stale() {
        let cursor = ProjectionCursor {
            last_event_id: Some(Uuid::nil()),
            pulled_at: Utc::now(),
        };
        assert_eq!(
            evaluate_staleness(&cursor, Some(Uuid::from_u128(1))),
            StalenessOutcome::Stale
        );
    }

    #[test]
    fn evaluate_staleness_both_none_is_fresh() {
        let cursor = ProjectionCursor {
            last_event_id: None,
            pulled_at: Utc::now(),
        };
        assert_eq!(evaluate_staleness(&cursor, None), StalenessOutcome::Fresh);
    }

    #[test]
    fn evaluate_staleness_server_advanced_from_none_is_stale() {
        let cursor = ProjectionCursor {
            last_event_id: None,
            pulled_at: Utc::now(),
        };
        assert_eq!(
            evaluate_staleness(&cursor, Some(Uuid::nil())),
            StalenessOutcome::Stale
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run -p temper-cli projection::tests::evaluate_staleness`
Expected: FAIL — `cannot find type StalenessOutcome` / `cannot find function evaluate_staleness`.

- [ ] **Step 3: Add `StalenessOutcome` and `evaluate_staleness`**

Add to `crates/temper-cli/src/projection.rs`, after the `ProjectionCursor` struct and its IO functions (before `write_resource_file_from_parts` is fine — place it wherever it reads well; keep cursor-related items together):

```rust
/// Outcome of a non-blocking staleness pre-flight for one context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StalenessOutcome {
    /// No cursor sidecar — the context was never pulled. The check made no
    /// network call; the caller stays silent.
    NotProjected,
    /// A cursor exists and matches the server's latest event. Silent.
    Fresh,
    /// A cursor exists but the server has advanced past it. The caller warns.
    Stale,
    /// The check could not complete — offline, or the context could not be
    /// resolved. Silent (a debug log is emitted at the failure site).
    Skipped,
}

/// Compare a context's cursor against the server's latest event id for that
/// context. Pure: the staleness *decision*, with no IO. The server's id is
/// recorded into the cursor at pull time, so any divergence means at least
/// one event landed since the last pull.
fn evaluate_staleness(cursor: &ProjectionCursor, server_latest: Option<Uuid>) -> StalenessOutcome {
    if server_latest == cursor.last_event_id {
        StalenessOutcome::Fresh
    } else {
        StalenessOutcome::Stale
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p temper-cli projection::tests::evaluate_staleness`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/projection.rs
git commit -m "$(cat <<'EOF'
feat(cli): StalenessOutcome + evaluate_staleness pure decision

The staleness pre-flight's decision half: compare a per-context cursor
against the server's latest event id. No IO, fully unit-tested. The
network fetch and the warning output build on this in the next commit.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `check_context_staleness` + `warn_if_context_stale` — the network check

**Files:**
- Modify: `crates/temper-cli/src/projection.rs`
- Test: `tests/e2e/tests/projection_pull_test.rs`

The async half: read a context's cursor, and only if one exists, resolve the context id and fetch the server's latest event id, then run `evaluate_staleness`. A missing cursor short-circuits to `NotProjected` with **zero network calls** — a context the user never pulled costs nothing and never nags. Any failure (offline, unresolvable context) is `Skipped` and silent. `warn_if_context_stale` is the thin caller-facing wrapper that prints the one `⚠` line when the outcome is `Stale`.

- [ ] **Step 1: Write the failing e2e tests**

Add to `tests/e2e/tests/projection_pull_test.rs`. The `ProjectionCursor`/`write_cursor` items are `pub` in `temper_cli::projection`; import what you use at the call site with full paths as below.

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn staleness_not_projected_when_context_never_pulled(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client.contexts().create("snp").await.expect("ctx");
    seed_resource(&app, "snp", "research", "Doc").await;

    let config = projection_test_config(&app);
    // Never pulled — no cursor sidecar exists.
    let outcome =
        temper_cli::projection::check_context_staleness(&app.client, &config.state_dir, "snp")
            .await;
    assert_eq!(
        outcome,
        temper_cli::projection::StalenessOutcome::NotProjected
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn staleness_fresh_immediately_after_pull(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client.contexts().create("sfr").await.expect("ctx");
    seed_resource(&app, "sfr", "research", "Doc").await;

    let config = projection_test_config(&app);
    temper_cli::projection::pull_context(&app.client, &config, "sfr")
        .await
        .expect("pull");

    let outcome =
        temper_cli::projection::check_context_staleness(&app.client, &config.state_dir, "sfr")
            .await;
    assert_eq!(outcome, temper_cli::projection::StalenessOutcome::Fresh);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn staleness_stale_after_post_pull_write(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client.contexts().create("sst").await.expect("ctx");
    seed_resource(&app, "sst", "research", "First Doc").await;

    let config = projection_test_config(&app);
    temper_cli::projection::pull_context(&app.client, &config, "sst")
        .await
        .expect("first pull");

    // A write after the pull advances the context's event stream.
    seed_resource(&app, "sst", "research", "Second Doc").await;

    let outcome =
        temper_cli::projection::check_context_staleness(&app.client, &config.state_dir, "sst")
            .await;
    assert_eq!(outcome, temper_cli::projection::StalenessOutcome::Stale);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn staleness_skipped_when_context_unresolvable(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    // A cursor exists on disk for a context that does not exist on the
    // server (e.g. a stale sidecar for a deleted context). The check reads
    // the cursor, fails to resolve the context id, and skips silently.
    let config = projection_test_config(&app);
    temper_cli::projection::write_cursor(
        &config.state_dir,
        "ghost",
        &temper_cli::projection::ProjectionCursor {
            last_event_id: None,
            pulled_at: chrono::Utc::now(),
        },
    )
    .expect("write cursor");

    let outcome =
        temper_cli::projection::check_context_staleness(&app.client, &config.state_dir, "ghost")
            .await;
    assert_eq!(outcome, temper_cli::projection::StalenessOutcome::Skipped);
}
```

> `projection_test_config` and `seed_resource` are the Chunk 1 helpers already in this file. `chrono` is a dependency of the e2e crate (Chunk 1 used `Utc::now()` in this file); if the path differs, match the existing Chunk 1 usage.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db staleness_`
Expected: FAIL — `no function check_context_staleness in projection`.

- [ ] **Step 3: Implement `resolve_context_id`, `check_context_staleness`, `warn_if_context_stale`**

Add to `crates/temper-cli/src/projection.rs`. The staleness items need no new top-of-file imports beyond what Tasks 1/3 added (`TemperClient`, `Uuid`, `Path` are already imported).

```rust
/// Resolve a context name to its UUID via the contexts list. Returns `None`
/// when the context is not found or the API call fails — the caller treats
/// either as "cannot check", not as an error.
async fn resolve_context_id(client: &TemperClient, context: &str) -> Option<Uuid> {
    let rows = client.contexts().list().await.ok()?;
    rows.into_iter()
        .find(|c| c.name == context)
        .map(|c| Uuid::from(c.id))
}

/// Non-blocking staleness pre-flight for one context. Reads the context's
/// cursor sidecar; only if one exists does it resolve the context id and
/// fetch the server's latest event id. Never errors and never blocks:
///
/// - no cursor            → `NotProjected` (zero network calls)
/// - cursor + server even → `Fresh`
/// - cursor + server ahead → `Stale`
/// - any failure          → `Skipped` (debug log)
pub async fn check_context_staleness(
    client: &TemperClient,
    state_dir: &Path,
    context: &str,
) -> StalenessOutcome {
    let cursor = match read_cursor(state_dir, context) {
        Ok(Some(cursor)) => cursor,
        Ok(None) => return StalenessOutcome::NotProjected,
        Err(e) => {
            tracing::debug!("staleness check skipped: cursor read failed for {context}: {e}");
            return StalenessOutcome::Skipped;
        }
    };
    let Some(context_id) = resolve_context_id(client, context).await else {
        tracing::debug!("staleness check skipped: could not resolve context '{context}'");
        return StalenessOutcome::Skipped;
    };
    let server_latest = match client.events().latest_for_context(context_id).await {
        Ok(latest) => latest,
        Err(e) => {
            tracing::debug!("staleness check skipped: latest_for_context failed: {e}");
            return StalenessOutcome::Skipped;
        }
    };
    evaluate_staleness(&cursor, server_latest)
}

/// Run the staleness pre-flight and print one warning line if the context's
/// projection is stale. All other outcomes are silent. This is the
/// caller-facing entry point for context-touching commands.
pub async fn warn_if_context_stale(client: &TemperClient, state_dir: &Path, context: &str) {
    if check_context_staleness(client, state_dir, context).await == StalenessOutcome::Stale {
        crate::output::warning(format!(
            "projection for '{context}' is stale — run `temper pull {context}` to refresh"
        ));
    }
}
```

> Verify against the real client API: `client.contexts().list()` returns `Result<Vec<ContextRow>>`; `ContextRow` has `name: String` and `id: ContextId`. `Uuid::from(c.id)` converts the `ContextId` newtype (same conversion Chunk 1's `pull_context` uses for `kb_context_id`). `client.events().latest_for_context(Uuid)` returns `Result<Option<Uuid>>` (Chunk 1). `crate::output::warning` writes one line to stderr.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db staleness_`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/projection.rs tests/e2e/tests/projection_pull_test.rs
git commit -m "$(cat <<'EOF'
feat(cli): check_context_staleness + warn_if_context_stale

The async half of the staleness pre-flight: read a context's cursor and,
only when one exists, resolve the context id and compare against the
server's latest event id. A never-pulled context short-circuits with zero
network calls; offline or unresolvable contexts skip silently.
warn_if_context_stale prints the one ⚠ line on a Stale outcome.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Wire the staleness pre-flight into `resource list` and `search`

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs`
- Modify: `crates/temper-cli/src/commands/search_cmd.rs`

Both commands run a cloud query inside a `runtime::with_client` closure. Add `warn_if_context_stale` as the first thing the closure does, gated on a `--context` actually being present. The warning goes to stderr via `output::warning`, so machine-readable stdout (`--format json`) is unaffected — no format gating is needed.

- [ ] **Step 1: Wire into `resource list`**

In `crates/temper-cli/src/commands/resource.rs`, the `list` function builds `let context = params.context.map(ToString::to_string);` and then runs:

```rust
    let rows_result = runtime::with_client(move |client| {
        Box::pin(async move { fetch_list_rows(client, &doc_type, context.as_deref(), limit).await })
    });
```

Before the `runtime::with_client` call, add a clone of the state dir:

```rust
    let state_dir = config.state_dir.clone();
```

Then replace the `runtime::with_client` call with the staleness-aware form (match the real `with_client` closure shape — the Chunk 1 `commands/pull.rs` rewrite is the reference for the `let x = x.clone(); Box::pin(async move { ... })` pattern):

```rust
    let rows_result = runtime::with_client(move |client| {
        let doc_type = doc_type.clone();
        let context = context.clone();
        let state_dir = state_dir.clone();
        Box::pin(async move {
            if let Some(ctx) = context.as_deref() {
                crate::projection::warn_if_context_stale(client, &state_dir, ctx).await;
            }
            fetch_list_rows(client, &doc_type, context.as_deref(), limit).await
        })
    });
```

> `doc_type` is the `String` bound earlier in `list` (`let doc_type = params.doc_type.to_string();`). `config.state_dir` is the `.temper` state directory (Chunk 1's `projection_test_config` reads `config.state_dir`). If the real `with_client` closure does not need the inner `.clone()` lines (depends on whether it is `FnOnce`), keep only what compiles — the goal is: when a context is set, call `warn_if_context_stale` before `fetch_list_rows`.

- [ ] **Step 2: Wire into `search`**

In `crates/temper-cli/src/commands/search_cmd.rs`, the `run` function has `context: Option<&str>`, `vault_root`, and `temper_dir` (`= vault_root.join(".temper")`). It runs:

```rust
    let results = runtime::with_client(|client| {
        let params = search_actions::build_search_params(search_actions::CliSearchArgs { ... });
        Box::pin(async move { search_actions::search_api(client, params).await })
    })?;
```

Before that call, add owned copies for the closure:

```rust
    let ctx_for_check = context.map(ToString::to_string);
    let state_dir = temper_dir.clone();
```

Then make the closure run the staleness check first:

```rust
    let results = runtime::with_client(|client| {
        let params = search_actions::build_search_params(search_actions::CliSearchArgs {
            query,
            embedding: embedding.clone(),
            context,
            doc_type,
            limit,
            seed_ids: seed_ids.clone(),
            edge_types: edge_types.clone(),
            depth,
            no_graph,
        });
        let ctx_for_check = ctx_for_check.clone();
        let state_dir = state_dir.clone();
        Box::pin(async move {
            if let Some(ctx) = ctx_for_check.as_deref() {
                crate::projection::warn_if_context_stale(client, &state_dir, ctx).await;
            }
            search_actions::search_api(client, params).await
        })
    })?;
```

> `temper_dir` is the `.temper` directory the function already computes (`let temper_dir = vault_root.join(".temper");`). Keep the `build_search_params` call exactly as it is in the current file — the only additions are the two `let` lines and the `if let Some(ctx) ...` block at the top of the `async move` body. Match the real closure shape; do not change the search behavior itself.

- [ ] **Step 3: Verify it compiles and run the existing command tests**

Run: `SQLX_OFFLINE=true cargo build -p temper-cli`
Expected: clean build.

Run: `cargo nextest run -p temper-cli`
Expected: PASS — the `temper-cli` unit suite (including the projection tests) stays green.

> This task has no dedicated test of its own: the wiring is `if context is set, call warn_if_context_stale`. `warn_if_context_stale`'s behavior is covered by Task 4's e2e tests on `check_context_staleness`. Correctness here is compilation plus the suite staying green (Task 6). The end-to-end "a post-pull write makes `resource list` print the ⚠ line" is the `staleness_stale_after_post_pull_write` test exercising the same `check_context_staleness` the command now calls.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs crates/temper-cli/src/commands/search_cmd.rs
git commit -m "$(cat <<'EOF'
feat(cli): staleness pre-flight on `resource list` and `search`

When a --context is set, both commands run the non-blocking staleness
pre-flight before their cloud query: if that context's projection cursor
exists and the server has advanced, a single ⚠ line prints to stderr.
A never-pulled context is silent. stdout (incl. --format json) is
unaffected — the warning goes to stderr.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Full verification

**Files:** none (verification only)

- [ ] **Step 1: Quality gates**

Run: `cargo make check`
Expected: fmt, clippy (`-D warnings`), docs, machete, and TypeScript checks all pass.

> If clippy fails with `error communicating with database`, start Docker Postgres (`cargo make docker-up`) or prefix with `SQLX_OFFLINE=true`.

- [ ] **Step 2: Workspace test sweep**

Run: `cargo nextest run --workspace`
Expected: PASS. Per the `feedback_workspace_test_surfaces_pipeline_bugs` lesson, the workspace run activates feature unification that narrower runs miss — it must be green.

- [ ] **Step 3: e2e suite**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db`
Expected: PASS — including the new `projection_pull_test.rs` tests (`write_resource_file_from_parts_materializes_a_document`, `staleness_not_projected_when_context_never_pulled`, `staleness_fresh_immediately_after_pull`, `staleness_stale_after_post_pull_write`, `staleness_skipped_when_context_unresolvable`), every Chunk 1 projection test, and the pre-existing `show` tests (`show_cache_e2e_test`, `cloud_only_show_fallback_test`). This chunk is additive; nothing should regress.

- [ ] **Step 4: Confirm additivity**

Run: `grep -rn "VaultState\|VaultBackend\|sync_orchestration" crates/temper-cli/src/projection.rs`
Expected: no matches — the projection module still does not depend on the mode switch or the sync engine.

Run: `grep -n "VaultState::Local" crates/temper-cli/src/commands/resource.rs`
Expected: still present — `show_generic`'s local-mode branch is untouched. Chunk 2 added to the `Cloud` branch only.

- [ ] **Step 5: Mark the task done in the vault**

Run: `temper resource update 2026-05-22-cloud-only-vault-chunk-2-staleness-check-and-per-resource-refresh --type task --stage done`
Expected: stage updated.

---

## Self-Review

**Spec coverage (Chunk 2 acceptance criteria):**
- "Cloud-mode `temper resource show` writes the resource to its canonical projection path" → Tasks 1, 2. ✓
- "After a `pull`, a post-pull write makes the next `resource list` / `search` print the `⚠` stale line" → Tasks 3, 4, 5; the `staleness_stale_after_post_pull_write` e2e test exercises the exact `check_context_staleness` path the commands call. ✓
- "A context that was never pulled produces no warning and no extra API call" → Task 4 (`check_context_staleness` returns `NotProjected` before any network call); `staleness_not_projected_when_context_never_pulled`. ✓
- "Offline / unresolvable-context during the check is silent (debug log)" → Task 4 (`Skipped` arms with `tracing::debug!`); `staleness_skipped_when_context_unresolvable`. ✓
- "Additive only — local mode and existing tests still pass" → Task 6 Steps 3–4; `show_generic`'s `VaultState::Local` branch and the `show_cache` ladder are untouched. ✓
- "`cargo make check` and `cargo nextest run --workspace` green" → Task 6 Steps 1–2. ✓

**Spec refinement notes:**
- The spec lists `show_writes_canonical_projection_file` as an e2e test of the `show` command. The CLI `show_generic` function is private and gated on `VaultState::from_env()`, and the e2e harness drives `projection`/`client` library functions directly rather than self-client-constructing `commands::` functions (the Chunk 1 precedent). Chunk 2 therefore tests the projection-write *logic* `show` invokes — `write_resource_file_from_parts` — directly at the e2e layer (Task 1), and treats the cloud-`show` call site as thin wiring (Task 2), exactly as Chunk 1 Task 9 treated the `pull` command rewrite. Coverage of the behavior is equivalent; the test's home moved from the command to the function it calls.
- The spec lists `staleness_check_skipped_when_offline` as a `unit (cli)` test. `check_context_staleness` needs a `TemperClient`, so a true no-IO unit test is not possible; the `Skipped` path is instead covered by `staleness_skipped_when_context_unresolvable` (an e2e test that writes a cursor for a non-existent context — a real scenario, a stale sidecar for a deleted context — and asserts `Skipped`). The pure decision (`evaluate_staleness`) is genuinely unit-tested (Task 3). The `latest_for_context`-errored `Skipped` arm is `Err(_) => Skipped` — correct by inspection and structurally identical to the resolution-failed arm the e2e test does cover.

**Placeholder scan:** no TBD/TODO; every code step contains complete code. The `>`-noted verifications (`ContentResponse` import path, `with_client` closure shape, `ContextRow` field names, `config_clone.vault_root`/`slug_inner` bindings) each name the concrete thing to confirm and the fallback, rather than leaving a guess.

**Type consistency:** `write_resource_file_from_parts(vault_root, row, content) -> Result<PathBuf>` (Task 1) is called identically by `write_resource_file` (Task 1) and cloud `show` (Task 2). `StalenessOutcome` (Task 3, variants `NotProjected`/`Fresh`/`Stale`/`Skipped`) is produced by `evaluate_staleness` (Task 3) and `check_context_staleness` (Task 4) and consumed by `warn_if_context_stale` (Task 4) and the Task 4 e2e tests. `check_context_staleness(client, state_dir, context) -> StalenessOutcome` and `warn_if_context_stale(client, state_dir, context)` signatures are stable across their definition (Task 4) and call sites (Task 5).
