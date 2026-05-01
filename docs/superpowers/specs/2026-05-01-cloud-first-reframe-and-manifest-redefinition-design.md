# Cloud-First Reframe & Manifest Redefinition — Design Spec

**Date:** 2026-05-01
**Context:** `temper`
**Mode:** build
**Effort:** large (multi-session; pairs with shared-core-execution-paths spec)
**Branch:** `jct/wave1-shared-execution-paths-and-cloud-first-reframe`

**Related work:**
- Path-to-alpha goal item #3
- Companion spec: `2026-05-01-shared-core-execution-paths-design.md` (#4 — operations / backends / surfaces). The two specs are halves of the Wave 1 cloud-first reframe; #4 sets the architectural shape, this spec rides on it to define the conceptual model and lifecycle semantics.

---

## Problem

Cloud mode arrived as a feature alongside local mode. In practice it has become the cleaner, more portable, more powerful model — but the system still treats local mode as the conceptual default in skill, docs, and operational defaults. The mismatch produces:

- **Skill and CLAUDE.md presume local-first.** Operations are described as file-first, with cloud as the special case. This is backwards from where we want users to land.
- **Manifest as a steady-state concern instead of a recovery concern.** The manifest exists today as a three-way-merge ledger consulted on every push/pull. In a cloud-first world, most operations don't need it — they go straight to the server. Manifest's load-bearing role is recovery from offline / partial-failure states, not steady-state correctness.
- **`show` writes to the vault file** in local mode, then `update` reads from it. In a cloud-first model, `show` should write to a scratch space (temp file) that is *not* manifest-tracked; the user edits, then `update` pushes the new state. The temp file is a buffer, not a vault artifact.
- **Conflicting state semantics are implicit.** What does it mean for a resource to be "drifted"? When does the manifest detect divergence? How is auto-merge applied? Today these are answered by the sync code's behavior; they should be answered by an explicit lifecycle model.

The companion spec #4 establishes the architectural ground (`temper-core/operations/`, `Backend` trait, `Surface` enum, `VaultBackend` and `DbBackend` peers). This spec defines what *runs through* that architecture: the conceptual default, the lifecycle states, and the manifest's narrowed role.

## The Reframe

Three changes that together flip the conceptual model:

1. **Cloud-first becomes the documented default.** Skill, CLAUDE.md, README, and CLI help text all assume cloud-mode as the user's starting point. Local mode (`CliLocalVault`) is the documented opt-in: "I want my vault checked out on disk for faster ripgrep / find / native-bash agent investigation."
2. **`show` writes to scratch space, not vault.** In `CliCloud`, `show` always fetches from server and writes to a scratch buffer (temp file). In `CliLocalVault`, `show` uses the three-tier ladder (already shipped) but the result is *also* expressible as a scratch buffer — the vault file is incidental, not the source of authority. The scratch buffer is not manifest-tracked.
3. **Manifest narrows to a recovery artifact.** The manifest tracks the last-successfully-pushed hash for each resource and the queue of pending pushes. It does not gate steady-state operations. Steady-state CliLocalVault writes go through write-then-push (graceful fallback to deferred push on offline / auth fail). The manifest is consulted when sync push or sync pull runs, not on every operation.

## Lifecycle State Machines

Two backend-specific machines (Db and Vault), composed for `CliLocalVault`'s view.

### DbBackend Lifecycle

Server-side states for a resource row:

```
States:    Active(version=N) | SoftDeleted

Transitions:
  ∅                  --CreateResource-->  Active(0)
  Active(N)          --UpdateResource-->  Active(N+1)
  Active(*)          --DeleteResource-->  SoftDeleted
  SoftDeleted        --RestoreResource--> Active(*)         [future scope]

Events emitted on transition:
  DbResourceCreated, DbResourceUpdated, DbResourceSoftDeleted
  Plus body-changed:  DbChunksGenerated, DbEmbeddingTriggered
```

Versioning is server-managed. `version=N` is monotonically increasing per resource. Concurrent updates from two sessions land deterministically (last write wins at the SQL transaction level; concurrency control is the existing optimistic-lock pattern, not changed by this spec).

### VaultBackend Lifecycle

Local-vault states for a resource as understood from the manifest:

```
States:
  Drafted        — local file exists; never pushed
  Synced         — manifest synced_hash == current_file_hash
  Modified       — manifest synced_hash present but ≠ current_file_hash; push not attempted
  PendingPush    — push attempted; failed (offline / not authed); queued for retry
  Conflicting    — server has changes since last synced_hash; local also has unsync'd changes
  LocallyDeleted — file removed; tombstone push pending

Transitions (canonical events shown; full list in events section):
  ∅              --CreateResource (push ok)----->  Synced(hash=H)
  ∅              --CreateResource (push fail)--->  Drafted
  Drafted        --SyncPushResource (ok)-------->  Synced(hash=H)
  Drafted        --SyncPushResource (fail)------>  PendingPush
  Synced(H)      --UpdateResource (push ok)----->  Synced(hash=H')
  Synced(H)      --UpdateResource (push fail)--->  PendingPush
  Synced(H)      --SyncPullResource (no remote)->  Synced(H)
  Synced(H)      --SyncPullResource (auto-merge)>  Synced(H')
  Synced(H)      --SyncPullResource (conflict)-->  Conflicting
  Modified       --SyncPushResource (ok)-------->  Synced(hash=H')
  Modified       --SyncPushResource (fail)------>  PendingPush
  Modified       --SyncPullResource (conflict)-->  Conflicting
  PendingPush    --SyncPushResource (ok)-------->  Synced
  PendingPush    --SyncPullResource (conflict)-->  Conflicting
  Conflicting    --resolve (manual or auto)----->  Synced
  *              --DeleteResource-------------->  LocallyDeleted
  LocallyDeleted --tombstone push (ok)---------->  ∅

Events emitted on transition:
  VaultFileWritten, VaultManifestUpdated, VaultFileRemoved
  RemoteSynced, PushDeferred, ConflictDetected, AutoMergeApplied
```

`Modified` is reachable as a transient state when a write happens but push hasn't been attempted yet (e.g., user edited via `show-edit-cat`'s scratch file in a way the CLI doesn't auto-push). In normal `temper resource update` operation, the state goes `Synced → Modified → (push) → Synced` or `Synced → Modified → (push fail) → PendingPush` without `Modified` being externally observable for long.

### Composite View — `CliLocalVault`

A `CliLocalVault` resource has a *composite* state: `(VaultState, LastKnownDbState)`.

- Steady-state ideal: `(Synced, Active(N))` — local file matches what was last pushed; server has version N.
- After a successful update: both halves advance atomically — `(Synced(H'), Active(N+1))`.
- After an update with deferred push: only the vault half advances — `(PendingPush, Active(N))`. The server remains at N until the bulk-recovery sync drains the queue.
- After a remote-side change (another session pushed): `(Synced(H), Active(N+k))` until next pull or push detects divergence.

The composite view is what state machines for `CliLocalVault` operate on. Each command transitions the composite state; the events emitted by the operation describe both halves.

## Conflicting-State Semantics

A `Conflicting` state arises when a sync pull (or a push that the server rejects with "remote diverged") detects that *both* the local file and the server row have changed since the last `Synced(H)` baseline.

### Detection

- **On push:** if push attempts to write a resource whose server-side hash diverges from the manifest's last-known synced hash, server returns a conflict response. Manifest transitions resource to `Conflicting`.
- **On pull:** if pull fetches a server-side resource whose hash diverges from the manifest's last-known synced hash *and* the local file has also been modified since that hash, transition to `Conflicting`.
- **On auto-merge attempt:** the `similar` crate's paragraph-level merge runs first. If the merge succeeds (non-overlapping changes in distinct paragraphs / frontmatter regions), state transitions `Conflicting → Synced(H'')` and emits `AutoMergeApplied`. If the merge fails, state remains `Conflicting`.

### Surfacing

`Conflicting` resources are visible in:

- `temper sync status` output (a new or evolved subcommand) — lists conflicting resources with both versions' hashes.
- Output of any `temper resource update` or `temper resource show` against a conflicting resource — explicit warning, no silent overwrite.

### Resolution

Three options, in order of automation:

1. **Auto-merge (default attempt)** — `similar`-based paragraph-level merge runs on conflict detection. Succeeds for the common case of independent edits in distinct sections.
2. **Manual local resolution** — user edits the local file to merge, then runs `temper resource update <slug>` (or `temper sync push`) which pushes the resolved state and clears the conflict.
3. **Take-server / take-local** — explicit verb flags (e.g., `temper sync resolve <slug> --take-server` / `--take-local`) for when the user knows which side to keep without manual editing.

Auto-merge is the default-on behavior. Manual and take-* are explicit user actions.

## Manifest's Narrowed Role

Today's manifest:
- Tracks every locally-known resource with hash.
- Consulted on every read (decide whether to fetch / use cache).
- Consulted on every write (decide push base hash).
- Powers three-way merge for conflict resolution.

After this spec:
- Tracks last-successfully-pushed hash per resource (recovery baseline).
- Tracks the pending-push queue (resources in `PendingPush` state).
- Tracks the conflict queue (resources in `Conflicting` state).
- Not consulted on steady-state read or write — those go directly to backend through `Surface::dispatch`.
- Consulted by `temper sync push` / `temper sync pull` / `temper sync status` for recovery and conflict resolution.

This is option (b) from the path-to-alpha goal text: "manifest restricted to a narrower role." Not option (a) "fully optional" (it stays a real artifact in `CliLocalVault`) and not option (c) "replaced by checkout abstraction" (no new verb introduced).

### `show-edit-cat` and Scratch Space

`temper resource show <slug>` writes to a scratch buffer, not to a manifest-tracked vault file. In:

- **CliCloud**: scratch buffer at `${TMPDIR}/temper-show/<slug>.<doctype>.<ts>.md` (or similar) — a temp file that lives outside the vault entirely. Edit, then `cat <buffer> | temper resource update <slug>` pushes the new state.
- **CliLocalVault**: same scratch-buffer model conceptually. The vault file is incidental — `show` may write to it as a debounced cache (existing three-tier ladder), but the *authoritative* output for `show-edit-cat` is the scratch buffer. The vault file is updated by an explicit `update` command, not by `show`.

The scratch buffer is **never manifest-tracked**. It is a session-scoped artifact.

This makes the `show-edit-cat` idiom uniform across modes and removes the asymmetry that produced the phantom-flow CLAUDE.md paragraph (now-fixed in PR #63 but the conceptual unification is what closes the door on the bug class).

## Skill and CLAUDE.md Updates

- **Skill (`~/.claude/skills/temper/SKILL.md` and `reference.md`)**: rewrite "How to use temper" sections to assume `CliCloud` is the default user surface. Local-mode opt-in is documented as "if you want files on disk for ripgrep, set up a vault and `unset TEMPER_VAULT_STATE`."
- **CLAUDE.md**: replace the existing local-first paragraphs with cloud-first paragraphs. Document `show-edit-cat` as scratch-buffer-based, with vault-file specifics as a sub-note for local-mode users.
- **README**: update the "getting started" section to install + log in + ingest + use, with no manifest or vault-checkout step.

These edits are part of this spec's scope. Item #21 (CLAUDE.md correctness audit) catches anything we miss.

## Acceptance Criteria

- [ ] `temper-core/operations/state.rs` defines `DbResourceState`, `VaultResourceState`, and `CompositeResourceState` types reflecting the lifecycle states above.
- [ ] State transitions are encoded as a function or method per command on each state type, with an exhaustive `match` over current state — illegal transitions become compile errors, not runtime panics.
- [ ] Manifest persists only `synced_hash`, `pending_push_queue`, and `conflict_queue` (plus per-resource identity fields). Steady-state ops do not touch the manifest file.
- [ ] `temper resource show` writes to a scratch buffer outside the vault. The buffer path is reported in CLI output. The vault file is not modified by `show` (in `CliLocalVault`, the existing three-tier-ladder cache may still be used internally; it is not the surfaced artifact).
- [ ] `temper sync status` lists `PendingPush` and `Conflicting` resources distinctly with hash info.
- [ ] `temper sync resolve <slug> --take-server` and `--take-local` flags exist for explicit conflict resolution.
- [ ] Auto-merge runs on conflict detection (existing `similar` crate machinery; preserved unchanged).
- [ ] Skill, CLAUDE.md, README updated to cloud-first defaults.
- [ ] Existing tests pass (`cargo make test`, `test-db`, `test-e2e`). New tests cover state-machine transitions per the testing plan below.

## Testing Plan

Aligns with companion spec #4's Phase 6.

### temper-core unit tests (pure)

- Each command × each starting state → assert resulting state and emitted events.
- Illegal transitions (e.g., `CreateResource` against an `Active(N)` server state with the same slug) produce explicit error variants, not panics.
- Auto-merge against representative conflicting documents (frontmatter-only conflict, body-paragraph conflict, both) produces expected merged output or `Conflicting` state.

### Per-backend integration tests

- `DbBackend` (`temper-api` / `tests/`): each command transitions DB state correctly; events emitted match expected; SQL effects match (rows updated, chunks generated, embedding triggered).
- `VaultBackend` (`temper-cli` integration): each command transitions vault file + manifest correctly; events match; file-system effects match (file written, removed, manifest entry updated).

### E2E (`tests/e2e/`)

- Composite `CliLocalVault` flows: write happy path (Synced → Synced); offline write (Synced → PendingPush → recovery → Synced); conflicting case (Synced → Conflicting → resolve → Synced).
- `CliCloud` and `Mcp` flows for the same commands; assert identical observable outcomes for the same input where backend-specific divergence doesn't apply.
- `show-edit-cat` round-trip in both modes — scratch buffer not manifest-tracked.

## Out of Scope

- The architectural foundation (commands, actions, events, backend trait, surface enum, module placement). Owned by companion spec #4.
- Auth refresh, MCP transport changes, web UI changes — all separate goal items.
- Replacing manifest with a `temper checkout` verb (option (c) of the goal text). Considered and rejected: option (b) — narrowing — preserves more of the working sync machinery without forcing a new mental model on existing users.
- Hard-deletion / restore semantics. `RestoreResource` is sketched in the Db lifecycle as future scope; no client-side surface for it in alpha.

## Open Questions

- **Scratch buffer cleanup.** When does the scratch directory get cleaned? On CLI exit? On a TTL? Manual `temper scratch clean`? Defer to implementation; default to "session-scoped + TTL fallback."
- **`PendingPush` retry policy.** Does `temper sync push` retry pending items in order, oldest first? Deepest-pending-first? Defer to implementation; default to oldest-first FIFO.
- **Conflict surfacing in `temper resource list`.** Should conflicting resources be highlighted in list output, or only in `sync status`? Lean: highlight in list (a column or marker) so users notice without opening sync status separately. Decide at implementation time.
