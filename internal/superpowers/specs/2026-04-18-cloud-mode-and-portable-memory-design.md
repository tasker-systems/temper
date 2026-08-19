# Cloud Mode and Portable Memory — Design Spec

**Date:** 2026-04-18
**Context:** `temper`
**Mode:** research + build (three units of work)
**Effort:** large (multiple sessions)
**Branch (design doc + prototype PR):** `jct/temper-cloud-mode-portable-memory`

**Related work:**
- Builds on the provisional-id system (`docs/superpowers/plans/2026-04-05-provisional-id-system.md`)
- Builds on unified sync hashing (`docs/superpowers/plans/2026-04-11-sync-cycle-unified-hashing.md`)
- Parallel track, not blocking: MCP tool parity for new commands

---

## Problem

Temper's value proposition is session-over-session, task-over-task, goal-over-goal continuity for humans and agents. The local workflow — `temper init`, `temper auth login`, `temper sync run` — is the connective tissue that makes this real. Ephemeral cloud agent sessions (Claude Code on the web, Cursor cloud agents, Devin-shaped runners) break that workflow at three points:

1. **No interactive OAuth.** `temper auth login` runs an Auth0 device authorization PKCE flow that assumes a browser and a human (`crates/temper-client/src/auth.rs`). Cloud sessions have neither.
2. **No persistent disk.** The vault on-disk is the anchor for the manifest and the three-way sync engine (`crates/temper-cli/src/commands/sync_cmd.rs:42-174`). Cloud sessions get fresh containers — nothing persists between runs.
3. **No inherited MCP.** Cloud agents don't inherit MCP connections from the host. Even if they did, MCP alone doesn't give them a file-level editing surface over markdown, which both humans and agents work best with.

The net effect: any work done in a cloud session of any kind within a temper context is effectively lost to memory. This spec closes that gap without forking the mental model of how temper works.

## North Star

A single workflow vocabulary — `create`, `update`, `push`, `pull`, `list`, `show`, `search`, `sync` — that executes across three surfaces:

1. **Local-vault mode** (today): the full manifest-backed three-way sync workflow.
2. **Cloud mode** (new): the same commands, routed straight through the API, with a minimal working directory of only files the session has actively touched. `sync` is disabled with a clear redirect to `push`.
3. **MCP-only mode** (parallel track): the same API surface, exposed as MCP tools, for agents that have no CLI at all.

Skills and agent patterns are written once, in the workflow vocabulary, and work across all three surfaces. The connective tissue — context scoping, provisional-ids, tier hashing, schema validation — is shared code in `temper-core` and `temper-ingest`.

## Principles

1. **A resource is a resource.** No special-case doctypes, no hidden storage paths. Memory participates in sync, graph, FTS, pgvector, MCP like any other resource.
2. **Reuse sync's proven primitives.** Normalize, hash, tier-split, ownership preflight already live in `temper-core` and are invariant across CLI and API. Push and pull become orchestration over a one-element set of the same primitives.
3. **Provisional-id is the create→canonical handoff.** When a cloud-mode `resource create` POSTs, the response carries a canonical `temper-id` that replaces the local `temper-provisional-id`. From that moment on the file is server-authoritative; no manifest tracking is needed.
4. **No partial-manifest state.** The manifest is a three-way-merge ledger. Cloud mode operates manifest-less. `sync` is disabled in cloud mode; `push <id|path>` and `pull <id>` are the primitives.
5. **Client-side template rendering, for now.** Templates stay in `temper-cli/templates/` and render client-side. Server-side template parity is a future consideration, not a prerequisite.

---

## Unit A — Unified Push/Pull Primitives

**First in sequence.** Factor the one-entry orchestration out of sync so push and pull can be first-class commands that reuse the same engine.

### Scope

- Introduce `push_one_resource(client, vault_root, path_or_id, manifest: Option<&mut Manifest>) -> PushResult` in `crates/temper-cli/src/actions/sync.rs` (or a sibling module), factored from the existing `push_resource` loop.
- Introduce `pull_one_resource(client, vault_root, id, manifest: Option<&mut Manifest>) -> PullResult` similarly. The `pull_one_resource` primitive generalizes the current `pull.rs:12-95` (which already bifurcates on manifest presence — this just formalizes the split).
- The `Option<&mut Manifest>` parameter is how these primitives stay useful across modes: local-mode passes `Some(&mut manifest)` and the entry is tracked; cloud-mode passes `None` and the entry is not tracked.
- Make `sync_orchestration` a batched caller of these primitives so the underlying engine is unified in one direction.
- New commands wiring: `temper push <id|path>` and `temper pull <id>` as top-level CLI commands (pull already exists at `crates/temper-cli/src/commands/pull.rs` and becomes a thin wrapper; push is new).

