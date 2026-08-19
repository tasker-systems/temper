# WS6 flip-readiness strategy — the adoption-grade gate to cutover

**Date:** 2026-06-16
**Status:** Strategy / sequencing decision (roadmap over WS6's remainder). Not an implementation plan; each gating unit below gets its own spec → plan.
**Supersedes phrasing in:** the `substrate-kernel-to-cognitive-map` goal's WS6 "Remaining" line and §D-adjacent text where they conflict with this document.

## The bar (decided 2026-06-16)

Flip-readiness is the **gate for adoption by other organizations**, not merely the gate for flipping temperkb.io. The consequence is a hard rule:

> **No flip-with-a-gap.** Every place `NextBackend` is a lossy or partial port relative to the production read/write surface is a hard flip prerequisite. "Safe-by-population because temperkb.io is single-tenant (Pete + Claude, blast radius 2)" is **not** an acceptable correctness argument anywhere, because the readiness target is an arbitrary org with real differential access, multiple subjects, and full surface use.

This decision retires three earlier framings:
- The "defer WS2 past the flip + tripwire" option (single-tenant safety) — **rejected**. WS2 gates.
- The "minimal owner-only scoping" middle path — **rejected**. Full scoping gates.
- The goal's stale "short dual-write window" — **already superseded by §D** (hard cutover, no dual-write); restated here for the record.

## What is already done (the flip *mechanism* exists)

Chunks 1–4 (PRs #134, #135, #136, #137) delivered the cutover machinery. Behind the `kb_backend_selection` flag, **both reads and writes already work against `temper_next`**, gated OFF in production:

- Synthesis-from-state — repeatable rebuild of `temper_next` from live `public.*` (`crates/temper-next/src/synthesis/`).
- §8 per-resource body hash-parity gate (`synthesis/parity.rs`).
- Parity-read harness — list / show+meta / body / FTS / vector / graph (chunk 3).
- `NextBackend` reads (`readback`) + writes (4c), grown `Backend` trait, surfaces dispatching through `select_backend`.

So the remaining work is **closing the last port gaps + rehearsing**, not building the cutover.

## The adoption-grade flip-prerequisite inventory

Each row is grounded in a real site, not the goal's narrative. Status as of 2026-06-16.

### 1. WS2 — full access-scoping over `temper_next` (GATES — longest pole, unspecced)

NextBackend reads are **visibility-UNSCOPED**:
> `crates/temper-api/src/backend/read_selector.rs:12-13` — *"Reads are visibility-UNSCOPED at the §9 floor (access-scoping over `temper_next` is a named flip prerequisite, WS2)."*

Production scopes every read through `resources_visible_to` / `can_modify_resource`. The successor must reproduce the full model proven artifact-side in PR #129: teams-DAG, capability descriptor, producer-intersection, public floor via root team, profile-direct grants (consumer axis). There is **no** `resources_visible_to`-over-`temper_next` design doc on disk — the access specs present (`2026-06-02-access-capability-model-design.md`, `2026-06-11-access-scaffold-scenario-proof-design.md`) are artifact-side scenario proofs and earlier system-access work, not the readback-scoping unit. **This is the first build: it needs its own brainstorm → spec → plan.**

### 2. Surface-completeness port (GATES — every production surface must answer fully from `temper_next`)

- **`by_uri` re-addressing.** `read_selector.rs:6-9` — *"`by_uri` is NOT covered … the addressing key does not exist there; `origin_uri` is the substrate key. It stays on legacy under `next`."* Today it silently falls back to legacy; post-flip there is **no legacy to fall back to**, so it becomes a hard break unless re-addressed.
- **MCP `get_resource` / `list_resources` enrichment reads** — deferred from 4b (relationship enrichment over public ids); still legacy-only.
- **Native-id write addressing.** `NextBackend` write addressing by scoped slug is unimplemented:
  `crates/temper-api/src/backend/next_backend.rs:173` and `:375` — `ResourceRef::Scoped { .. } => Err(TemperError::NotImplemented(...))`. Writes resolve only by UUID via the `ResolvedIds` prod→next indirection (`read_selector.rs:135,233`). This is the chunk-5 core, and **it is where the §5 `ResourceRef::Scoped` collapse / slug-retirement actually belongs** (per Adjudication 5's identity contract: UUID | decorated `sluggify(title)-<uuid>`, trailing-UUID-only resolution, one resolver).

### 3. Deployed-adapter `next-backend` enable (GATES — mechanical)

`next-backend` is a **non-default** cargo feature:
> `crates/temper-api/Cargo.toml:40` — `next-backend = ["dep:temper-next"]`

The Vercel runtime adapter (`api/axum.rs`, `api/mcp.rs`) has no per-deploy `--features` (`vercel.json` passes only `SQLX_OFFLINE`). The deployed binary must compile NextBackend in by flip time (make it default, or adjust the adapter build). Easy to forget; a hard blocker.

## Not gating (explicitly off the critical path)

- **§5-narrow hygiene** — `correlation_id`→neutral edge-handle rename (neutral surface only: `operations/commands.rs`, the `Backend` trait per `backend.rs:77` "the edge handle", `relationship_requests.rs` — **not** the `temper-events` ledger field nor the production DB column, both of which keep `correlation_id` legitimately) + `ManagedMeta` genericization (cognitive-map domain prep, not workflow-adoption-gating). Behavior-neutral; land whenever convenient or ride along with the surface-completeness work.
- **Crate extraction** (`temper-substrate` / `temper-workflow`) — post-cutover, last, against the stable schema.

## The one orthogonal call: re-minted-id continuity (temperkb-local, NOT an adoption gate)

Synthesis currently **mints fresh ids** per resource (`crates/temper-next/src/synthesis/mod.rs:141`, tracked in `state.resource_id_by_old`). Re-minted ids are a declared §9 non-invariant, and native-id addressing drops the `ResolvedIds` map post-flip — so any external reference to a prod-uuid (UI URLs, durable docs, bookmarks) goes dead at the flip.

**This is not an adoption gate:** a new org starts fresh on `temper_next`; there is no migration to break. It only affects temperkb.io's own prod→next continuity. PR #124's identity-as-input mechanism *could* preserve prod ids in synthesis and dissolve the breakage entirely — a clean choice worth evaluating, but decidable on its own merits and resolvable any time before the real flip. It does **not** gate the build work below.

## Sequence

1. **WS2 — full access-scoping over `temper_next`.** Spec first (own brainstorm → spec → plan → build). Longest pole.
2. **Surface-completeness port** — `by_uri` re-addressing + MCP enrichment reads + native-id write addressing (folds in the `ResourceRef::Scoped` collapse / slug-retirement). One narrative: "every production surface answers fully from `temper_next`."
3. **Deployed-adapter `next-backend` enable** — mechanical.
4. **§5-narrow hygiene** — ride-along, non-gating.
5. **Re-mint-vs-preserve id decision** — temperkb-local; resolve before the real flip.
6. **Rehearse** synthesis + hash-parity + parity-read on fresh Neon branches until boring → **flip** (write-freeze → final synthesis → set flag → rename legacy aside) → done.

## First build

WS2 access-scoping over `temper_next` — its spec.

## Grounding citations (evidence, per implementation-grounding GD-1)

- `crates/temper-api/src/backend/read_selector.rs:6-13` — `by_uri` legacy-only; reads visibility-unscoped (WS2 named).
- `crates/temper-api/src/backend/next_backend.rs:173,375` — scoped-ref write addressing `NotImplemented`.
- `crates/temper-api/src/backend/read_selector.rs:135,233` — `ResolvedIds` prod→next indirection.
- `crates/temper-api/Cargo.toml:40` — `next-backend` non-default feature.
- `crates/temper-next/src/synthesis/mod.rs:141` — fresh-id minting in synthesis.
- `crates/temper-core/src/operations/resource_ref.rs:16-30` — `ResourceRef::{Uuid,Scoped}`.
- `crates/temper-core/src/operations/backend.rs:77` — "the edge handle (`Uuid`): correlation_id for `DbBackend`, edge_id for [NextBackend]".
- Adjudication 5 + §D: `docs/superpowers/specs/2026-06-12-ws6-convergence-delta-adjudication-design.md:253-279,380-439`.
