# T2 — Templated Team-Telos Genesis Implementation Plan (REVISED)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Revision note (2026-07-01):** This plan replaces an earlier draft that built a *pure-Rust*
> `team_telos_charter()` function + a bespoke `cogmap genesis-team` command with a deterministic
> uuidv5 id. A code survey (background investigation, session note) established that the durable,
> reusable home is the **existing `schema-artifact/manifests/*.yaml` genesis mechanism**, and that
> **every CRUD primitive already exists**: `temper team create`, `temper context create`,
> `temper cogmap create --manifest --name`, `temper cogmap bind`. A telos charter is **id-free**
> (blocks keyed by role+seq, fold-and-reprojected wholesale), and genesis ids are optional /
> CLI-minted, so a **telos-only genesis manifest is already reusable across teams as-is**. The
> revised T2 is therefore **near-zero Rust**: author one durable YAML artifact + one SoP that
> composes the existing idempotent primitives — mirroring the org-bootstrap precedent
> (`schema-artifact/install-profile.yaml` + `scripts/bootstrap/system-bootstrap.sh` +
> `docs/guides/org-bootstrap.md`). Decisions confirmed with the user: (1) YAML template + compose
> existing CRUD commands (lean into CRUD, SoP can start as an echo-only script); (2) **team-agnostic
> telos prose** (zero interpolation — the team's identity rides on the map's `--name`).

**Goal:** Deliver a **reusable, durable** team-self-cognition telos-charter template — a cognitive
map born 1:1 with a team, from the "understand how this team works" charter — plus the SoP that
composes the existing idempotent CRUD primitives to birth and bind it for any team. This is the
foundation the Eve steward (T5) tends.

**Architecture:** One new static artifact + one SoP. The artifact is a **telos-only genesis
manifest** (`schema-artifact/manifests/team-self-cognition.yaml`) mirroring
`schema-artifact/manifests/org-identity.yaml`'s shape (`name` / `telos_title` / `telos:` with
`statement` + questions-with-context + `framing`) but with **team-agnostic** prose. The SoP composes
three existing commands — `temper team create <slug>` → `temper cogmap create --manifest <file>
--name "<Team> — self-cognition"` → `temper cogmap bind <cogmap-ref> <team>` — capturing the minted
cogmap id between steps (the exact pattern `install-profile.yaml` documents). Idempotency is
**inherited from the primitives** (team create ON CONFLICT; cogmap create is a no-op at a pinned id;
bind is idempotent on the join PK).

**Tech Stack:** YAML (genesis manifest) + Markdown (SoP runbook); optional POSIX shell (applier
script). The genesis embed path is the existing `#[cfg(feature = "embed")]` `cogmap create` — **no
Rust changes in this plan**.

## Global Constraints

- **No Rust code changes** are expected. If execution discovers a genuinely-missing primitive,
  STOP and escalate — do not silently add a bespoke command; the whole point of this revision is to
  compose existing CRUD.
- The genesis manifest must parse via the existing `genesis::parse_manifest`
  (`crates/temper-cli/src/actions/genesis.rs`) with **no parser change** — match `org-identity.yaml`'s
  keys exactly. Verify by running `temper cogmap create --manifest <the new file> --name …` against
  the dev DB (Task 3), not by adding a unit test to a crate this plan otherwise doesn't touch.
- Telos prose is **team-agnostic** ("this team", never a hardcoded team name) — this is what makes
  the artifact reusable with zero interpolation.
- The charter is embedded **client-side** (ONNX) by `cogmap create`, so Task 3 requires the `embed`
  feature: `cargo build -p temper-cli --bin temper --features embed`.
- Keep the artifact **telos-only** — do NOT add a `reconcile`-style `entries:` section. Reconcile
  entries carry a *required static uuidv7 primary key* and would collide across teams (not reusable).
  Per-team landmark delivery, if ever wanted, mints ids per team — out of scope here.

---

## File Structure

- **Create:** `schema-artifact/manifests/team-self-cognition.yaml` — the durable, reusable telos-only
  genesis manifest (the template). One responsibility: "the authored 'understand how this team works'
  charter, team-agnostic, ready for `cogmap create --manifest`."