### Code touchpoints

- `crates/temper-cli/src/actions/sync.rs` — extract primitives
- `crates/temper-cli/src/commands/pull.rs` — reduce to primitive wrapper
- `crates/temper-cli/src/commands/push.rs` — new
- `crates/temper-cli/src/commands/mod.rs` — register `push`
- `crates/temper-cli/src/main.rs` (or `cli.rs`) — clap arg wiring

### Acceptance

- `temper push <path>` round-trips a single file identically to how that file would be pushed inside a full `sync run`, with the same hash invariants, same schema validation, same ownership preflight.
- `temper pull <id>` with a manifest present writes to the manifest-resolved vault path and updates the entry (preserves current behavior).
- `temper pull <id>` with no manifest writes to CWD as a snapshot (preserves current "ADDED" behavior at `pull.rs:85-90`).
- Existing `sync run` behavior unchanged — all current sync tests pass without modification.

### Open questions for Unit A

- Should `push <id>` accept a bare id and resolve the path by scanning the manifest, or require `push <path>` when there's no manifest? Likely: both, with id-form requiring a manifest.
- Does `push` surface conflicts the same way `sync run` does, or is it strictly last-writer-wins by default with an opt-in `--check` preflight? Likely: last-writer-wins with `--check` as the preflight flag, because a manifest-less cloud session doesn't have local-state to three-way-merge against.

---

## Unit B — Cloud Mode Plumbing

**Second in sequence.** The env var floor plus the dispatch rewrites that make cloud mode a real operating mode.

### B.1 — Env var floor (prototype PR, this session)

Smallest unit that unlocks temper-in-temper cloud sessions today, without any dispatch changes.

