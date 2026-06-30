# T2 — Templated Team-Telos Genesis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a team-parameterized genesis that births a cognitive map 1:1 with a team (joined via `kb_team_cogmaps`) from a *templated* team-telos-charter ("understand how this team works"), idempotently — the foundation for the team-self-cognition steward MVP.

**Architecture:** Compose the two existing idempotent operations the L0 kernel migration already composes inline — `cogmap_genesis` (birth an unbound cogmap from a telos charter) + `bind_team` (`INSERT kb_team_cogmaps … ON CONFLICT DO NOTHING`) — behind one new CLI command, with a deterministic per-team cogmap id so re-runs are idempotent (1:1). The charter prose comes from a new pure `team_telos_charter(team_name)` template producing a `ManifestTelos`, embedded client-side (the existing embed-gated genesis path).

**Tech Stack:** Rust, clap (CLI), temper-client (HTTP), temper-core charter types, ONNX embed (existing `embed` feature gate), sqlx/Postgres (server, unchanged), cargo-make + cargo-nextest.

## Global Constraints

- Quality gate: `cargo make check` (fmt + clippy `-D warnings` + docs + machete + TS). Run before every commit; the pre-commit hook also runs it.
- Always build/clippy with `--all-features`.
- The genesis embed path is gated `#[cfg(feature = "embed")]`; the new command mirrors that gate (non-embed build returns the same "requires embed" error as `cogmap create` at `commands/cogmap.rs:181`).
- No new SQL: this task reuses `cogmap_genesis` and `bind_team`; **no migration**, so no sqlx cache regen needed for new queries unless you add a `sqlx::query!` (you won't).
- UUIDs: repo convention is uuidv7 for minted ids; this task introduces **one** deterministic uuidv5 derivation for the per-team self-cogmap id (reserved-derived id, same spirit as L0's literal reserved ids).
- After any temper-cli change that the e2e suite spawns, rebuild the bin: `cargo build -p temper-cli --bin temper` (nextest rebuilds the lib, not the spawned bin).

---

## File Structure

- **Create:** `crates/temper-cli/src/actions/team_telos.rs` — the pure charter template + deterministic id derivation. One responsibility: "given a team, produce its self-cognition telos charter + stable ids."
- **Modify:** `crates/temper-cli/src/actions/mod.rs` — declare `pub mod team_telos;`.
- **Modify:** `crates/temper-cli/src/cli.rs` — add a `GenesisTeam` variant to the cogmap subcommand enum.
- **Modify:** `crates/temper-cli/src/commands/cogmap.rs` — the `genesis_team()` handler (orchestrates template → genesis → bind), embed-gated.
- **Test:** unit tests inline in `team_telos.rs`; a clap parse test inline in `cli.rs`'s existing cogmap test module (or `commands/cogmap.rs`).

**Consumes (exact, from the codebase as mapped):**
- `crate::actions::reconcile::ManifestTelos { statement: String, questions: Vec<CharterQuestion>, framing: Vec<String> }` (`crates/temper-cli/src/actions/reconcile.rs:29-34`).
- `temper_core::charter::CharterQuestion { question: String, context: String }` (`crates/temper-core/src/charter.rs:9-14`).
- `crate::actions::genesis::{manifest_to_request}` and `GenesisManifestDoc { cogmap_id: Option<Uuid>, telos_resource_id: Option<Uuid>, name: String, telos_title: String, telos: Option<ManifestTelos> }` (`crates/temper-cli/src/actions/genesis.rs:14-30, 43-`).
- `crate::actions::cogmap::resolve_team_id(client, team) -> Result<Uuid>` (`crates/temper-cli/src/actions/cogmap.rs:62-84`).
- Client calls: `client.cognitive_maps().create_cognitive_map(&CreateCogmapRequest) -> CreateCogmapOutcome { cogmap_id, telos_resource_id, created }` and the bind call used by `bind_api` (`crates/temper-cli/src/actions/cogmap.rs:87-98`, `BindTeamRequest { team_id }`).
- `client.teams().list()` for team name lookup (used inside `resolve_team_id`).

---

## Task 1: The team-telos charter template (pure)

**Files:**
- Create: `crates/temper-cli/src/actions/team_telos.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs` (add `pub mod team_telos;`)

**Interfaces:**
- Produces: `pub fn team_telos_charter(team_name: &str) -> ManifestTelos`; `pub fn self_cogmap_id(team_id: Uuid) -> Uuid`; `pub fn self_telos_resource_id(team_id: Uuid) -> Uuid`; `pub const TEAM_SELF_COGMAP_NAMESPACE: Uuid`.

- [ ] **Step 1: Write the failing test**

In a new file `crates/temper-cli/src/actions/team_telos.rs`, add at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charter_is_team_named_and_nonempty() {
        let t = team_telos_charter("Temper");
        assert!(t.statement.contains("Temper"));
        assert!(!t.questions.is_empty(), "must seed orienting questions");
        assert!(!t.framing.is_empty(), "must seed framing");
        // every question carries context (the questions-with-context topology)
        assert!(t.questions.iter().all(|q| !q.question.is_empty() && !q.context.is_empty()));
    }

    #[test]
    fn ids_are_deterministic_per_team() {
        let team = Uuid::parse_str("019f1ac7-ed17-78c1-8003-7fe3af72609d").unwrap();
        assert_eq!(self_cogmap_id(team), self_cogmap_id(team), "stable across calls");
        // cogmap id and telos id are distinct
        assert_ne!(self_cogmap_id(team), self_telos_resource_id(team));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p temper-cli team_telos -v`
Expected: FAIL — `team_telos.rs` doesn't define `team_telos_charter` / `self_cogmap_id` yet (compile error).

- [ ] **Step 3: Write the implementation**

At the top of `crates/temper-cli/src/actions/team_telos.rs`:

```rust
//! Templated team self-cognition telos charter.
//!
//! Produces the "understand how this team works" telos for a team's 1:1 self-cognition
//! cognitive map, plus deterministic (reserved-derived) ids so re-running genesis for the
//! same team is idempotent. The steward (Workflow A) tends this map; regions emerge from
//! `materialize`. See docs/superpowers/specs/2026-06-30-steward-act-model-cogmap-resource-vocabulary-design.md.

use uuid::Uuid;

use crate::actions::reconcile::ManifestTelos;
use temper_core::charter::CharterQuestion;

/// Namespace for deriving stable per-team self-cogmap ids (uuidv5). Fixed constant — do not change
/// once any team's self-cogmap has been born, or ids would drift and re-genesis would duplicate.
pub const TEAM_SELF_COGMAP_NAMESPACE: Uuid =
    Uuid::from_u128(0x05a1_face_0000_0000_0000_5e1f_c061_7a00);

/// Deterministic cogmap id for a team's self-cognition map.
pub fn self_cogmap_id(team_id: Uuid) -> Uuid {
    Uuid::new_v5(&TEAM_SELF_COGMAP_NAMESPACE, format!("cogmap:{team_id}").as_bytes())
}

/// Deterministic telos-resource id for a team's self-cognition map.
pub fn self_telos_resource_id(team_id: Uuid) -> Uuid {
    Uuid::new_v5(&TEAM_SELF_COGMAP_NAMESPACE, format!("telos:{team_id}").as_bytes())
}

fn q(question: &str, context: &str) -> CharterQuestion {
    CharterQuestion { question: question.to_string(), context: context.to_string() }
}

/// The "understand how this team works" telos charter, parameterized by team name.
pub fn team_telos_charter(team_name: &str) -> ManifestTelos {
    ManifestTelos {
        statement: format!(
            "Understand how the {team_name} team works — what they are working on, the problems \
             they solve, the decisions and commitments they hold, the domains they own, and how \
             they operate. This map is the team's self-cognition, dogfed from the team's own \
             temper resources."
        ),
        questions: vec![
            q("What is this team actively working on?",
              "surfaces live themes and the most active threads"),
            q("What problems does this team solve, and for whom?",
              "the team's reason-for-being, distilled from its work"),
            q("What does this team know — its domains of expertise and responsibility?",
              "the areas the team owns"),
            q("What has this team decided, and what has it committed to?",
              "settled decisions and outstanding commitments"),
            q("What concerns or open questions is the team holding?",
              "live tensions and unresolved questions worth tracking"),
        ],
        framing: vec![
            "Nodes are distilled from the team's resources and carry a `derived_from` edge to \
             their source(s).".to_string(),
            "The steward tends declared structure (create / assert / facet / fold); regions emerge \
             from `materialize` — the steward never clusters.".to_string(),
            "Node labels are expressive: concept, fact, memory, question, theme, concern, \
             principle, commitment, domain.".to_string(),
        ],
    }
}
```

Then add to `crates/temper-cli/src/actions/mod.rs` (alongside the other `pub mod` lines):

```rust
pub mod team_telos;
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo nextest run -p temper-cli team_telos -v`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/actions/team_telos.rs crates/temper-cli/src/actions/mod.rs
git commit -m "feat(cli): team-telos charter template + deterministic self-cogmap ids"
```

---

## Task 2: The `cogmap genesis-team` command (orchestrates birth + bind)

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (cogmap subcommand enum — find the enum the `Create`/`Reconcile`/`Shape` variants at `cli.rs:650-667` belong to; add a sibling `GenesisTeam` variant)
- Modify: `crates/temper-cli/src/commands/cogmap.rs` (handler + dispatch arm; mirror `create()` at `commands/cogmap.rs:144-181`)

**Interfaces:**
- Consumes: Task 1's `team_telos_charter`, `self_cogmap_id`, `self_telos_resource_id`; `genesis::manifest_to_request`; `cogmap::resolve_team_id`; the client's create + bind calls.
- Produces: `temper cogmap genesis-team --team <slug-or-uuid>` — births (idempotent) and binds the team's self-cognition cogmap; prints the `CreateCogmapOutcome` + bind result.

- [ ] **Step 1: Add the clap variant**

In `crates/temper-cli/src/cli.rs`, add to the cogmap subcommand enum (next to `Create`):

```rust
/// Genesis a team's 1:1 self-cognition cognitive map from the team-telos template.
///
/// Idempotent: the cogmap id is derived deterministically from the team id, so re-running
/// is a no-op birth + idempotent team bind. Admin-gated (system admin) like `create`.
GenesisTeam {
    /// Team ref — a slug (optional `+` sigil) or a team UUID.
    #[arg(long)]
    team: String,
},
```

- [ ] **Step 2: Write the failing parse test**

In the cogmap command/cli test module (mirror the existing clap `try_parse_from` tests, e.g. those near `commands/edge.rs:142-313`), add:

```rust
#[test]
fn genesis_team_parses_team_flag() {
    use clap::Parser;
    let cli = crate::cli::Cli::try_parse_from(
        ["temper", "cogmap", "genesis-team", "--team", "temper-system"],
    ).expect("parses");
    // navigate to the GenesisTeam variant and assert team == "temper-system"
    // (match the exact enum path used by sibling tests in this module)
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo nextest run -p temper-cli genesis_team_parses -v`
Expected: FAIL — `GenesisTeam` arm not yet handled / variant just added but no dispatch (compile error in `commands/cogmap.rs` match).

- [ ] **Step 4: Write the handler**

In `crates/temper-cli/src/commands/cogmap.rs`, add the dispatch arm (next to the `Create` arm in the command match) and the handler. Mirror `create()` (cogmap.rs:144-181), but build the request from the template instead of a manifest file:

```rust
#[cfg(feature = "embed")]
pub fn genesis_team(team: &str) -> Result<()> {
    runtime::with_client(|client| {
        Box::pin(async move {
            // 1. Resolve the team (slug or uuid) and its display name.
            let team_id = crate::actions::cogmap::resolve_team_id(&client, team).await?;
            let teams = client.teams().list().await?;
            let team_name = teams.iter()
                .find(|t| t.id == team_id)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| team.to_string());

            // 2. Build a templated genesis manifest with deterministic ids (idempotent re-genesis).
            let doc = crate::actions::genesis::GenesisManifestDoc {
                cogmap_id: Some(crate::actions::team_telos::self_cogmap_id(team_id)),
                telos_resource_id: Some(crate::actions::team_telos::self_telos_resource_id(team_id)),
                name: format!("{team_name} — self-cognition"),
                telos_title: format!("{team_name}: how this team works"),
                telos: Some(crate::actions::team_telos::team_telos_charter(&team_name)),
            };
            let req = crate::actions::genesis::manifest_to_request(doc)?;

            // 3. Birth (idempotent on cogmap_id) then bind 1:1 to the team (idempotent ON CONFLICT).
            let outcome = client.cognitive_maps().create_cognitive_map(&req).await?;
            crate::actions::cogmap::bind_api(client, outcome.cogmap_id, team).await?;

            crate::output::print_json(&outcome)?; // match the surrounding output convention
            Ok(())
        })
    })
}

#[cfg(not(feature = "embed"))]
pub fn genesis_team(_team: &str) -> Result<()> {
    // Mirror the non-embed error returned by `create()` at commands/cogmap.rs:181.
    anyhow::bail!("`cogmap genesis-team` requires the `embed` feature (client-side charter embedding)")
}
```

> Verify against the codebase as you write: the exact `runtime::with_client` signature and the
> output helper (`create()` shows both); `bind_api`'s parameters (`actions/cogmap.rs:87-98`); and
> whether `manifest_to_request` is `#[cfg(feature = "embed")]` (it is — keep the handler under the
> same gate). `client.teams().list()` item field names (`id`, `name`) — confirm from `resolve_team_id`.

- [ ] **Step 5: Run the parse test + full crate tests**

Run: `cargo nextest run -p temper-cli` and `cargo build -p temper-cli --all-features`
Expected: PASS / clean build (both embed and non-embed arms compile).

- [ ] **Step 6: `cargo make check`**

Run: `cargo make check`
Expected: green (fmt, clippy `-D warnings`, docs, machete, TS).

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/cogmap.rs
git commit -m "feat(cli): cogmap genesis-team — templated team self-cognition genesis + bind"
```

---

## Task 3: End-to-end idempotency verification (manual + optional e2e)

The genesis path is admin-gated and embed+DB-bound, so the durable check is a real run, not a unit test.

- [ ] **Step 1: Bring up the dev DB and rebuild the bin**

```bash
cargo make docker-up
cargo build -p temper-cli --bin temper --features embed
```

- [ ] **Step 2: Run genesis-team for the temper-system team twice (idempotency)**

```bash
temper cogmap genesis-team --team temper-system
temper cogmap genesis-team --team temper-system   # second run
```

Expected: first run `created: true`; second run `created: false` (existence pre-check on the deterministic `kb_cogmaps.id`), and the team bind stays a single row (`ON CONFLICT DO NOTHING`).

- [ ] **Step 3: Confirm the 1:1 join and the charter**

```bash
temper cogmap shape <printed-cogmap-ref>            # map exists
```

Expected: the cogmap is bound to `temper-system` in `kb_team_cogmaps` (one row), and its telos carries the templated `statement`/`question`/`framing` blocks (verifiable once Task in T1's charter-read tool lands; until then, inspect via the existing reconcile/analytics surface).

- [ ] **Step 4 (optional): add an e2e test**

If the e2e harness (`tests/e2e/`) has an admin-minting fixture and the embed feature is enabled (`cargo make test-e2e-embed`), add a test that calls `genesis-team` twice through the real CLI→API→DB path and asserts `created: true` then `created: false` plus exactly one `kb_team_cogmaps` row. Place it under `tests/e2e/tests/`. (Gate behind `test-embed`; the genesis path needs client-side embedding.)

- [ ] **Step 5: Commit (if an e2e test was added)**

```bash
git add tests/e2e/tests/<file>.rs
git commit -m "test(e2e): genesis-team births + binds team self-cogmap idempotently"
```

---

## Self-Review

- **Spec coverage:** Implements goal-task T2 ("templated team-telos genesis: parameterize the L0-style birth for any team, joined 1:1 via `kb_team_cogmaps`, idempotent, works for the temper team"). The 1:1 + idempotency come from the deterministic id + existing existence pre-check; the template is the parameterization.
- **Open decision surfaced for the executor:** the MVP's *first target team* — `temper-system` is the only canonical team today. The plan targets it; if a dedicated `temper` team is wanted instead, create it first (`temper team create` / seed) and pass its slug — no code change (the command is team-agnostic).
- **Placeholder scan:** the only "verify against the codebase as you write" note is on faithful-copy glue (`with_client`/output helper/`bind_api` arity) where the exact local idiom must match its neighbors — the agent mapped the copy-source line ranges; this is a copy-match instruction, not a TODO.
- **Type consistency:** `ManifestTelos`/`CharterQuestion`/`GenesisManifestDoc`/`CreateCogmapOutcome` field names match the mapped definitions; `self_cogmap_id`/`self_telos_resource_id` names are consistent across Tasks 1–2.