- **Create:** `docs/guides/team-self-cognition-bootstrap.md` — the SoP runbook: the exact command
  sequence to birth + bind any team's self-cognition map, with the id-capture step called out.
- **(Optional) Create:** `scripts/bootstrap/team-self-cognition.sh` — a thin applier that either
  echoes the workflow (dry-run default) or runs it (`--apply`), mirroring
  `scripts/bootstrap/system-bootstrap.sh`'s idiom. May be deferred to a follow-up; the runbook is the
  MVP deliverable.

**Consumes (existing, verified in the codebase):**
- `temper team create <slug> [--name <n>] [--parent <ref>]` (`crates/temper-cli/src/cli.rs:552`,
  `commands/team.rs`) — creates a team; caller becomes owner. Idempotent by slug.
- `temper context create <name> [--team <ref>]` (`crates/temper-cli/src/cli.rs:481` `ContextAction`)
  — the team's working context (the ingest source; e.g. `building`).
- `temper cogmap create --manifest <file> [--name <n>] [--id <ref>]`
  (`crates/temper-cli/src/cli.rs:655`, `commands/cogmap.rs`, `actions/genesis.rs`) — genesis from a
  manifest; admin-gated; idempotent at a pinned id (`created:false` on re-run).
- `temper cogmap bind <cogmap-ref> <team>` (`crates/temper-cli/src/cli.rs:689`,
  `actions::cogmap::bind_api`) — the 1:1 team↔cogmap join; admin-gated; idempotent on the join PK.
- Genesis manifest shape: `GenesisManifestDoc { cogmap_id?, telos_resource_id?, name, telos_title,
  telos? }` with `ManifestTelos { statement, questions: Vec<CharterQuestion{question, context}>,
  framing: Vec<String> }` (`crates/temper-cli/src/actions/{genesis,reconcile}.rs`,
  `crates/temper-core/src/charter.rs`).

---

## Task 1: The durable team-self-cognition telos manifest (the template)

**Files:**
- Create: `schema-artifact/manifests/team-self-cognition.yaml`

**Interface:**
- Produces a telos-only genesis manifest that `temper cogmap create --manifest` parses and births a
  cogmap from — reusable for **any** team (team identity supplied at apply time via `--name`).

- [ ] **Step 1: Author the manifest** — mirror `schema-artifact/manifests/org-identity.yaml`'s exact
  key shape (`name` / `telos_title` / `telos:` → `statement` + `questions[].{question,context}` +
  `framing[]`). Omit `cogmap_id` / `telos_resource_id` (the SoP pins or captures them). Use the
  content below verbatim (team-agnostic prose; the goal statement, the determinism reframe, and the
  D3 label vocabulary are the load-bearing lines):