- **`TEMPER_TOKEN` env var** recognized at client construction in `temper-client`. If `auth.json` is missing or the cached token is expired and `TEMPER_TOKEN` is set, build an in-memory `AuthData` by parsing the JWT's `exp`, `sub`, and `iss` claims (mirroring `crates/temper-cli/src/commands/auth.rs:42-97` without touching disk). Never persist to disk — ephemeral sessions must stay ephemeral.
- **`TEMPER_VAULT_STATE` env var** (`cloud` | `local`, defaulting to `local`) recognized in `temper-core::types::config`. Recognition only in this PR — not yet wired to dispatch. Exposes a `VaultState` enum on the resolved `Config`.
- **`TEMPER_PROVIDER` env var** recognized alongside `TEMPER_TOKEN` — which Auth0 provider the token came from (today's hardcoded `auth0` default is fine if unset).
- Unit tests: JWT claim parsing on a synthetic signed token, env var precedence over `auth.json`, config parsing with `VaultState::Cloud`.
- Docs update in `docs/guides/cloud-agents.md` pointing at the env vars and sketching the intended SessionStart hook wiring.

This PR is scoped to ~60–150 LOC and is a precondition for every other part of Unit B.

### B.2 — Dispatch rewrites (subsequent session)

- `resource::create` checks `VaultState::Cloud`: render template locally with `temper-provisional-id`, POST `/resources` with managed+open+body, receive canonical `temper-id`, rewrite the local file swapping provisional for canonical. No manifest write. On POST-success-but-local-write-failure, log a clear "created, local write failed, recover via `temper pull <id>`" message with the canonical id in the log line.
- `resource::update` (or the per-doctype equivalents): read the file, extract `temper-id` from frontmatter, PUT/PATCH `/resources/{id}` with the body + managed + open projection. No manifest write.
- `temper push <id|path>` in cloud mode: same as update, routed through Unit A's primitive with `manifest: None`.
- `temper pull <id>` in cloud mode: routed through Unit A's primitive with `manifest: None`, writing to a working directory (see B.3).
- `temper list`, `show`, `search`: route straight to the API without vault read when `VaultState::Cloud`. Search already does this in MCP; this generalizes to the CLI.
- `temper sync run`: no-op in cloud mode with a redirecting error ("cloud mode; use `temper push` and `temper pull` — sync is for manifest-backed vaults only").

### B.3 — Working directory and SessionStart wiring (subsequent session)

- In cloud mode, `temper init` creates a minimal `.temper/session/` directory in the current working directory (repo root, typically). This directory is gitignored by default via a `.temper/session/.gitignore` committed to the repo's top-level `.gitignore` or the session dir itself. Files the agent has actively touched live here; there is no full vault mirror.
- `.claude/settings.local.json` SessionStart hook calls `tools/bin/setup-claude-web.sh`, which now (post B.1) does: verify `TEMPER_TOKEN` is set, `cargo install --path crates/temper-cli`, `temper init` in cloud mode, warmup a per-context working dir. Committed to the repo.
- Audit of `tools/bin/setup-claude-web.sh` against this flow — today it's a lightweight scaffold; it needs to do the above.

### B.4 — Auth0 research block

This is explicitly a research subsection, not a prescribed implementation. The following questions must be answered before the B.2 and B.3 implementation sessions. Answers should land in a follow-up design note and inform whatever token issuance UX we build.

1. **Non-interactive token issuance paths.** Three candidate shapes:
   - **Machine-to-machine (M2M) client credentials** — a service account per user, scoped to their profile. Secure, rotatable, but requires a server-side enrollment step and the credentials have to live *somewhere* that the cloud session can read (GitHub secrets, Vercel env).
   - **Long-lived refresh tokens** — user runs `temper auth login` locally once, exports a refresh token via a new `temper auth export-refresh` command, drops it into cloud-session secrets. Cloud session exchanges it for access tokens. Refresh tokens stay user-bound and revocable via the Auth0 dashboard.
   - **Device-flow-token reuse** — cloud sessions pick up the same `access_token` the user already has cached locally, passed via env var. Simplest, but single-use-until-expiry, and an 8-hour access token won't outlive most multi-hour sessions.
   Research: which does Auth0 recommend for this exact shape (CI + cloud-agent runners that represent a human user)? What's the token rotation story for each?
2. **Refresh semantics in cloud sessions.** If the session holds a refresh token, it needs to be able to refresh mid-session. The current `temper-client` refresh code (`auth.rs:191-244`) writes the refreshed token back to `auth.json`. Cloud mode needs the same logic without the disk write — in-memory only, with the updated token living on the `Client`.
3. **Token scope/expiry trade-offs.** A cloud session doesn't need full user scope — it needs exactly what the current session is doing. Can we issue scope-reduced tokens per session? (Auth0 supports step-up and downgrade auth flows.) Trade-off: a reduced-scope token is safer if leaked, but adds complexity to the bootstrap flow.
4. **Security posture.** Where does the token actually live in a cloud session? (GitHub Actions secret, Claude/Cursor session secret, dropped into a file that's read-then-unlinked, kept only in memory?) How is it rotated? If a session is compromised, how is the token revoked without affecting other active sessions from the same user? The user's existing local flow has no equivalent of "revoke this one session's access" — that capability needs to be designed in.
5. **Provider abstraction.** `temper-client::auth` has a `provider: String` field that today is effectively hardcoded. Is cloud mode the right forcing function to make provider a real abstraction (Auth0 today, hypothetical self-hosted IdP tomorrow), or does that expand scope? Likely: leave the abstraction door open (enum-shaped provider field, not stringly-typed), but don't build the second provider speculatively.

The B.1 env-var floor is small enough that the answers to these research questions don't block it — it works with whatever token the user can hand it. B.2 and B.3 need answers first.

### Acceptance (for Unit B as a whole, across sub-PRs)

- A cloud session with `TEMPER_TOKEN` and `TEMPER_VAULT_STATE=cloud` exported can: `temper resource create --type session --context temper --title "test"`, observe the file is written with a canonical `temper-id` (not provisional), and `temper show <id>` (from a different cloud session) retrieves the same content from the server.
- `temper sync run` in cloud mode errors with the redirect message.
- No `auth.json` is created or required for any cloud-mode command.

---

## Unit C — Memory Doctype

**Third in sequence.** A new first-class doctype for builder/agent operating guidance. Thin schema, following the concept/decision precedent.

### Scope

- New schema file `crates/temper-core/schemas/memory.schema.json`, structurally identical to `concept.schema.json` and `decision.schema.json`:

  ```json
  {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "$id": "https://temperkb.io/schemas/memory.schema.json",
    "allOf": [
      { "$ref": "base.schema.json" }
    ],
    "properties": {
      "temper-type": { "const": "memory" },
      "slug": {
        "type": "string",
        "pattern": "^[a-z0-9][a-z0-9-]*$",
        "description": "URL-safe identifier"
      },
      "date": {
        "type": "string",
        "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$",
        "description": "Date memory was recorded (YYYY-MM-DD)"
      }
    },
    "required": ["slug", "date"],
    "additionalProperties": true
  }
  ```

- Add `DocType::Memory` variant to `crates/temper-core/src/frontmatter/document.rs:14` and wire through `to_str`, `from_str`, `schema_json` arms (pattern: follow the `Concept` and `Decision` arms exactly).
- Add `"memory"` to `VALID_DOC_TYPES` in `crates/temper-cli/src/commands/resource.rs:19`.
- New template `crates/temper-cli/templates/memory.md`, identical in shape to `concept.md` / `decision.md` (to be reviewed for exact fields when writing — they are already thin).
- No changes needed to sync, graph, search, MCP — they operate on `DocType` generically and gain memory for free.

### Path convention

`{owner}/{context}/memory/{slug}.md`. Context-scoped. No `_global` escape hatch — operating guidance is context-specific because working in a Rust project is different from working in a Bun project is different from working in a creative-writing project.

### Scope semantics

Memories carry the standard `owner` field from base schema, which reflects the authenticated profile. A memory written by a cloud agent session authenticated as `@you` is authored as `@you`. This is intentional: the JWT establishes identity, and the agent is acting on the user's behalf. If we want finer grain later (agent-written-for-user vs. human-written-for-self), it's an additive field, not a redesign.

### Migration path

The current skill-scoped `fundamentals.md` (and whatever the temper skill's `/temper init` writes today) becomes the seed set of memory resources, one per distinct piece of guidance. Migration is a one-time `temper resource create --type memory --context <ctx> --stdin < path/to/fundamentals.md` invocation — no special tooling needed because memory is a normal resource.

### Acceptance

- `temper resource create --type memory --context temper --title "..."` creates a valid memory resource, participates in sync, is returned by `temper list`, is searchable via `temper search`, shows up in graph builds.
- No changes required to existing tests beyond adding a memory-doctype smoke test.

---

## Sequencing Rationale

**A → B → C**, with B.1 shippable as a prototype PR in the same session as this spec.

- **A before B** because cloud-mode dispatch targets are cleaner if push/pull already exist as factored primitives. Building cloud-mode dispatch directly against the current `sync_orchestration` internals would re-create the factoring pressure inside cloud-mode code.
- **B before C** because memory wants to be exercisable from cloud sessions immediately — that's where the "portable guidance across machines" payoff lands. Shipping C before B works technically but defers the user-visible value.
- **B.1 before everything else in B** because the env var floor is small, reversible, and unblocks `cargo install --path crates/temper-cli && TEMPER_TOKEN=... temper sync run` in a cloud session today even without cloud mode proper. It's the minimum viable bootstrap.

All three units are independently shippable; intermediate states of the tree are never broken.

---

## Out of Scope

- **Decision cascade.** The propagation-engine shape of the `decision` doctype — new graph edge types (`supersedes`, `obviates`, `constrains`), cascade policies, content-patch mechanics — is its own spec. This doc does not block it and does not require it.
- **`_global` / cross-context memory.** Rejected. Operating guidance is context-specific.
- **MCP tool parity for new commands.** `temper-mcp` should gain `push`, `pull`, `memory_create` tools in parallel, but that work is a separate track and does not block Units A–C. The API endpoints exist either way; parity is strictly additive.
- **Server-side template rendering.** Deferred. Client-side rendering stays canonical for now; revisit only if CLI and server templates drift in practice.
- **Full MCP-only-mode session support.** This spec focuses on CLI-in-cloud-session. The MCP-only shape (agents with no CLI at all) follows the same shared-vocabulary principle but is its own design problem.

---

## Open Questions

Tracked here so subsequent sessions can pick them up explicitly.

1. **Auth0 research (B.4)** — full answer required before B.2 lands.
2. **Working directory layout in cloud mode (B.3)** — `.temper/session/` in the repo vs. `$XDG_RUNTIME_DIR/temper-session-<id>/`. The spec currently assumes in-repo under a gitignore; sanity-check this against how Claude Code on the web actually exposes working dirs.
3. **Template rendering locus.** Revisit only if and when CLI and server template behavior diverge.
4. **Write-through failure recovery.** The `pull <id>` recovery path is adequate for the ghost-resource-on-create case, but we should add a diagnostic command (`temper resource claim <id>`) for the rare case where the canonical id was never written back to local.
5. **Token provider abstraction.** How far to push the `provider: String` → enum migration as part of Unit B.

---

## Acceptance Criteria Summary

Per unit:

- **A**: one-entry primitives factored; `push` and `pull` as first-class commands; existing `sync run` behavior unchanged.
- **B.1**: `TEMPER_TOKEN`, `TEMPER_VAULT_STATE`, `TEMPER_PROVIDER` env vars recognized; no dispatch changes; no disk writes triggered by env vars.
- **B.2 / B.3**: cloud-mode `create` round-trips with provisional→canonical handoff; `sync run` redirects clearly; `list`/`show`/`search` route to API; SessionStart hook wired and documented.
- **B.4**: research block answered in a follow-up design note before B.2 implementation begins.
- **C**: `memory.schema.json` lands; `DocType::Memory` added; `temper resource create --type memory` works end-to-end; memory resources participate in sync, graph, search without doctype-specific code.

---

## Notes on This Document

This spec is the repo-side record of the design. The parallel vault-side goal resource is created separately via `temper resource create --type goal --context temper` and tracks the same work from the vault surface. Subsequent sessions should treat this doc as the authoritative starting point and add task-level work breakdowns in `docs/superpowers/plans/` as units enter execution.
