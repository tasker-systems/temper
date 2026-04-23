# Cloud-First Routing & Mode Collapse — Design Spec

**Date:** 2026-04-22
**Context:** `temper`
**Mode:** build
**Effort:** medium (single-to-two sessions)
**Branch:** `jct/temper-cloud-mode-portable-memory`

**Related work:**
- Reshapes Unit B.2 from `docs/superpowers/specs/2026-04-18-cloud-mode-and-portable-memory-design.md`
- Supersedes Tasks 10–17 of `docs/superpowers/plans/2026-04-19-unit-b-2-cloud-mode-dispatch.md`. Tasks 1–9 stand (Part 1 auth foundation, Part 2 CLI surface, Task 9 guard test already shipped on this branch).
- Builds on Unit A primitives (`push_one_resource` / `pull_one_resource`, manifest-optional)

---

## Problem

The Unit B.2 plan shaped cloud-mode dispatch as a parallel code path: a sibling `resource_cloud` module with doctype-agnostic rendering, forked `list` / `show` / `create` / `update` entry points branching on `VaultState`. When we went to execute Part 3 (Tasks 10–17), grep-verification against current code surfaced significant drift:

- Server `ResourceListParams` has no `stage` / `goal` / `status` filters — those are local-mode frontmatter reads.
- `ResourceRow` field names don't match what the plan assumed (actual: `context_name`, `doc_type_name`, `updated`, `slug: Option<String>`; no `goal_slug`, `status`, or `goal`).
- Local-mode `list` uses per-doctype `col_registry::display_columns` + `TableRenderer`; plan's hardcoded eight-column table would diverge user-visible output between branches.
- E2E harness invokes `app.client.*()` in-process, not CLI subprocess; plan's `e2e_cli_cmd` helper doesn't exist.

More importantly, the drift exposed a deeper mismodel: treating cloud-mode as a separate dispatch hierarchy alongside local-mode. That framing splits the CLI in two, produces behavioral inconsistency between the branches (sort order, filter shape, output format), and misses the actual concern the original Unit raised — that **local `list` / `show` don't see remote resources at all**, so a cloud session's work is invisible to a local session until the user manually syncs.

Two concrete bugs under the current split:

1. **Missing remote rows.** `temper list --type session --context temper` returns only files present in the local vault. A session written from a cloud Claude Code run is not there until `temper sync run` (or `temper pull`) lands it locally.
2. **Wrong sort order.** Local-mode `list` appears to return sessions earliest-first; server returns `updated DESC`. Users see different orderings between local and remote reads.

Both fall out naturally once we route reads through the server.

## The Reframe

**Manifest + three-way merge isn't local-first legacy scaffolding. It's concurrency control.**

The cloud-first flip is about **routing** — where does the authoritative data live, which direction does it flow. Concurrency control is a distinct layer — how do we resolve simultaneous edits from multiple sessions / machines / teammates. The two are orthogonal.

Under the new model:

- **Routing**: always server-first. Reads hit the API. Writes go through the API. The local vault is a read-optimized cache of the server's truth, with the manifest as its cache-state ledger.
- **Concurrency control**: existing `push_one_resource` + manifest + `similar`-based three-way merge, **unchanged**. Kicks in when a write finds a diverged remote (another session, another user). Agents and humans reconcile through the same CLI ergonomics (`temper pull` to refresh, `temper push` to retry). No git-rebase-cosplay.

This collapses the model from "two dispatch branches" to "one flow, expressed through a file-I/O toggle."

## Modes

Two modes, distinguished by **presence or absence of a local vault**:

**Cloud mode** — ephemeral. Triggered by `TEMPER_VAULT_STATE=cloud`. No vault directory, no manifest, no provisional-ids. `TEMPER_TOKEN` required (already enforced by Task 9's guard on `runtime::resolve_token_store`). Every command is a pure API pass-through: render response to stdout, exit. `sync run` errors with a clear redirect. `show` / `list` never touch disk. `create` / `update` POST/PUT and render the response without persisting anything locally. On a 409 from concurrent writes, surface the error; the ephemeral session doesn't own conflict resolution.

**Local mode** — normal. The default. A local vault directory exists. Routing is still server-first — reads hit the API, writes go through `push_one_resource` — but the vault serves as a read cache and the manifest serves as the cache-state ledger. Writes that diverge from the server's current state fall into the existing three-way-merge path. `sync run` keeps today's bulk-refresh behavior. **Offline** is a degraded sub-state of Local mode, not a third mode: network failure surfaces as a warning, reads fall back to local cache, and writes either use provisional-ids (for `create`) or warn dirty-without-push (for `update`).

## Read Path

- **`list`** — API-only. Always calls `client.resources().list(...)`. Does not touch the local vault. Results flow through the existing `col_registry::display_columns(doc_type)` + `TableRenderer` pipeline so table output matches today's per-doctype column shape. Server sort (`ORDER BY updated DESC`) is authoritative. Offline: warn + render an in-memory projection of the local manifest's known rows (sorted `updated DESC`) as a best-effort substitute.
- **`show`** — API-first in Local mode via a **three-tier freshness ladder**:
  1. **Debounce**: if a local file exists and `now - mtime < DEBOUNCE_SECONDS` (default 30s, configurable), render local. Skip API entirely.
  2. **Hash-verify**: otherwise, call a cheap version-check primitive (use the one `sync status` already uses — a metadata-only `GET /resources/{id}` returning `updated` + hash triplet, no body). If local matches, `touch` the mtime to now and render local. If diverged, fall to tier 3.
  3. **Full fetch**: `GET /resources/{id}/content`. Overwrite local, render. Agents sniffing `.md` files directly see canonical frontmatter.
  Cloud mode: no ladder. Straight `GET /resources/{id}/content`, render, no disk write. Offline Local: freeze at tier 1 with warning.
- **`search`** — already API-only today. No change.

## Write Path

- **`create`** — renders a template client-side with `temper-provisional-id`, writes to disk, then routes through `push_one_resource` with `manifest: Some(&mut manifest)` and `PushTarget::Path`. Push receives canonical `temper-id`, rewrites provisional → canonical in the local file, updates the manifest entry. Cloud mode: skip template-to-disk; POST directly via a thin helper that reads body (from stdin for `session` / `task` where the existing CLI uses stdin; from a minimal in-memory template for auto-generated doctypes); render the created resource's canonical id to stdout. Offline Local: provisional-id stays until the next `push`.
- **`update`** — reads the current local file, applies CLI arg-driven frontmatter mutations, writes the file, then routes through `push_one_resource` with `manifest: Some` and `PushTarget::Id`. Conflict path: manifest-mediated three-way merge via `similar` (unchanged). Cloud mode: read-modify-push doesn't apply — there's no local file. Cloud-mode `update` accepts frontmatter args only (stdin body support is in the Deferred list). It fetches the current server state, applies the frontmatter mutation in memory, PUTs. On 409: surface error.
- **Update contract** (documented in CLI help + skill guidance): `temper resource update <slug> --<field>` pushes the whole file, not just the frontmatter. Manual body edits made via editor or shell redirection (`>`, `>>`) must happen **before** `update` is invoked. Body-only changes can continue to use `temper push <slug>` directly.
- **`delete`** — DELETE to the API, remove the local file, remove the manifest entry. Cloud mode: DELETE only.

## Mode Implementation

`VaultState::{Local, Cloud}` stays a first-class enum, matched at each divergence site via exhaustive `match` (consistent with `feedback_no_stringly_typed_match`):

```rust
match VaultState::from_env() {
    VaultState::Cloud => { /* API-only path */ }
    VaultState::Local => { /* cloud-first with cache + manifest */ }
}
```

**Not** parallel dispatch modules. **Not** a magic vault abstraction that silently no-ops on cloud. Match branches live inside the existing command functions (`commands/resource.rs::show`, `commands/resource.rs::list`, etc.) where the divergence is specific and small. Shared work (the API call itself) happens once before the match; the match handles only the Local-specific cache and manifest steps.

**Ergonomic pattern**: where both arms share work, do the shared call first, then match on the Local-only disk/manifest steps:

```rust
let content = client.resources().get_content(id).await?;
match VaultState::from_env() {
    VaultState::Cloud => {}  // no cache warm
    VaultState::Local => cache::write(path, &content)?,
}
output::plain(content);
```

Where the arms genuinely diverge (e.g., `update` — Cloud PUTs direct, Local goes through `push_one_resource`), match owns the whole flow.

No `_ => default` fallback arms. Every `VaultState` variant is named.

## Auth (unchanged dependency)

Auth foundation + CLI surface shipped in Parts 1 (Tasks 1–4) and 2 (Tasks 5–8) last session. This design depends on that surface but does not modify it:

- **Cloud session bootstrap**: `TEMPER_TOKEN` injected as an env var (via `docs/guides/cloud-agents.md` setup). `MemoryTokenStore::from_env_required()` reads it. Task 9's guard ensures a missing `TEMPER_TOKEN` in cloud mode errors clearly rather than falling back to disk.
- **Local session bootstrap**: `temper auth login` runs the Auth0 device flow (unchanged). Tokens persist in `DiskTokenStore`.
- **Producing `TEMPER_TOKEN` for a cloud session**: `temper auth export-token` on the user's local machine refreshes the access token from `DiskTokenStore` and prints the JWT to stdout with a security warning on stderr. The user copies the JWT into the cloud environment's secret store or env var injection step. Refuses to run when `TEMPER_VAULT_STATE=cloud` (no disk store available).
- **Accepting an externally-issued JWT**: `temper auth token` reads a JWT from stdin (no positional args) and saves it to the local `DiskTokenStore`.

## Scope

**In scope for this Unit:**

1. **Read path rewrite**: `list`, `show`, `search` always route to the API. `show` implements the three-tier freshness ladder in Local mode; `list` uses the existing per-doctype column pipeline with server-side results; `search` verifies already-API-only and is documented.
2. **Write path consistency**: confirm `resource create` and `resource update` both route through `push_one_resource` end-to-end in Local mode. Wire up any gaps. Surface the "update implies push" contract in CLI help text and the `temper` skill's workflow files.
3. **Mode implementation**: `match VaultState` at each divergence site. No `resource_cloud.rs` sibling module. Cloud-mode code lives inline in existing command modules where the match arm belongs.
4. **Sync behavior**: `sync run` in Cloud mode errors with a clear redirect message. Local mode is unchanged.
5. **Sort-order consistency**: API-first list returns server's `ORDER BY updated DESC`. Offline fallback sorts local results to match. Users see one ordering regardless of mode or connectivity.
6. **Debounce + hash-check plumbing**: `show`'s three-tier ladder requires a cheap version-check primitive. If `sync status` already exposes this internally, reuse it. If not, add a thin helper that returns `(updated, body_hash)` without fetching the body.
7. **Acceptance tests**: e2e tests that exercise the in-process harness for each of the read and write paths in both modes.

**Deferred to follow-up Units (named so they're not lost):**

- `temper update --body-append` / `--body-overwrite` stdin ergonomics for agent-friendly body mutation (parity with `create`'s existing stdin-body flow).
- Re-auth prompts on 401 / expired-token responses.
- Manifest UX rethink (conflict-marker surfacing, reconcile-in-CLI affordances). The three-way-merge machinery is kept intact in this Unit; its ergonomics are a separate conversation.
- Team-context / multi-user concurrency polish (distinct-author conflict handling, per-resource lock hints).
- Programmatic token issuance (managed-agent platform handing a cloud session a scoped short-lived token without copy-paste).

## Acceptance Criteria

- **Sort order** (bug fix): `temper list --type session --context temper` in Local mode returns sessions newest-first (matching server `ORDER BY updated DESC`).
- **Remote visibility** (bug fix): `temper list --type X --context Y` in Local mode returns server rows, including resources created by other sessions that haven't been manually pulled.
- **Show debounce**: after a `temper show <slug>` call writes or touches the local file, a second `temper show <slug>` within the debounce window makes zero API calls (verified via test harness request count).
- **Show hash-verify**: `temper show <slug>` after the debounce window, when the server's version matches local, issues the cheap metadata check but does not fetch the body content.
- **Show cache warm**: `temper show <slug>` when the server has a newer version overwrites the local file with the server version and renders it.
- **Update round-trip**: `temper resource update <slug> --stage done` on a file with manual body edits pushes both frontmatter and body to the server in one operation.
- **Cloud create**: a cloud session (`TEMPER_TOKEN` + `TEMPER_VAULT_STATE=cloud`) running `temper resource create --type session --context temper --title "test"` returns the canonical `temper-id` on stdout, writes nothing to disk, and creates no `auth.json`.
- **Cloud cross-session visibility**: `temper show <id>` from a second cloud session (or from a Local session) retrieves the content the first cloud session created.
- **Sync redirect**: `temper sync run` in cloud mode errors with the redirect message; in local mode behaves as today.
- **Mode mismatch hard-fail**: cloud mode with `TEMPER_TOKEN` unset errors clearly (Task 9 — already locked in).
- **No parallel dispatch module**: no file `crates/temper-cli/src/commands/resource_cloud.rs` exists. `grep -r "resource_cloud" crates/temper-cli/` returns nothing.

## Migration Notes

- **Plan file** `docs/superpowers/plans/2026-04-19-unit-b-2-cloud-mode-dispatch.md`: Tasks 1–9 stand as shipped/verified. Tasks 10–17 are obsolete; the new plan supersedes them. The original plan file can stay in place as a historical artifact; the new plan will link back.
- **No new `resource_cloud` module.** All dispatch lives in the existing command modules.
- **`VaultState::Cloud` enum variant stays.** What changes is how we *use* it — match at divergence sites, not as a dispatch-hierarchy root.
- **`temper-client` resource surface is sufficient.** Grep-verified: `client.resources().list(&ResourceListParams)` and `client.resources().get_content(id)` etc. exist and cover the needed read paths.
- **Existing `push_one_resource` / `pull_one_resource` primitives are sufficient.** No changes to Unit A's shipped API.

## Out-of-Scope (repeated for clarity)

- Any change to the manifest format or three-way-merge algorithm.
- Any change to the auth foundation or CLI auth commands.
- Any new doctype, schema change, or vault layout change.
- Any change to MCP surface (parallel track per the original spec).
- Any change to the server API shape beyond (possibly) exposing the existing `sync status` hash-check as a per-resource primitive if one isn't already callable that cheaply.
