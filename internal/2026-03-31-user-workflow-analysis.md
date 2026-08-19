# User Workflow Analysis — 2026-03-31

Traces the complete lifecycle from `temper init` through `temper import`, `temper sync`, and `temper search`, identifying gaps between what the code does today and what the tickets expect.

---

## 1. Deployment Architecture — Two API Surfaces

The Vercel routing in `vercel.json` creates a critical split:

```
{ "handle": "filesystem" }       ← matches files in api/ first
{ "src": "/(.*)", "dest": "/api/axum" }  ← everything else → Rust Axum
```

This means:

- **TypeScript handlers** (matched by filesystem): `/api/ingest`, `/api/ingest/[id]`, `/api/upload`, `/api/sync/status`, `/api/sync/complete`, `/api/auth/cli-callback`, `/api/workflows/*`
- **Rust Axum** (catch-all): `/api/resources`, `/api/resources/{id}`, `/api/resources/{id}/content`, `/api/profile`, `/api/profile/auth-links`, `/api/events`, `/api/search`, `/api/health`

These two surfaces have **different auth-to-profile resolution**, which is the most critical gap identified.

---

## 2. Auth-to-Profile Lifecycle — The Gap

### Axum path (Rust): auto-provisions profiles
`middleware/auth.rs::require_auth()` (step 5) calls `profile_service::resolve_from_claims()`, which:
1. Looks up `kb_profile_auth_links` by `(provider, external_user_id)`
2. If not found, attempts email reconciliation with verified emails
3. If still not found, **creates a new profile + auth link**

This means any first request to an Axum-routed endpoint auto-provisions the user.

### TypeScript path: requires pre-existing profile
`middleware.ts::authenticateRequest()` calls `getProfileId()` from `ingest.ts`, which:
1. Looks up `kb_profiles` via `kb_profile_auth_links` join on `claims.sub`
2. If no match, returns `null` → handler returns **404 "Profile not found"**

There is **no auto-provisioning** in the TypeScript path.

### Impact on real user workflows

A new user who runs `temper auth login` → `temper import <file>` will hit:

1. `temper import` → CLI calls `ingest_file()` → client POSTs to `/api/ingest`
2. `/api/ingest` is a TypeScript endpoint (filesystem match)
3. TypeScript `getProfileId()` finds no profile → **404**

The user gets "Profile not found" with no explanation. The import fails.

### Fix required before I5e can work

**Option A (minimal):** Before the first ingest/sync call, have the CLI make a dummy request to an Axum endpoint (e.g., `GET /api/profile`) which will auto-provision the profile via `resolve_from_claims()`. This is a one-line add to the client's auth flow.

**Option B (correct):** Port the profile auto-provisioning logic from `profile_service::resolve_from_claims()` into the TypeScript `getProfileId()` (or a new `resolveProfile()` function). This ensures any entry point works.

**Option C (I5f scope):** I5f already identifies that "prior agents build their own visibility checks rather than using the sql functions." Extending this to unify the auth-to-profile path is natural scope for that ticket.

**Recommendation:** Option A as an immediate unblock for I5e, Option B as part of I5f.

---

## 3. `temper import` — Full Code Path Trace

### What happens today

```
CLI: commands/import_cmd.rs::run()
  → detect file vs UUID (file path)
  → require --context
  → actions/runtime::build_runtime_and_client()
  → actions/ingest::ingest_file(client, path, context, doc_type)
      → temper_embed::extract::extract_to_markdown(path)        [local kreuzberg]
      → actions/ingest::build_ingest_request(content, mime, path, ctx, dt)
          → computes content_hash, builds metadata JSON
          → sets context_name, doc_type_name (name-based, not UUID)
          → sets kb_context_id = Uuid::nil(), kb_doc_type_id = Uuid::nil()
      → client.ingest().create(&request)
          → POST /api/ingest (multipart: metadata JSON + content string)
          → [TypeScript] parse metadata, authenticate, getProfileId(†)
          → resolveContextId (auto-creates context if new)
          → resolveDocTypeId (returns null if unknown → error)
          → findByContentHash (dedup check)
          → insertResource (INSERT into kb_resources)
          → processIngest (Vercel Workflow: chunk → embed → store)
          → returns ResourceRecord
  → actions/ingest::write_vault_file_and_register()
      → builds vault path: {vault_root}/{context}/{doc_type}/{uuid}.md
      → generates frontmatter with temper-id, title, context, doc_type
      → writes file
      → loads manifest, inserts entry with content_hash, saves manifest
  → output success
```