```yaml
# Team self-cognition cognitive map — GENESIS manifest (REUSABLE TEMPLATE).
#
# Consumed by:  temper cogmap create --manifest schema-artifact/manifests/team-self-cognition.yaml \
#                 --name "<Team> — self-cognition"
# Then bound 1:1 to the team:  temper cogmap bind <minted-cogmap-ref> <team>
#
# This is the reusable template for a TEAM SELF-COGNITION map: a cognitive map born 1:1 with a team,
# whose ingest source is the team's OWN temper resources. The prose is deliberately TEAM-AGNOSTIC
# ("this team") so the single artifact serves every team without interpolation — the team's identity
# rides on the map's `--name` at apply time. Tended by the Eve steward (create / assert / facet /
# fold); regions emerge from `materialize` — the steward never clusters. See the T3 spec:
# docs/superpowers/specs/2026-06-30-steward-act-model-cogmap-resource-vocabulary-design.md
#
# IDENTITY: omit `cogmap_id` / `telos_resource_id` and the CLI mints stable uuidv7s and prints them.
# Pin them in the SoP (per team) once you want a reproducible, re-runnable genesis — a re-run at the
# same id is an idempotent no-op (`created: false`).

name: "Team — self-cognition"
telos_title: "How this team works"
telos:
  statement: >-
    Understand how this team works — what it is actively working on, the problems it solves and for
    whom, the domains it owns, the decisions it has settled and the commitments it holds, and the
    concerns and open questions it carries. This map is the team's self-cognition, dogfed from the
    team's own temper resources: its nodes are distilled from those resources and situated by this
    telos. Salience is judged under this purpose — never universally.
  questions:
    - question: "What is this team actively working on?"
      context: "Surfaces the live themes and the most active threads — the team's current front."
    - question: "What problems does this team solve, and for whom?"
      context: "The team's reason-for-being, distilled from its work rather than declared abstractly."
    - question: "What does this team know — its domains of expertise and responsibility?"
      context: "The areas the team owns; where its judgment is authoritative."
    - question: "What has this team decided, and what has it committed to?"
      context: "Settled decisions and outstanding commitments — the load-bearing choices to honor."
    - question: "What concerns or open questions is the team holding?"
      context: "Live tensions and unresolved questions worth tracking before they are settled."
  framing:
    - "Nodes are distilled from the team's own resources and carry a `derived_from` edge to their source(s)."
    - "The steward tends declared structure (create / assert / facet / fold); regions emerge from `materialize` — the steward never clusters."
    - "Node labels are expressive: concept, fact, memory, question, theme, concern, principle, commitment, domain."
```

- [ ] **Step 2: Validate it parses + births** — this is verified end-to-end in Task 3 (there is no
  crate unit test to add; the manifest is data consumed by the existing embed-gated CLI path). Do a
  quick local YAML sanity check if desired (`python3 -c "import yaml,sys; yaml.safe_load(open(sys.argv[1]))" schema-artifact/manifests/team-self-cognition.yaml`).

- [ ] **Step 3: Commit**

```bash
git add schema-artifact/manifests/team-self-cognition.yaml
git commit -m "feat(schema-artifact): reusable team-self-cognition telos genesis manifest"
```

---

## Task 2: The SoP runbook (compose the CRUD primitives)

**Files:**
- Create: `docs/guides/team-self-cognition-bootstrap.md`

