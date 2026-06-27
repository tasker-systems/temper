# Context-ref Addressing Arc Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make context addressing UUID-primary with a decorated `@owner/slug` human form across every surface, rejecting bare `name`, resolving server-side through one function — and light up search's dormant context filter.

**Architecture:** A pure `parse_context_ref` in temper-core returns a `ContextRef` enum; one server-side `resolve_context_ref` (temper-api, visibility-gated) turns a ref + authenticated principal into a `ContextId`. Every inbound `context_name` wire field becomes `context_ref` (still a `String`, now a ref), resolved once at each handler boundary, then SQL filters by the resolved id. Outbound rows gain a copy-pasteable decorated `ref`.

**Tech Stack:** Rust (axum, sqlx, ts-rs, schemars), PostgreSQL, SvelteKit/TS, cargo-make + cargo-nextest.

## Global Constraints

- **Compile gate is whole-workspace.** The pre-commit hook runs `SQLX_OFFLINE=true cargo clippy --all-targets --all-features -- -D warnings`, rustdoc with `-D warnings`, and **temper-cloud** TS typecheck/biome. It does **not** run temper-ui. Every task's commit must leave the entire Rust workspace + temper-cloud green. temper-ui changes (Task 8) are not gate-blocked but must land in this same PR for CI.
- **All vault writes route through `temper-client` → `temper-api`.** Never inline `sqlx::query!()` in a surface; never call write persistence directly from a surface — go through the backend trait. Reads (list/show/search) stay service-direct by design.
- **Typed structs over inline JSON.** No `serde_json::json!()` for known shapes.
- **`#[expect(lint, reason=…)]`** not `#[allow]`. All public types implement `Debug`.
- **sqlx caches:** after changing any production SQL run `cargo sqlx prepare --workspace -- --all-features`; after changing **test-target** SQL run `cargo make prepare-api` (temper-api) and/or `cargo make prepare-e2e`.
- **Env for bare cargo/nextest:** `export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development` and `cargo make docker-up` first. Integration tests use `--features test-db`; e2e via `cargo make test-e2e`.
- **No data migration** — `kb_contexts.slug` is already NOT NULL + populated. This arc is wire + code only.
- **Decorated grammar (verbatim):** `@me/<slug>`, `@<handle>/<slug>`, `+<team-slug>/<slug>`, or a bare UUID. Bare `name` (no sigil, not a UUID) and a sigil-owner without `/<slug>` are **errors**. The path component is always the context **slug**, never `name`.

---

## File map

| File | Responsibility | Task |
|---|---|---|
| `crates/temper-core/src/context_ref.rs` (new) | `ContextRef`, `ContextOwnerRef`, `ContextRefError`, `parse_context_ref` (pure) | 1 |
| `crates/temper-core/src/lib.rs` | add `pub mod context_ref;` | 1 |
| `crates/temper-api/src/services/context_service.rs` | `resolve_context_ref` (server-side, gated); later remove `resolve_by_name` | 2,3 |
| `crates/temper-core/src/types/ingest.rs` | `IngestPayload.context_name → context_ref` | 3 |
| `crates/temper-workflow/src/operations/commands.rs` | `CreateResource.context: String → ContextId`; `UpdateResource.FileMove.context_to → ContextId` | 3,6 |
| `crates/temper-api/src/handlers/ingest.rs` | resolve ref at boundary; build `CreateResource` with `ContextId` | 3 |
| `crates/temper-api/src/backend/db_backend.rs` | create/move use resolved `ContextId`, drop `writes::resolve_context` calls | 3,6 |
| `crates/temper-mcp/src/resources.rs` | create/list tools `context_name → context_ref`; drop `resolve_by_name` | 3,4 |
| `crates/temper-client/src/resources.rs` | client create/list/search send `context_ref` | 3,4,5 |
| `crates/temper-cli/src/cli.rs`, `commands/resource.rs`, `commands/search.rs` | `--context`/`--context-to` pass refs; error text | 3,4,5,6 |
| `crates/temper-workflow/src/types/resource.rs` | `ResourceListParams.context_name → context_ref`, drop `kb_context_id` | 4 |
| `crates/temper-api/src/backend/substrate_read.rs` | list filter `c.name=$ → c.id=$resolved`; `search_select` passes resolved `p_context_id` | 4,5 |
| `crates/temper-core/src/types/api.rs` | `SearchParams.context_name → context_ref`; outbound `context_ref` on result row | 5,7 |
| `crates/temper-api/src/handlers/resources.rs`, `search.rs` | resolve refs at boundary | 4,5,6 |
| `crates/temper-core/src/types/context.rs` | `ContextRow`/`ContextRowWithCounts` add `slug` + `owner_ref` (raw ingredients) | 7 |
| `crates/temper-substrate/src/readback/mod.rs` | surface home-context owner decoration + context slug on resource/search rows | 7 |
| `crates/temper-cli/src/commands/resource.rs`, `commands/context.rs` | inject decorated `ref`/`context_ref` at output (mirror `decorated_ref` at :62) | 7 |
| `crates/temper-ui/src/**` | build `${owner}/${context}` refs; render `ref` | 8 |
| `crates/temper-cli/skill-content/*`, `agent-skills/*` | decorated `--context` everywhere; regen via `temper skill install` | 9 |

---

## Task 1: `parse_context_ref` + `ContextRef` (temper-core, pure)

**Files:**
- Create: `crates/temper-core/src/context_ref.rs`
- Modify: `crates/temper-core/src/lib.rs` (add `pub mod context_ref;` after `pub mod validation;`, line ~12)
- Test: inline `#[cfg(test)]` in `context_ref.rs`