(†) This is where the profile gap bites.

### Specific issues in the import flow

**Issue 1: Profile not found (critical, covered above)**

**Issue 2: `resource_mode` not set during ingest**

The I6a ticket specifies that `kb_resources` has a `resource_mode` column (`'added'` | `'imported'`). The `ingest.ts::insertResource()` function does NOT set this column — it inserts with no `resource_mode` value, so the column will use its default (if any) or be null. The `sync_diff_for_device` SQL function filters `WHERE r.resource_mode = 'imported'`, meaning resources created via ingest **will not appear in sync diffs** unless `resource_mode` is explicitly set.

Let me verify:

The consolidated schema shows:
```sql
resource_mode VARCHAR(16) DEFAULT 'added' CHECK (resource_mode IN ('added', 'imported'))
```

So `ingest.ts::insertResource()` will default to `'added'`. This is correct for `temper add`, but **wrong for `temper import`** which should set `resource_mode = 'imported'`. The CLI `ingest_file()` function builds the `IngestRequest` but there's no `resource_mode` field on `IngestRequest` and no way to pass it through the multipart metadata.

**Fix needed:** Add `resource_mode` to `IngestMetadata` in TypeScript and to `IngestRequest` in Rust core. CLI `import_cmd.rs` should set it to `'imported'`.

**Issue 3: doc_type_name resolution can fail silently**

If the user provides a doc_type that doesn't exist in `kb_doc_types`, `resolveDocTypeId()` returns null and `insertResource()` throws "Unknown doc_type_name." The seed migration only creates a limited set of doc types. Users will need either a) the CLI to validate against known types, or b) the server to auto-create doc types like it does contexts.

---

## 4. `temper sync` — Full Code Path Trace

### What happens today

```
CLI: commands/sync_cmd.rs::run()
  → config::resolve_vault() → vault root
  → runtime::require_device_id() → from auth.json
  → manifest_io::load_manifest()
  → runtime::build_runtime_and_client()
  → actions/sync::sync_orchestration(client, manifest, vault_root, contexts)
      1. rehash_manifest — SHA-256 each vault file, mark LocalModified if changed
      2. build_status_request — group manifest entries by context → SyncStatusRequest
         → POST /api/sync/status [TypeScript endpoint]
         → authenticateRequest() → getProfileId(†) → computeSyncDiff()
         → sync_diff_for_device() SQL function
         → returns SyncDiffResult
      3. push_resource — for each to_push:
         → New resource: extract context/doc_type from path, ingest_file()
         → Existing: strip frontmatter, PUT /api/ingest/:id
      4. pull_resource — for each to_pull:
         → GET /api/resources/:id [Axum endpoint!]
         → GET /api/resources/:id/content [Axum endpoint!]
         → write_vault_file_and_register()
      5. Handle conflicts — mark Conflict in manifest
      6. Handle removed — delete local file, remove manifest entry
      7. complete — POST /api/sync/complete [TypeScript endpoint]
      8. Update manifest.last_sync
  → manifest_io::save_manifest()
```

### Cross-surface routing during sync

A single sync operation hits **both** API surfaces:
- `/api/sync/status` → TypeScript (needs profile to exist)
- `/api/resources/:id` → Axum (auto-provisions profile)
- `/api/resources/:id/content` → Axum
- `/api/ingest` → TypeScript (needs profile to exist)
- `/api/sync/complete` → TypeScript (needs profile to exist)

If the user hasn't hit an Axum endpoint first, the sync/status call will 404 before the pull (which goes to Axum) has a chance to auto-provision.

### Sync-specific issues

**Issue 4: Context filter comes from CLI args, but I5e wants it from config.toml**

The `temper sync run --context temper` flag works. But I5e's proposed config (`sync.subscriptions.contexts = ["temper", "storyteller"]`) is not yet parsed by `temper-cli`. The `SyncConfig` in `temper-core/src/types/config.rs` has a different shape (`subscriptions: Vec<SyncSubscription>` with per-subscription `context`, `team`, `doc_types`, `merge`). Neither shape matches the I5e proposal.

This needs reconciliation before I5e is complete: either update `CloudConfig` parsing to support the simplified config, or adjust I5e's design to match the existing core type.

**Issue 5: Sync push calls ingest which defaults resource_mode = 'added'**