**Interface:**
- A step-by-step runbook an operator (or the steward's deploy step, T6) follows to birth + bind a
  team's self-cognition map from the Task-1 template. Team-agnostic; `<team-slug>` / `<Team>` are
  placeholders.

- [ ] **Step 1: Write the runbook** — mirror the structure of `docs/guides/org-bootstrap.md` (the
  org analogue). Cover, in order, with the exact commands:
  1. `temper team create <team-slug> --name "<Team>"` — create the team (you become owner). Idempotent by slug.
  2. `temper context create <ctx> --team <team-slug>` — the team's working context (the ingest source, e.g. `building`). *(Confirm the exact `context create` flags against `cli.rs:481` while writing; adjust the command to match.)*
  3. `temper cogmap create --manifest schema-artifact/manifests/team-self-cognition.yaml --name "<Team> — self-cognition"` — births the map; **capture the printed `cogmap_id`** (or pin it in the manifest for a reproducible re-run, per the manifest's IDENTITY note).
  4. `temper cogmap bind <captured-cogmap-ref> <team-slug>` — the 1:1 team↔cogmap join.
  - Call out that **idempotency is inherited from the primitives** (re-running converges, does not
    duplicate) and that admin (system-admin) is required for `cogmap create` / `cogmap bind`
    (reference `reference_l0_content_delivery_admin_gate` behavior).
  - Note the id-capture-vs-pin choice explicitly (same tradeoff `install-profile.yaml` documents).

- [ ] **Step 2 (optional, may defer):** add `scripts/bootstrap/team-self-cognition.sh` — a thin
  applier taking `--team <slug> --name <Team> [--context <ctx>] [--apply]`; **default is a dry-run
  that echoes the exact command sequence** (the "no-op script that echoes the workflow" the user
  asked for), `--apply` runs them and captures the minted cogmap id between steps. Mirror
  `scripts/bootstrap/system-bootstrap.sh`'s option-parsing + `temper`-invocation idiom. If deferred,
  say so in the runbook and leave a `TODO(T6)` pointer.

- [ ] **Step 3: Commit**

```bash
git add docs/guides/team-self-cognition-bootstrap.md
# (+ scripts/bootstrap/team-self-cognition.sh if authored)
git commit -m "docs(bootstrap): SoP to birth + bind a team's self-cognition cogmap"
```

---

## Task 3: End-to-end verification (real run against the dev DB)

The genesis path is admin-gated + embed + DB-bound, so the durable check is a real run.

- [ ] **Step 1: Bring up the dev DB and build the embed CLI**

```bash
cargo make docker-up
cargo build -p temper-cli --bin temper --features embed
```

- [ ] **Step 2: Run the SoP once for a scratch team** (e.g. `--team t2-selfcog-smoke`), following
  Task 2's runbook exactly: `team create` → `context create` → `cogmap create --manifest … --name` →
  `cogmap bind`. Confirm genesis prints `created: true` and a `cogmap_id`; the bind succeeds.

- [ ] **Step 3: Re-run genesis at the SAME id** (pin the printed `cogmap_id` via `--id` or the
  manifest) and confirm `created: false` (idempotent no-op), and that the team bind stays a single
  `kb_team_cogmaps` row.

- [ ] **Step 4: Confirm the 1:1 join + the templated charter** — `temper cogmap shape <cogmap-ref>`
  shows the map; its telos carries the templated statement / five questions / three framing lines.
  (Once T1's `cogmap_read_charter` tool lands it reads the prose directly; until then inspect via the
  existing analytics/shape surface.)

- [ ] **Step 5: (optional) e2e test** — only if the `tests/e2e/` harness already has an
  admin-minting fixture + `test-embed`: a test that runs the sequence through the real CLI→API→DB
  path and asserts `created:true` then `created:false` + exactly one `kb_team_cogmaps` row
  (`cargo make test-e2e-embed`). Otherwise the manual run above is the acceptance evidence — note it
  in the report.

---

## Out of scope (Rejected vs Deferred)

**Rejected (load-bearing decisions — resist scope creep):**
- A bespoke `cogmap genesis-team` command or a pure-Rust `team_telos_charter()` function. The
  primitives already compose; a wrapper duplicates them and re-introduces a deterministic-id concern
  the SoP handles by id-capture-or-pin.
- `{{team_name}}` interpolation into the telos prose. Team-agnostic prose is reusable with zero code;
  the team identity rides on `--name`.
- A `reconcile`-style `entries:` section in the template (static uuidv7 primary keys → cross-team
  collision → not reusable).

**Deferred (in scope elsewhere or later):**
- The specific operational step of creating the **`temper` team + `building` context** and
  **re-homing** the current `@j-cole-taylor/temper` corpus to `+temper/building`. This is J's own
  install's one-time migration, **not needed in most installs**, and is a separate operational task
  (neonctl / the SoP applied to J's cloud) — decide-later, tracked outside this durable deliverable.
- Hardening the SoP script from echo-only into a full idempotent applier (folds into T6 deploy).
- Auto-birth-of-self-cogmap-per-team (MVP uses the on-demand SoP).

## Self-Review

- **Spec coverage (T2 acceptance):** a reusable templated team-telos charter (Task 1) applied 1:1 to
  a team via idempotent genesis + bind (Tasks 2–3), grounded on the real `schema-artifact` YAML
  mechanism per the user's steer. The 1:1 + idempotency come from the existing primitives; the
  template is the parameterization (via `--name`, team-agnostic prose).
- **Near-zero-code, by design:** the revision's whole point is that the CRUD primitives already
  exist. If an executor finds themselves writing Rust, that is the escalation signal in Global
  Constraints.
- **Reusability proof:** telos blocks are id-free and genesis ids are CLI-minted/optional, so the one
  static artifact serves every team — verified by Task 3's scratch-team run being independent of any
  specific team.
- **Precedent alignment:** mirrors `org-identity.yaml` (manifest) + `install-profile.yaml` /
  `system-bootstrap.sh` / `org-bootstrap.md` (SoP) — the same split the org-bootstrap surface already
  ships.