**Interfaces:**
- Consumes: `temper_core::validation::validate_owner_pattern(&str) -> Result<(), OwnerPatternError>` (validates `@handle`/`+team`).
- Produces:
  ```rust
  pub enum ContextOwnerRef { Me, Handle(String), Team(String) }
  pub enum ContextRef { Id(uuid::Uuid), OwnerSlug { owner: ContextOwnerRef, slug: String } }
  pub fn parse_context_ref(s: &str) -> Result<ContextRef, ContextRefError>;
  ```

- [ ] **Step 1: Write the failing tests**

Add to `crates/temper-core/src/context_ref.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn parses_bare_uuid() {
        let u = Uuid::now_v7();
        assert_eq!(parse_context_ref(&u.to_string()).unwrap(), ContextRef::Id(u));
    }

    #[test]
    fn parses_me_slug() {
        let r = parse_context_ref("@me/temper").unwrap();
        assert_eq!(r, ContextRef::OwnerSlug { owner: ContextOwnerRef::Me, slug: "temper".into() });
    }

    #[test]
    fn parses_handle_slug() {
        let r = parse_context_ref("@j-cole-taylor/temper").unwrap();
        assert_eq!(
            r,
            ContextRef::OwnerSlug { owner: ContextOwnerRef::Handle("j-cole-taylor".into()), slug: "temper".into() }
        );
    }

    #[test]
    fn parses_team_slug() {
        let r = parse_context_ref("+tasker-systems/general").unwrap();
        assert_eq!(
            r,
            ContextRef::OwnerSlug { owner: ContextOwnerRef::Team("tasker-systems".into()), slug: "general".into() }
        );
    }

    #[test]
    fn rejects_bare_name() {
        assert!(matches!(parse_context_ref("temper"), Err(ContextRefError::BareName(_))));
    }

    #[test]
    fn rejects_owner_without_slug() {
        assert!(matches!(parse_context_ref("@me"), Err(ContextRefError::MissingSlug(_))));
        assert!(matches!(parse_context_ref("+team"), Err(ContextRefError::MissingSlug(_))));
    }

    #[test]
    fn rejects_empty_slug() {
        assert!(parse_context_ref("@me/").is_err());
    }

    #[test]
    fn rejects_bad_owner() {
        assert!(parse_context_ref("@/temper").is_err());
        assert!(parse_context_ref("@UPPER/temper").is_err());
        assert!(parse_context_ref("temper/x").is_err()); // no sigil
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(
            parse_context_ref("  @me/temper  ").unwrap(),
            ContextRef::OwnerSlug { owner: ContextOwnerRef::Me, slug: "temper".into() }
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p temper-core context_ref`
Expected: FAIL — `parse_context_ref` / `ContextRef` not found.

- [ ] **Step 3: Write the implementation**

Top of `crates/temper-core/src/context_ref.rs`:

```rust
//! Context addressing by ref: a bare UUID or a decorated `@owner/slug` form.
//!
//! UUID-primary, mirroring resource refs (`temper_workflow::operations::parse_ref`).
//! Pure string parsing — no DB, no principal. Resolution to a `ContextId`
//! (owner lookup + visibility gate) lives server-side in temper-api.

use uuid::Uuid;

use crate::validation::validate_owner_pattern;

/// The owner half of a decorated context ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextOwnerRef {
    /// `@me` — the calling principal's own profile.
    Me,
    /// `@<handle>` — a personal profile addressed by its global-unique handle.
    Handle(String),
    /// `+<team-slug>` — a team addressed by its global-unique slug.
    Team(String),
}

/// A parsed context reference. UUID-primary; the decorated form carries an
/// owner + the context's per-owner-unique slug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextRef {
    /// Canonical: the `kb_contexts.id` UUID.
    Id(Uuid),
    /// Decorated: resolved via the `(owner_table, owner_id, slug)` natural key.
    OwnerSlug { owner: ContextOwnerRef, slug: String },
}

/// Why a context ref string could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContextRefError {
    #[error("not a context ref: bare names are not addressable — use a UUID or `@owner/slug` (got {0:?})")]
    BareName(String),
    #[error("context ref is missing the `/slug` after the owner (got {0:?})")]
    MissingSlug(String),
    #[error("context ref slug must be lowercase alphanumeric with hyphens (got {0:?})")]
    BadSlug(String),
    #[error("context ref owner is invalid: {0}")]
    BadOwner(#[from] crate::validation::OwnerPatternError),
}

/// Same slug rules contexts already enforce: lowercase alnum + hyphens, leading-alnum.
fn validate_slug(slug: &str) -> Result<(), ContextRefError> {
    let ok = !slug.is_empty()
        && slug.as_bytes()[0].is_ascii_alphanumeric()
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(ContextRefError::BadSlug(slug.to_owned()))
    }
}

/// Parse a context ref. Pure — no DB, no principal. See [`ContextRef`].
pub fn parse_context_ref(s: &str) -> Result<ContextRef, ContextRefError> {
    let s = s.trim();

    // Bare UUID — canonical.
    if let Ok(id) = Uuid::parse_str(s) {
        return Ok(ContextRef::Id(id));
    }

    let first = s.as_bytes().first().copied();
    if first != Some(b'@') && first != Some(b'+') {
        return Err(ContextRefError::BareName(s.to_owned()));
    }

    // Decorated: `<owner>/<slug>` where owner keeps its sigil.
    let (owner_part, slug) = s
        .split_once('/')
        .ok_or_else(|| ContextRefError::MissingSlug(s.to_owned()))?;

    validate_owner_pattern(owner_part)?; // validates `@handle` / `+team`
    validate_slug(slug)?;

    let owner = if owner_part == "@me" {
        ContextOwnerRef::Me
    } else if let Some(handle) = owner_part.strip_prefix('@') {
        ContextOwnerRef::Handle(handle.to_owned())
    } else {
        // `+` guaranteed by validate_owner_pattern's sigil check
        ContextOwnerRef::Team(owner_part[1..].to_owned())
    };

    Ok(ContextRef::OwnerSlug { owner, slug: slug.to_owned() })
}
```