When sync pushes a new local-only resource, it calls `ingest_file()` which POSTs to `/api/ingest`. The server creates the resource with `resource_mode = 'added'` (default). But sync should only operate on imported resources. This means a pushed resource won't appear in future sync diffs (since `sync_diff_for_device` filters on `resource_mode = 'imported'`).

The push effectively becomes a one-shot upload with no round-trip capability.

---

## 5. `temper search` — Code Path

```
CLI: commands/search_cmd.rs::run()
  → resolve_vault, require_device_id, load_manifest
  → actions/search::embed_query(query)  [local temper-embed, feature-gated]
  → runtime::with_client → search::query_api(embedding, context, doc_type, limit)
      → POST /api/search [Axum endpoint]
      → search_service::search() — pgvector cosine similarity with resources_visible_to()
  → enrich_results(results, manifest) — mark local/remote, attach vault_path
  → format output
```

This path goes entirely through Axum, so profile auto-provisioning works. The search implementation is complete and functional, assuming content has been indexed (which requires ingest → processIngest workflow to have run).

---

## 6. Config Unification — I5e Gap

I5e proposes a **single** `~/.config/temper/config.toml` that combines vault, sync, CLI, and auth config. Today:

- **temper-cli** reads `{vault_root}/temper.toml` for vault config (via `config::load()`)
- **temper-client** reads `~/.config/temper/config.toml` for auth + cloud config (via `config::load_cloud_config()`)
- These are **separate files with separate schemas**

I5e wants them merged into one file. This requires:
1. Updating `temper-client::config::CloudConfig` to include vault/sync/cli sections
2. Updating `temper-cli::config::load()` to read from `~/.config/temper/config.toml` instead of `{vault_root}/temper.toml`
3. Updating `temper init` to write to `{vault_root}/.temper/temper.toml` (the new location per I5e)
4. Deprecating `{vault_root}/temper.toml` at the root

This is structural refactoring that should be completed before I5g (migrating the knowledge base), since I5g needs the new config to work.

---

## 7. Upcoming Ticket Foundation Assessment

### I5e — Local KB Restructure (in-progress)
**Foundation status: partially ready, critical gaps**
- ✅ Vault path convention `{vault_root}/{context}/{doc_type}/{slug}.md` — used by `ingest::build_vault_path()`
- ✅ Manifest in `.temper/manifest.json` — code exists
- ❌ Config unification (vault + auth in one file) — not implemented
- ❌ Profile auto-provisioning on first ingest — blocked (§2)
- ❌ `resource_mode = 'imported'` propagation — missing (§3)
- ❌ `temper init` writing to `.temper/` — current code writes `temper.toml` at vault root

### I5f — Refactor Axum Handlers + Context CRUD (backlog)
**Foundation status: ready**
- ✅ SQL functions (`resources_visible_to`, `can_modify_resource`, `contexts_visible_to`) exist in schema
- ✅ Handler → service → SQL layering in place
- ✅ The ticket correctly identifies that services have hand-rolled visibility instead of using SQL functions
- The `resource_service.rs` `list_visible` and `get_visible` do use `resources_visible_to`, but `update` and `delete` use raw `can_modify_resource` checks which is actually correct. The context CRUD endpoints don't exist yet.

### I5g — Migrating the Knowledge Base (backlog)
**Foundation status: depends entirely on I5e/I5f**
- Correctly identifies that frontmatter differs from new format
- Correctly proposes a throwaway importer script
- Cannot begin until I5e's config unification and import flow are working

### I6b — Auto-Merge & Workflow Integration (backlog)
**Foundation status: solid**
- ✅ I6a sync infrastructure is complete (Rust + TypeScript)
- ✅ `ManifestEntryState::Conflict` exists in core types
- ✅ `rehash_manifest()` exists in actions/sync.rs
- ❌ No `ManifestEntryState::Merged` variant yet (I6b scope, but core type needs adding)
- ❌ `--auto-sync` flag not yet on any workflow command
- ❌ `sync.contexts` config not yet parsed (same config unification gap as I5e)
- The merge-notice injection and event-UUID idempotency are well-specified and have no missing foundations.

### I6c — Team Sync & Manual Resolution (backlog)
**Foundation status: mostly ready**
- ✅ `resources_visible_to(profile_id, team_id)` already supports team scoping
- ✅ `sync_diff_for_device` uses `resources_visible_to` for visibility
- ✅ `SyncResolveRequest` and `ResolutionType` types exist in core
- ❌ No `POST /api/sync/resolve` endpoint yet (I6c scope)
- ❌ Team CRUD endpoints don't exist (I7 dependency, but sync can work without them using direct DB-seeded teams)
- ❌ `VaultConfig` (server-side) has subscription/per-device structures, but no CLI code reads these from the server

### I7 — Team Management & Resource Sharing (backlog)
**Foundation status: schema ready, no code**
- ✅ All tables exist: `kb_teams`, `kb_team_members`, `kb_team_resources`, `kb_team_invitations`, `kb_transfers`
- ✅ SQL functions: `can_manage_team()`, `resources_visible_to(p_team_id)`
- ✅ Core types: `Team`, `TeamMember`, `TeamRole`, `TeamInvitation`, `InvitationStatus`, `ResourceTransfer`, `TransferStatus`, `AccessLevel`, `TeamResource`
- ❌ Zero API endpoints for teams
- ❌ Zero CLI commands for teams
- The schema and types are comprehensive and well-thought-out. Implementation is pure greenfield but has a solid base.

### I8 — Upload Pipeline & temper-embed (backlog)
**Foundation status: mostly already done**
- ✅ TypeScript pipeline (I4a/I4b): upload → extract → chunk → embed → store is live
- ✅ temper-embed crate exists with `extract` + `embed` features, kreuzberg integration, bge-base-en-v1.5 via ONNX
- The ticket was re-scoped after I4a/I4b and I5d. Most of I8's original scope has been delivered across those tickets. Remaining scope is batch processing for large sync workloads, which may not be needed until scale demands it.

### I9 — Search Unification & CLI Evolution (backlog)
**Foundation status: partially superseded**
- ✅ `temper search` is already cloud-routed (I5d)
- ✅ Local embedding + cloud query is working
- ❌ Graph/keyword search modes not implemented
- ❌ `temper context` alias doesn't exist
- ❌ The ticket's vision of `temper ticket` / `temper milestone` aliases diverges from what was actually built (the CLI still uses `temper task` / `temper goal`, not `temper ticket` / `temper milestone`)
- This ticket's nomenclature predates I5a's rename from tickets→tasks, milestones→goals. The scope document needs updating to reflect current naming.

### I10 — temper-mcp (backlog)
**Foundation status: ready when I9 stabilizes**
- ✅ `crates/temper-mcp` directory exists (empty scaffold)
- ✅ `temper-client` provides all needed sub-clients
- ✅ Auth credentials in `~/.config/temper/auth.json` are accessible
- The MCP server depends on stable search (I5d ✓) and stable resource API (I5f), both of which are approaching readiness.

---

## 8. Ticket Drift Assessment

### Drifted significantly
- **I9**: Nomenclature (ticket/milestone vs task/goal), search modes (graph/keyword not built), "thin aliases" vision replaced by direct commands. Needs a rewrite of scope to match current reality.
- **I8**: Mostly delivered across I4a/I4b/I5d. Remaining scope is narrow. Could be collapsed into a smaller ticket or closed with notes.

### Partially drifted
- **I5e**: Config unification design doesn't match either existing config schema. The `sync.subscriptions` structure in core types vs. the simplified `sync.subscriptions.contexts = [...]` proposed in I5e need reconciliation.
- **I6b**: `sync.contexts` config support depends on the I5e config unification, creating an implicit dependency not captured in the ticket.

### On track
- **I5f**: Correctly scoped, clear gaps identified, foundations solid.
- **I5g**: Correctly identifies dependencies and approach.
- **I6c**: Well-specified, schema supports it, clear sequencing.
- **I7**: Schema + types ready, greenfield implementation.
- **I10**: Dependencies clear, scaffold exists.

---

## 9. Recommended Immediate Actions

1. **Profile auto-provisioning (blocks I5e):** Add a `GET /api/profile` pre-flight call to `temper-client` auth flow, or port auto-provisioning to TypeScript. Without this, `temper import` will 404 for new users.

2. **resource_mode propagation (blocks sync round-trip):** Add `resource_mode` to `IngestMetadata` (TS) and `IngestRequest` (Rust). CLI import sets `'imported'`, CLI add sets `'added'`. Without this, imported resources won't sync.

3. **Config reconciliation (blocks I5e completion):** Decide on one config shape for `~/.config/temper/config.toml` that satisfies both `temper-cli` vault needs and `temper-client` cloud/auth needs. Update the I5e ticket with the chosen design.

4. **Update I9 scope (housekeeping):** Rewrite to reflect task/goal naming, remove ticket/milestone aliases, note that cloud-routed search is done.