Add `pub mod context_ref;` to `crates/temper-core/src/lib.rs` after line 12 (`pub mod validation;`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-core context_ref`
Expected: PASS (10 tests).

- [ ] **Step 5: Clippy + commit**

Run: `SQLX_OFFLINE=true cargo clippy -p temper-core --all-features -- -D warnings`
```bash
git add crates/temper-core/src/context_ref.rs crates/temper-core/src/lib.rs
git commit -m "feat(core): parse_context_ref — UUID-primary + decorated @owner/slug"
```

---

## Task 2: `resolve_context_ref` resolver (temper-api, visibility-gated)

**Files:**
- Modify: `crates/temper-api/src/services/context_service.rs` (add resolver; keep `resolve_by_name` for now)
- Test: `crates/temper-api/tests/context_ref_resolve_test.rs` (new, `--features test-db`)

**Interfaces:**
- Consumes: `temper_core::context_ref::{ContextRef, ContextOwnerRef, parse_context_ref}`; `temper_core::types::ids::{ContextId, ProfileId}`; `ApiError::{NotFound, Forbidden}`.
- Produces:
  ```rust
  pub async fn resolve_context_ref(
      pool: &PgPool, principal: ProfileId, r: &ContextRef,
  ) -> ApiResult<ContextId>;
  ```

Resolution rules (verbatim from spec §3.2). Visibility gate reuses the predicate already in `resolve_by_name` (context_service.rs:82): owned-by-principal OR shared via `kb_team_contexts`/`kb_team_members`.

- [ ] **Step 1: Write the failing integration tests**

Create `crates/temper-api/tests/context_ref_resolve_test.rs`. Follow the harness in `crates/temper-api/tests/common/fixtures.rs` (it builds owned/team contexts via `writes::resolve_context` by owner+slug). Tests:

```rust
// Pseudocode shape — fill bodies against the existing fixtures harness in
// crates/temper-api/tests/common/. Each uses #[sqlx::test] with the api MIGRATOR.
//
// 1. resolves @me/<slug> to the caller's own context id
// 2. two visible contexts sharing a NAME but different slugs each resolve
//    distinctly by slug (the ambiguity-fix regression)
// 3. resolves +<team-slug>/<slug> for a member; Forbidden/NotFound for a non-member
// 4. resolves a bare UUID iff visible; NotFound when not visible
// 5. @<handle>/<slug> for another visible profile resolves; NotFound when not visible
```

Mirror the assertion style of `crates/temper-api/tests/relationship_handler_test.rs`. Use `parse_context_ref` to build the `ContextRef` inputs, then `resolve_context_ref`.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo make docker-up && cargo nextest run -p temper-api --features test-db --test context_ref_resolve_test`
Expected: FAIL — `resolve_context_ref` not found.

- [ ] **Step 3: Implement the resolver**

Add to `crates/temper-api/src/services/context_service.rs`:

```rust
use temper_core::context_ref::{ContextOwnerRef, ContextRef};
use temper_core::types::ids::{ContextId, ProfileId};

/// Resolve a context ref to a `ContextId`, gated to what `principal` may see.
///
/// The single source of truth for context resolution. `@me` uses the caller's
/// profile; `@handle`/`+team` resolve the owner then the `(owner, slug)` row;
/// a bare UUID must be visible. Replaces `resolve_by_name` (name was ambiguous).
pub async fn resolve_context_ref(
    pool: &PgPool,
    principal: ProfileId,
    r: &ContextRef,
) -> ApiResult<ContextId> {
    match r {
        ContextRef::Id(id) => {
            // Visible-to-principal gate (same predicate as resolve_by_name).
            let found = sqlx::query_scalar!(
                r#"
                SELECT c.id FROM kb_contexts c
                 WHERE c.id = $2
                   AND ((c.owner_table = 'kb_profiles' AND c.owner_id = $1)
                        OR EXISTS (
                             SELECT 1 FROM kb_team_contexts tc
                               JOIN kb_team_members tm ON tm.team_id = tc.team_id
                              WHERE tc.context_id = c.id AND tm.profile_id = $1))
                "#,
                *principal,
                id
            )
            .fetch_optional(pool)
            .await?;
            found.map(ContextId::from).ok_or(ApiError::NotFound)
        }
        ContextRef::OwnerSlug { owner, slug } => match owner {
            ContextOwnerRef::Me => lookup_profile_context(pool, *principal, slug).await,
            ContextOwnerRef::Handle(handle) => {
                let owner_id = sqlx::query_scalar!(
                    "SELECT id FROM kb_profiles WHERE handle = $1",
                    handle
                )
                .fetch_optional(pool)
                .await?
                .ok_or(ApiError::NotFound)?;
                // Resolve, then gate visibility to the principal.
                let cid = lookup_profile_context(pool, owner_id, slug).await?;
                ensure_context_visible(pool, *principal, *cid).await?;
                Ok(cid)
            }
            ContextOwnerRef::Team(team_slug) => {
                let team_id = sqlx::query_scalar!(
                    "SELECT id FROM kb_teams WHERE slug = $1",
                    team_slug
                )
                .fetch_optional(pool)
                .await?
                .ok_or(ApiError::NotFound)?;
                // Membership gate.
                let is_member = sqlx::query_scalar!(
                    r#"SELECT EXISTS(
                         SELECT 1 FROM kb_team_members
                          WHERE team_id = $1 AND profile_id = $2) AS "ok!""#,
                    team_id,
                    *principal
                )
                .fetch_one(pool)
                .await?;
                if !is_member {
                    return Err(ApiError::Forbidden);
                }
                let id = sqlx::query_scalar!(
                    "SELECT id FROM kb_contexts \
                     WHERE owner_table = 'kb_teams' AND owner_id = $1 AND slug = $2",
                    team_id,
                    slug
                )
                .fetch_optional(pool)
                .await?
                .ok_or(ApiError::NotFound)?;
                Ok(ContextId::from(id))
            }
        },
    }
}

async fn lookup_profile_context(
    pool: &PgPool,
    owner_id: uuid::Uuid,
    slug: &str,
) -> ApiResult<ContextId> {
    let id = sqlx::query_scalar!(
        "SELECT id FROM kb_contexts \
         WHERE owner_table = 'kb_profiles' AND owner_id = $1 AND slug = $2",
        owner_id,
        slug
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(ContextId::from(id))
}

async fn ensure_context_visible(
    pool: &PgPool,
    principal: uuid::Uuid,
    context_id: uuid::Uuid,
) -> ApiResult<()> {
    let visible = sqlx::query_scalar!(
        r#"
        SELECT EXISTS (
          SELECT 1 FROM kb_contexts c
           WHERE c.id = $2
             AND ((c.owner_table = 'kb_profiles' AND c.owner_id = $1)
                  OR EXISTS (
                       SELECT 1 FROM kb_team_contexts tc
                         JOIN kb_team_members tm ON tm.team_id = tc.team_id
                        WHERE tc.context_id = c.id AND tm.profile_id = $1)))
        AS "ok!""#,
        principal,
        context_id
    )
    .fetch_one(pool)
    .await?;
    if visible { Ok(()) } else { Err(ApiError::NotFound) }
}
```

- [ ] **Step 4: Regenerate the api test sqlx cache, run tests**

Run: `cargo make prepare-api && cargo nextest run -p temper-api --features test-db --test context_ref_resolve_test`
Expected: PASS.

- [ ] **Step 5: Clippy + commit**

Run: `SQLX_OFFLINE=true cargo clippy -p temper-api --all-targets --all-features -- -D warnings`
```bash
git add crates/temper-api/src/services/context_service.rs crates/temper-api/tests/context_ref_resolve_test.rs crates/temper-api/.sqlx
git commit -m "feat(api): resolve_context_ref — one gated server-side resolver"
```

---

## Task 3: Create path cutover (`context_name → context_ref`, resolve at ingest)

Atomic Rust commit: renaming `IngestPayload.context_name` breaks client/cli/mcp until all updated.

**Files:**
- Modify: `crates/temper-core/src/types/ingest.rs:16` (`context_name` → `context_ref`)
- Modify: `crates/temper-workflow/src/operations/commands.rs:28` (`CreateResource.context: String` → `pub context: ContextId`) + its unit tests in the same file
- Modify: `crates/temper-api/src/handlers/ingest.rs:48` (resolve ref → `ContextId`, build `CreateResource`)
- Modify: `crates/temper-api/src/backend/db_backend.rs:580` (use `cmd.context` directly; drop `writes::resolve_context`)
- Modify: `crates/temper-mcp/src/resources.rs:161` (parse+resolve `context_ref`; drop `resolve_by_name`)
- Modify: `crates/temper-client/src/resources.rs` (create sends `context_ref`)
- Modify: `crates/temper-cli/src/commands/resource.rs:176` (pass `--context` value as the ref)
- Test: e2e in `tests/e2e/tests/` (create into context by ref; bare-name rejected)

**Interfaces:**
- Consumes: `resolve_context_ref` (Task 2), `parse_context_ref` (Task 1), `ContextId`.
- Produces: `IngestPayload.context_ref: String`; `CreateResource.context: ContextId`.

- [ ] **Step 1: Write the failing e2e test**

In a new `tests/e2e/tests/context_ref_test.rs` (mirror an existing e2e like `tests/e2e/tests/` resource-create flows). Assert:
```rust
// 1. POST create with context_ref = "@me/<existing-slug>" succeeds and lands in that context.
// 2. POST create with context_ref = "temper" (bare name) → 400 BAD_REQUEST.
// 3. POST create with a bare UUID of a visible context succeeds.
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo make test-e2e` (filtered): `cargo nextest run -p temper-e2e --features test-db --test context_ref_test`
Expected: FAIL (field `context_ref` doesn't exist yet / bare name still accepted).

- [ ] **Step 3: Rename the wire field**

`crates/temper-core/src/types/ingest.rs:16`:
```rust
    // before: pub context_name: String,
    /// Context **ref** (UUID or decorated `@owner/slug`), resolved server-side.
    pub context_ref: String,
```

- [ ] **Step 4: Make `CreateResource` carry a resolved `ContextId`**

`crates/temper-workflow/src/operations/commands.rs:28` — change `pub context: String,` to:
```rust
    /// The resolved home context (resolution happens at the surface boundary).
    pub context: temper_core::types::ids::ContextId,
```
Update the unit tests in the same file that build `CreateResource` to pass a `ContextId::new()`.

- [ ] **Step 5: Resolve at the ingest boundary**

`crates/temper-api/src/handlers/ingest.rs` — before building the command:
```rust
    use temper_core::context_ref::parse_context_ref;
    let cref = parse_context_ref(&payload.context_ref)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let context = crate::services::context_service::resolve_context_ref(
        &state.pool, ProfileId::from(auth.0.profile.id), &cref,
    ).await?;
    // ...
    let cmd = CreateResource { context, /* doctype, slug, title, body, … unchanged */ };
```

`crates/temper-api/src/backend/db_backend.rs:580` — drop the `writes::resolve_context(&self.pool, owner, &cmd.context)` call; use `cmd.context` directly as the home `ContextId`.

- [ ] **Step 6: Update MCP create (drop `resolve_by_name`)**

`crates/temper-mcp/src/resources.rs:161` — replace the `resolve_by_name` call with `parse_context_ref` + `resolve_context_ref`; rename the tool input field `context_name → context_ref` (and its doc string: "Context ref (UUID or `@owner/slug`)"). No enum, so no `schemars(inline)` concern.

- [ ] **Step 7: Update client + CLI**

`crates/temper-client/src/resources.rs` create: send `context_ref` in the payload.
`crates/temper-cli/src/commands/resource.rs:176`: the `--context` value is now passed through verbatim as the ref (no local name handling). Keep `require_context` (still required for create) but update its error text (Task 9 owns the user-facing copy; here just keep it compiling).

- [ ] **Step 8: Prepare caches, run unit + e2e**

Run:
```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-e2e
cargo nextest run -p temper-e2e --features test-db --test context_ref_test
cargo nextest run --workspace   # ensure workflow/core unit tests green
```
Expected: PASS.

- [ ] **Step 9: Clippy + commit**

Run: `SQLX_OFFLINE=true cargo clippy --all-targets --all-features -- -D warnings`
```bash
git add -A
git commit -m "feat: create path takes context_ref, resolved server-side; drop resolve_by_name"
```

---

## Task 4: List path cutover (filter by resolved id — the ambiguity fix)

**Files:**
- Modify: `crates/temper-workflow/src/types/resource.rs:84` (`context_name → context_ref`); delete `kb_context_id` (line ~82) from the params
- Modify: `crates/temper-api/src/backend/substrate_read.rs:74-161` (`filtered_visible_page`: resolve ref → id; SQL `c.name = $2` → `c.id = $2`); also the meta path (`list_meta_select`)
- Modify: `crates/temper-mcp/src/resources.rs` `ListResourcesInput.context_name → context_ref`
- Modify: `crates/temper-client/src/resources.rs` list/list_meta
- Modify: `crates/temper-cli/src/commands/resource.rs` list
- Test: `crates/temper-api/tests/` list-by-context-ref integration (`--features test-db`)

**Interfaces:**
- Consumes: `resolve_context_ref`, `parse_context_ref`.
- Produces: `ResourceListParams.context_ref: Option<String>` (no `kb_context_id`).

- [ ] **Step 1: Write the failing integration test**

New `crates/temper-api/tests/list_context_ref_test.rs`: seed two visible contexts sharing a `name` but distinct slugs, each with a resource; assert `list(context_ref="@me/<slugA>")` returns only A's resource (today's `c.name=$` would return both / first-match). Assert bare name → BadRequest.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db --test list_context_ref_test`
Expected: FAIL.

- [ ] **Step 3: Rename params, drop `kb_context_id`**

`crates/temper-workflow/src/types/resource.rs`: remove `pub kb_context_id: Option<Uuid>,` (line ~82) and rename `pub context_name: Option<String>,` → `pub context_ref: Option<String>,`.

- [ ] **Step 4: Resolve + filter by id**

`crates/temper-api/src/backend/substrate_read.rs` `filtered_visible_page` (and `list_meta_select`): before building SQL, resolve the optional ref to an `Option<Uuid>`:
```rust
    let context_id: Option<uuid::Uuid> = match params.context_ref.as_deref() {
        Some(s) => {
            let cref = temper_core::context_ref::parse_context_ref(s)
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            Some(*crate::services::context_service::resolve_context_ref(pool, ProfileId::from(profile_id), &cref).await?)
        }
        None => None,
    };
```
Change the SQL predicate `AND ($2::text IS NULL OR c.name = $2)` (line ~113) to `AND ($2::uuid IS NULL OR c.id = $2)` and bind `context_id` instead of `params.context_name` (line ~125).

- [ ] **Step 5: Update mcp/client/cli consumers**

Rename `ListResourcesInput.context_name → context_ref` (mcp) + doc; client list/list_meta build `context_ref`; CLI list passes `--context` through as the ref.

- [ ] **Step 6: Prepare caches, run tests**

Run:
```bash
cargo sqlx prepare --workspace -- --all-features && cargo make prepare-api
cargo nextest run -p temper-api --features test-db --test list_context_ref_test
cargo nextest run --workspace
```
Expected: PASS.

- [ ] **Step 7: Clippy + commit**

```bash
SQLX_OFFLINE=true cargo clippy --all-targets --all-features -- -D warnings
git add -A
git commit -m "feat: resource list filters by resolved context id (fixes name ambiguity)"
```

---

## Task 5: Search path cutover (light the dormant `p_context_id`)

**Files:**
- Modify: `crates/temper-core/src/types/api.rs:57` (`SearchParams.context_name → context_ref`; update the long doc comment — it no longer says "not honored")
- Modify: `crates/temper-api/src/backend/substrate_read.rs:300-353` (`search_select`: resolve ref → `Option<ContextId>`, pass as `context_id` instead of `None`)
- Modify: `crates/temper-client/src/resources.rs` search; `crates/temper-cli/src/commands/search.rs`
- Test: `crates/temper-api/tests/` search-by-context-ref integration

**Interfaces:**
- Consumes: `resolve_context_ref`; `readback::UnifiedSearchQuery.context_id` (already `Option<Uuid>`).
- Produces: `SearchParams.context_ref: Option<String>`.

- [ ] **Step 1: Write the failing integration test**

New `crates/temper-api/tests/search_context_ref_test.rs`: two contexts, a matching resource in each; assert `search(query, context_ref="@me/<slugA>")` returns only A's hit; assert `search(context_ref="@me/no-such")` → NotFound; assert unknown bare name → BadRequest (closes Beat-2 C1).

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db --test search_context_ref_test`
Expected: FAIL (context currently ignored — both hits returned).

- [ ] **Step 3: Rename + update doc**

`crates/temper-core/src/types/api.rs:57`: `pub context_ref: Option<String>,` with doc: `/// Filter by context **ref** (UUID or decorated @owner/slug), resolved server-side.`

- [ ] **Step 4: Wire resolution into `search_select`**

`crates/temper-api/src/backend/substrate_read.rs` `search_select` (~300): replace the deferral block + `context_id: None` with:
```rust
    let context_id: Option<uuid::Uuid> = match params.context_ref.as_deref() {
        Some(s) => {
            let cref = temper_core::context_ref::parse_context_ref(s)
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            Some(*crate::services::context_service::resolve_context_ref(pool, ProfileId::from(profile_id), &cref).await?)
        }
        None => None,
    };
    // …
    readback::UnifiedSearchQuery { /* … */ context_id, /* … */ }
```

- [ ] **Step 5: Update client + CLI search consumers** (rename field usage).

- [ ] **Step 6: Prepare caches, run tests**

Run:
```bash
cargo sqlx prepare --workspace -- --all-features && cargo make prepare-api
cargo nextest run -p temper-api --features test-db --test search_context_ref_test
cargo nextest run --workspace
```
Expected: PASS.

- [ ] **Step 7: Clippy + commit**

```bash
SQLX_OFFLINE=true cargo clippy --all-targets --all-features -- -D warnings
git add -A
git commit -m "feat(search): honor context_ref via unified_search p_context_id"
```

---

## Task 6: Resource move (`--context-to`) by ref

**Files:**
- Modify: `crates/temper-workflow/src/operations/commands.rs:71` (`FileMove.context_to: Option<String>` → `Option<ContextId>`) + unit tests in-file
- Modify: `crates/temper-api/src/handlers/resources.rs:240` (resolve `context_to` ref → `ContextId` before building `UpdateResource`)
- Modify: `crates/temper-api/src/backend/db_backend.rs:789` (use resolved id; drop `writes::resolve_context`)
- Modify: `crates/temper-cli/src/cli.rs:290` help text only (value passes through as ref)
- Test: `crates/temper-api/tests/` move-by-ref integration

**Interfaces:** Consumes `resolve_context_ref`. Produces `FileMove.context_to: Option<ContextId>`.

- [ ] **Step 1: Write the failing integration test** — create a resource in context A, `update` it with `context_to="@me/<slugB>"`, assert its home is now B; bare name → BadRequest.

- [ ] **Step 2: Run to verify it fails**
Run: `cargo nextest run -p temper-api --features test-db --test move_context_ref_test`
Expected: FAIL.

- [ ] **Step 3: Change `FileMove.context_to` to `Option<ContextId>`** (commands.rs:71) + update its in-file unit tests (commands.rs:206 builds `UpdateResource`).

- [ ] **Step 4: Resolve at the update boundary** (`handlers/resources.rs:240`): when `context_to` present, `parse_context_ref` + `resolve_context_ref` → `ContextId`; build `UpdateResource`'s `FileMove` with it. In `db_backend.rs:789` use the resolved id directly.

- [ ] **Step 5: Prepare caches, run tests**
Run: `cargo sqlx prepare --workspace -- --all-features && cargo make prepare-api && cargo nextest run -p temper-api --features test-db --test move_context_ref_test && cargo nextest run --workspace`
Expected: PASS.

- [ ] **Step 6: Clippy + commit**
```bash
SQLX_OFFLINE=true cargo clippy --all-targets --all-features -- -D warnings
git add -A && git commit -m "feat: resource move resolves --context-to as a ref"
```

---

## Task 7: Outbound context refs (raw ingredients on rows + presentation injection)

**Pattern to mirror:** resources do **not** store `ref` on the wire struct — the CLI injects it at the output layer (`crates/temper-cli/src/commands/resource.rs:62-67`: `decorated_ref(title, id)` → `obj.insert("ref", …)`) and the UI computes it client-side (`packages/temper-ui/src/lib/ref.ts` `decoratedRef`). Follow that: surface the **raw ingredients** on the rows/producers, inject the decorated string at presentation. The round-trip property is tested by feeding the injected ref back through `parse_context_ref` + `resolve_context_ref`.

**Files:**
- New helper: `crates/temper-core/src/context_ref.rs` — `pub fn decorated_context_ref(owner_table: &str, owner_addressable: &str, context_slug: &str) -> String`
- Modify: `crates/temper-core/src/types/context.rs` (`ContextRow` + `ContextRowWithCounts`: add `pub slug: String`, `pub owner_ref: String`) + regen `context.ts`
- Modify: `crates/temper-api/src/services/context_service.rs` (`create` RETURNING + `list` SELECT: add `c.slug` and the owner addressable via JOIN — `kb_profiles.handle` prefixed `@` / `kb_teams.slug` prefixed `+`)
- Modify: `crates/temper-core/src/types/api.rs` (`UnifiedSearchResultRow`: add `pub context_owner_ref: Option<String>` + reuse existing `context` for slug? — see Step 5) ; `crates/temper-workflow/src/types/resource.rs:18` (`ResourceRow`: add `pub context_owner_ref: String`, `pub context_slug: String`)
- Modify: `crates/temper-substrate/src/readback/mod.rs` (resource/search producers JOIN home-context → owner to surface owner decoration + context slug)
- Modify: `crates/temper-cli/src/commands/resource.rs` (inject `context_ref` into list/show/search output) and a context command output path (inject `ref` for `/api/contexts` listings — mirror :62)
- Test: unit for `decorated_context_ref`; integration round-trip on context rows

**Interfaces:**
- Produces: `decorated_context_ref(owner_table, owner_addressable, slug) -> String` where `owner_addressable` is the bare handle/team-slug (no sigil). Returns `@<handle>/<slug>` (profiles) or `+<team-slug>/<slug>` (teams).

- [ ] **Step 1: Write the failing tests**

Unit (temper-core):
```rust
#[test]
fn decorates_profile_and_team() {
    assert_eq!(decorated_context_ref("kb_profiles", "j-cole-taylor", "temper"), "@j-cole-taylor/temper");
    assert_eq!(decorated_context_ref("kb_teams", "tasker-systems", "general"), "+tasker-systems/general");
}
```
Integration: extend `list_context_ref_test` — for each returned `ContextRowWithCounts`, build `decorated_context_ref(row.kb_owner_table, owner_addressable, row.slug)` (or read `row.owner_ref`+`row.slug`), feed `"{owner_ref}/{slug}"` through `parse_context_ref` + `resolve_context_ref`, assert it returns the same `row.id`.

- [ ] **Step 2: Run to verify they fail**
Run: `cargo nextest run -p temper-core decorated && cargo nextest run -p temper-api --features test-db --test list_context_ref_test`
Expected: FAIL.

- [ ] **Step 3: Add the helper** (context_ref.rs):
```rust
/// Build the decorated context ref for display/round-trip. `owner_addressable`
/// is the bare handle (profiles) or team-slug (teams), without a sigil.
pub fn decorated_context_ref(owner_table: &str, owner_addressable: &str, context_slug: &str) -> String {
    let sigil = if owner_table == "kb_teams" { '+' } else { '@' };
    format!("{sigil}{owner_addressable}/{context_slug}")
}
```

- [ ] **Step 4: Surface `slug` + `owner_ref` on context rows**

`ContextRow`/`ContextRowWithCounts` gain `pub slug: String` and `pub owner_ref: String` (the already-sigil'd owner, e.g. `@j-cole-taylor` / `+tasker-systems`). In `context_service::create` and `list`, select `c.slug` and compute `owner_ref` via a JOIN+CASE:
```sql
CASE c.owner_table
  WHEN 'kb_teams'   THEN '+' || (SELECT slug   FROM kb_teams    WHERE id = c.owner_id)
  ELSE                   '@' || (SELECT handle FROM kb_profiles WHERE id = c.owner_id)
END AS "owner_ref!"
```
Keep `query_as!` macro-checked. (The CLI/UI build the full `ref` as `{owner_ref}/{slug}`.)

- [ ] **Step 5: Surface home-context decoration on resource/search rows**

In the `readback` producers for `ResourceRow` and `unified_search`, JOIN the home context → its owner to surface `context_slug` and a `context_owner_ref` (same CASE as Step 4 on the home context's owner). Add `pub context_slug: String` + `pub context_owner_ref: String` to `ResourceRow`; add `pub context_slug: Option<String>` + `pub context_owner_ref: Option<String>` to `UnifiedSearchResultRow` (reuse the existing `context` display field for the name).

- [ ] **Step 6: Inject the decorated ref at presentation**

CLI: in `commands/resource.rs` list/show/search output (alongside the existing `ref` injection at :62), insert `"context_ref" = "{context_owner_ref}/{context_slug}"`. For the contexts listing output path, insert `"ref" = "{owner_ref}/{slug}"`. (UI computes its own in Task 8.)

- [ ] **Step 7: Regenerate TS types**
Run: `cargo make generate-ts-types`
Expected: `context.ts`, `resource.ts`, `search.ts` carry `slug`/`owner_ref`/`context_slug`/`context_owner_ref`.

- [ ] **Step 8: Prepare caches, run tests**
Run: `cargo sqlx prepare --workspace -- --all-features && cargo make prepare-api && cargo nextest run -p temper-core decorated && cargo nextest run -p temper-api --features test-db --test list_context_ref_test && cargo nextest run --workspace`
Expected: PASS.

- [ ] **Step 9: Clippy + commit**
```bash
SQLX_OFFLINE=true cargo clippy --all-targets --all-features -- -D warnings
git add -A && git commit -m "feat: surface context decoration on rows; inject ref at presentation"
```

---

## Task 8: temper-ui cutover

**Files:**
- Modify: `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/+page.server.ts` (send `context_ref` = `${owner}/${context}` instead of `context_name`)
- Modify: `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/graph/+page.server.ts`
- Modify: `packages/temper-ui/src/lib/components/CommandPalette.svelte` (use `row.context_ref` when present for nav)
- Modify: create flow to send `context_ref` (drop `kb_context_id`)
- Verify: generated types under `packages/temper-ui/src/lib/types/generated/` reflect Task 7's regen

**Interfaces:** Consumes the regenerated TS types (`context_ref`, `ref`).

- [ ] **Step 1: Update request-building** — `params.set('context_ref', `${routeParams.owner}/${routeParams.context}`)`. The route `[owner]` is already `@me`/`@handle`/`+team`; `[context]` is the slug → the concatenation is a valid decorated ref. Graph endpoint: send `context_ref` likewise (and update `/api/graph/subgraph` server-side resolution if it still reads `context`+`owner` separately — confirm during impl).
- [ ] **Step 2: Create flow** — send `context_ref` (the selected context's `ref` field) instead of `kb_context_id`.
- [ ] **Step 3: Run UI checks**
Run: `cd packages/temper-ui && bun run check && bun run build`
Expected: PASS (svelte-check clean, build OK).
- [ ] **Step 4: Commit**
```bash
git add -A && git commit -m "feat(ui): address contexts by ref (@owner/slug)"
```

> Note: if `/api/graph/subgraph` resolves owner+context by name server-side, fold its conversion into this task (resolve a `context_ref` query param via `resolve_context_ref`) and add the matching handler change under `crates/temper-api/src/handlers/`.

---

## Task 9: Skill regen + docs

**Files:**
- Modify: `crates/temper-cli/skill-content/reference.md` (every `--context <ctx>` / `<context>` → decorated form; the `require_context` error doc)
- Modify: `agent-skills/SKILL.md` (`## Contexts` examples, "On Task Start/Resume/Session Start" `--context` usages, the session-start invocation → `@me/temper`)
- Modify: `crates/temper-cli/src/commands/resource.rs` `require_context` **error text** → e.g. `no context specified — use --context <ref> (e.g. @me/temper or +team/general)`
- Modify (if needed): `crates/temper-cli/src/templates.rs` if any context example is baked into the Rust template
- Test: update `crates/temper-cli/tests/skill_test.rs` expectations for regenerated content

- [ ] **Step 1: Update the error text** (`commands/resource.rs` `require_context`).
- [ ] **Step 2: Rewrite skill sources** — replace bare `--context <ctx>` with decorated examples across `reference.md` + `agent-skills/SKILL.md`. Show both `@me/<slug>` and `+team/<slug>` forms; note bare name is rejected.
- [ ] **Step 3: Update `skill_test.rs`** to match the new generated content.
- [ ] **Step 4: Run skill tests + regen**
Run: `cargo nextest run -p temper-cli skill` then `temper skill install` (regenerates `~/.claude/skills/temper`, re-stamps config-hash).
Expected: tests PASS; install reports changed files.
- [ ] **Step 5: Clippy + commit**
```bash
SQLX_OFFLINE=true cargo clippy --all-targets --all-features -- -D warnings
git add -A && git commit -m "docs(skill): address contexts by decorated ref; reject bare name"
```

---

## Task 10: Cleanup, e2e, final verification

**Files:**
- Modify: `crates/temper-api/src/services/context_service.rs` — remove `resolve_by_name` (now caller-less after Task 3) and the stale comment at `handlers/resources.rs:161`
- Optional: fold in the deferred Beat-2 graph-search e2e (vault task 019f05b1) — a non-ignored `/api/search` graph test now that context + graph are both wired
- Verify: full suite

- [ ] **Step 1: Remove `resolve_by_name`** and confirm no references remain:
Run: `grep -rn "resolve_by_name" crates/ tests/` → expect no hits.
- [ ] **Step 2: Confirm `kb_context_id` fully gone from the list wire** (spec §7): `grep -rn "kb_context_id" crates/ packages/` → only legitimate internal uses (e.g. `ResourceRow.kb_context_id` display field) remain; the **params** field is gone.
- [ ] **Step 3: (Optional) Beat-2 graph-search e2e** — rewire one `#[ignore]`d test in `tests/e2e/tests/graph_search_test.rs` per vault task 019f05b1: create two resources + assert an edge via the relationship API, `/api/search` with `graph_expand` on + a `context_ref`, assert the neighbor surfaces with non-zero `graph_score` and context scoping holds.
- [ ] **Step 4: Full verification**
Run:
```bash
cargo make check
cargo make test-e2e
cargo nextest run --workspace
cd packages/temper-ui && bun run check
```
Expected: all green. Capture output as evidence.
- [ ] **Step 5: Commit**
```bash
git add -A && git commit -m "chore: remove resolve_by_name; final context-ref arc verification"
```

---

## Self-review

**Spec coverage:**
- §3.1 parser → Task 1. §3.2 resolver → Task 2. §3.3 wire rename (ingest/list/search) → Tasks 3/4/5; move → Task 6; `kb_context_id` drop → Task 4 + verified Task 10. §3.4 outbound ref → Task 7. §3.5 search lit → Task 5. §3.6 per-surface (mcp/cli/ui/skill) → Tasks 3–9. §5 tests → each task's TDD + Task 10. Decision 6 (one resolver; remove `resolve_by_name`) → Task 10. No data migration (Decision 5) → no migration task, asserted in constraints.
- Open question "error taxonomy" → resolver returns NotFound (Id/Handle/profile miss) vs Forbidden (team non-member), encoded in Task 2.
- Open question "`kb_context_id` removal blast radius" → Task 4 removes the params field; Task 10 Step 2 greps to confirm only the display field on `ResourceRow` remains.
- Open question "baked skill examples" → Task 9 Step covers `templates.rs` if any example is baked.

**Placeholder scan:** Task 2 Step 1 and Task 8 Step 1 carry pseudocode shapes rather than full bodies — intentional, because both must be written against existing harnesses (`crates/temper-api/tests/common/fixtures.rs`, the UI route files) the implementer reads in-task; each lists exact files to mirror and exact assertions. All production code (parser, resolver, helper, SQL predicate changes, boundary resolution) is shown in full.

**Type consistency:** `ContextRef`/`ContextOwnerRef`/`parse_context_ref`/`resolve_context_ref`/`decorated_context_ref` names and signatures are consistent across Tasks 1→7. `context_ref` is the inbound field name everywhere (ingest/list/search); `CreateResource.context`/`FileMove.context_to` carry `ContextId` post-resolution. Outbound, mirroring resources' `ref`: rows carry **raw ingredients** (`slug`/`owner_ref` on contexts; `context_slug`/`context_owner_ref` on resource & search rows) and the decorated `ref`/`context_ref` is **injected at presentation** (CLI output map at `commands/resource.rs:62`; UI `decoratedRef` in `ref.ts`) — never a wire/DB struct field, so no Rust raw-identifier (`r#ref`) is introduced.
