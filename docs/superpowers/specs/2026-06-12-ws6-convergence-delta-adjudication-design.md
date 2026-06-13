# WS6 Convergence Delta Adjudication: Production ↔ Artifact, Every Call Binding

**Date:** 2026-06-12
**Status:** Design — adjudicated in brainstorming, pending plan
**Goal:** `substrate-kernel-to-cognitive-map`, workstream 6 (migration/convergence) — first beat
**Charter:** `schema-artifact/seeds/temper-convergence.yaml` (question 1: *"The adjudication spec resolves
the named deltas … Work in flight builds on those calls, and a moved call invalidates silently."*)
**Extends:** `2026-06-01-data-model-reconciliation-design.md` (the delta master),
`2026-06-02-access-capability-model-design.md`, `2026-06-09-event-payload-formalization-design.md`

## What this spec is

The binding decision record for the temper → temper-next convergence. Every
production↔artifact delta was inventoried (six code axes + a spec-register sweep + a live
production-data audit), classified **settled / mechanical / decision-required**, and every
decision-required delta was adjudicated with Pete on 2026-06-12. Each adjudication below records
the call **and its binding remap rule** — the genesis-event-synthesis semantics that downstream
WS6 work (migrations expressed from the artifact, the parity-read harness, surface cutover) builds
against. A moved call requires reopening this spec, not a quiet local decision.

The deployment shape (cutover mechanics) is adjudicated here too (§D) — the charter's
pre-committed "short dual-write window" is **superseded** by this spec (see §D).

## Method and grounding

- Seven parallel inventory agents: (a) tables/columns, (b) SQL functions/views/triggers,
  (c) event ledger, (d) CLI/MCP/API/shared-type surfaces, (e) access model,
  (f) identity/addressing, (g) spec-register sweep over the June corpus. Every claim
  file:line-cited; classifications re-checked at synthesis.
- Live production audit (Neon `temper-cloud`, 2026-06-12, read-only) — the data realities below.
- Two agent claims were caught false at synthesis and are corrected in this record:
  `kb_resources.slug` **exists** in production (nullable, partial-unique per active context —
  `20260513065121`), and production's table is **`kb_scopes`**, not `kb_cogmaps` — the CS-3
  rename was spec-vocabulary only, never DDL (the data-model spec's "kb_cogmaps already exists"
  line is wrong about the built name).

## Production data realities (audited 2026-06-12)

| Probe | Value |
|---|---|
| Resources | 1,184 (1,181 active, 3 soft-deleted) |
| Edges (`kb_resource_edges`) | 529 (1 folded) — `near` 454, `contains` 68, `leads_to` 6, `express` 1; ≤3 labels/kind |
| Events | 8,661 — `body_updated` 4,453 + `managed_meta_updated` 3,095 (87% sync churn), `resource_created` 562, `relationship_asserted` 545, others ≤3 |
| Chunks | 14,394 current / 76,861 total; 3,262 revisions; 1,184 manifests |
| Emitters | 1 profile (all events); 105 distinct `(profile, device)` pairs, ~95 of them singleton UUIDv7 session-devices (May 19–27); stable: `api` 2,307, four user devices, `migration` 481, `mcp` 72, `ledger` 66 |
| Access | `kb_team_resources` **0 rows**, `kb_team_members` 0, join requests 0; 1 team; `immutable` never used anywhere in code |
| Contexts | 11 (profile- and team-owned); 1 `kb_scopes` row |
| `managed_meta` keys in use | exactly 16 (see §7); 705 resources carry non-empty `open_meta` |
| Operational tables | blob_files / ingestion_records / transfers / join_requests all 0; device_sync_state 3 |

Two structural facts drove adjudication: **the ledger is incomplete** (562 `resource_created`
for 1,184 resources — it started mid-life; verbatim replay cannot reconstruct state even in
principle), and a non-trivial slice of data reflects changed expectations or bugs not worth
cleaning at the time (Pete, in-session) — noise that synthesis-from-state deliberately leaves
behind.

## Inventory disposition (the non-decision bins)

**Settled — cited and carried, not re-decided here:** doctype demotion to `kb_properties
key='doc_type'`; `kb_resources` slimming + homes/access split with grant anchors
`('kb_teams','kb_profiles')`; kernel-side slug drop; edge-home polymorphism; porosity retirement;
teams-DAG + `kb_team_cogmaps` + producer-intersection (scenario-proven, PR #129); capability
booleans + coherence CHECK; system-access as profile status + trigger-maintained root membership;
payload-first ledger discipline (PR #124: typed versioned payloads, identity-as-input,
append-only, replay proofs); content-block model (blocks ⊃ chunks, roles as properties, D3
generic block reads).

**Mechanical:** index/constraint rewrites; `kb_team_members` composite-PK simplification;
`kb_topics` / `kb_chunk_content` carry unchanged; ts-rs regeneration ripples into temper-ui;
`payload_version=1` is moot (new ledger is born at v≥1); show-cache tier-2 re-sourcing follows §7.

**Spec-register sweep outcome:** 241 recorded deferred/open/gated items across the June corpus;
~79% answered by temper-next or later specs, ~7% obsolete (superseded framings), 33 live —
28 migration-relevant, and every one of those is either adjudicated below or named in an
adjudication's open residue. The corpus reconciles clean.

---

## Adjudication 0 — Unit of backfill & old-ledger disposition

**Delta.** Production's 8,661-event ledger is structurally incomplete, 87% sync churn, with
emitter noise and bug-era artifacts. The artifact requires a ledger that is the source of truth
(replay byte-identical).

**Gates.** Every other remap rule's vocabulary; the parity harness's definition of parity; the
rollback story.

**Call.** Backfill is **genesis-event synthesis from current projected state** — the old ledger
is not the migration source (the data forecloses it). The old ledger is **archived aside**:
`kb_events` + `kb_event_types` (and the rest of the legacy schema at cutover) move to a `legacy`
Postgres schema untouched, plus a pre-cutover Neon point-in-time branch. The new ledger starts
clean at genesis synthesis; its replay guarantee is never polluted by foreign rows; no legacy
emitter mapping is ever built. Synthesis-from-state is also the **curation boundary**: the new
ledger is born from what temper currently means; the noise stays in the archive.

**Binding remap rule.**
- Synthesis covers **active state only**: soft-deleted resources (3) and anything dangling off
  them are not synthesized — they live in the archive.
- Per live resource: `resource_created` (with block/chunk manifests per §8) → `property_asserted`
  per surviving key (§7) → `relationship_asserted` per edge (§4); folded rows synthesize as
  **assert + fold event pairs** so fold semantics stay event-sourced.
- Emitter: the `migration` entity (§1); per-resource hash-parity gate (§8) verifies every
  synthesized resource.
- Any future drop of the `legacy` schema is a named post-migration decision, never implicit.

**Open residue.** None.

## Adjudication 1 — Ledger merge: entities

**Delta.** Production emits `(profile_id, device_id-freeform)`; artifact requires
`emitter_entity_id` → `kb_entities` + polymorphic `producing_anchor`. Production mints a device
UUID per CLI session (the 95 singletons). The artifact has no `entity_created` event — entities
are administrative infrastructure.

**Call.**
- **1a — migration emitter:** one `migration` entity bound to Pete's profile,
  `metadata: {intent: "migration", source: "temper-production", migrated_at: …}` — the
  established `intent=migration` pattern at the entity tier.
- **1b — runtime granularity: durable per-(profile, surface) entities** — `pete@cli`,
  `pete@mcp`, `pete@web`. Session/device identifiers move into event `metadata`, never entity
  identity. Agent instances get explicit entities minted at launch with the artifact's
  launch-metadata shape (model, platform, persona, bound_cogmap). No per-session entities — the
  device sprawl does not reproduce at the actor tier.
- **1c — producing anchor for synthesized events:** the subject's home anchor —
  `('kb_contexts', ctx)` for context-homed resources; edge events anchor at the edge's home.

**Binding remap rule.** Migration seeds: the per-surface entities for the live profile + the
`migration` entity. Legacy `(profile, device)` pairs are never mapped (decision 0).

**Open residue.** Entity creation stays administrative (no event), matching the artifact. If the
admin-event-sourcing lane (operating-shape commitment) later wants entity lifecycle as events, it
lands there — not invented here.

## Adjudication 2 — Owned-context policy

> **Amendment (2026-06-13) — contexts are owner-scoped, slugged namespaces.** The original call below
> migrated contexts to a *thin unowned* `kb_contexts(id, name UNIQUE, created)`. That conflated two roles
> of ownership and dropped both: *ownership-for-access-gating* (`contexts_visible_to` — correctly retired;
> visibility is at home/team grain) **and** *ownership-for-namespace-scoping* (which namespace a context's
> name/slug lives under — wrongly dropped). A global `name UNIQUE` is a latent multi-tenant error and a
> concrete synthesis-collision risk: production keys uniqueness per-owner across 11 profile- **and**
> team-owned contexts. This amendment restores **only** the namespace-scoping role (the "bounding-constraint
> utility survives" line below, now in DDL). Revised shape:
> ```sql
> CREATE TABLE kb_contexts (
>     id           UUID PRIMARY KEY DEFAULT uuid_generate_v7(),  -- canonical reference + access mechanism
>     owner_table  VARCHAR(64) NOT NULL CHECK (owner_table IN ('kb_profiles','kb_teams')),
>     owner_id     UUID NOT NULL,
>     slug         TEXT NOT NULL,   -- per-owner addressable handle (team-style: unique slug, free name)
>     name         TEXT NOT NULL,   -- display label (may collide across owners)
>     created      TIMESTAMPTZ NOT NULL DEFAULT now(),
>     UNIQUE (owner_table, owner_id, slug)
> );
> ```
> uuid stays the reference everything else uses (`kb_resource_homes.anchor_id`, `kb_team_contexts.context_id`,
> `kb_events.producing_anchor_id` — all unchanged). `contexts_visible_to` **stays retired**; `kb_team_contexts`
> is still the sharing mechanism, orthogonal to owner. The **Call** and **Binding remap rule** below are
> superseded by the **Amended remap rule** at the end of this section.

**Delta.** Production: 11 contexts owned via `(kb_owner_table, kb_owner_id)`, uniqueness
`(owner, name)`, `contexts_visible_to` gates listing, every resource carries `kb_context_id`.
Artifact: `kb_contexts(id, name, created)` — unowned navigation anchors; ownership/visibility at
resource-home grain.

**The reframe (Pete, in-session).** Context-as-owned-namespace was the unix-path solution to the
sync-projection era's bounding problem; that mental model retires with sync. But the
bounding-constraint **utility survives**: contexts remain durable workflow bounds whose contents
become reachable sources for cognitive-map elaboration — workflow resources (research, session
notes, tasks, decisions) become event-sources a map can investigate along its telos-charter,
`$ref`-able via the payload-references vocabulary and the foreign-events door.

**Call.** Contexts migrate to the artifact's thin unowned shape. What replaces the useful part of
ownership is **context-shareability**: a team↔context association (`kb_team_contexts`, sibling to
`kb_team_cogmaps`) meaning "this team's vis-reach includes this context's resources (and
context-homed edges)." Cognitive maps reach workflow content **only through team-mediated
producer-intersection** (`resources_accessible_to_cogmap` over the map's joined teams) — no
direct map↔context coupling exists. The keyboard-holder's reach never enters; no new trust path.

**Binding remap rule.**
- Context rows synthesize by name; ownership columns drop; `contexts_visible_to` retires with no
  successor (listing is unprivileged; resource visibility does the gating).
- Every context-homed resource → home row `('kb_contexts', ctx)` carrying its current
  originator/owner.
- Team-owned-context content (the `general`-style bootstrap floor) reaches everyone via
  root-team mechanics, not context ownership.
- `temper-context` frontmatter key dies — derivable from the home row at render time.

**Design elements bound here (artifact amendments, leak-safety-gated).**
- `kb_team_contexts` does **not** exist in the artifact; per the charter ("adjudicated access
  deltas land in the access-scaffold first"), its DDL lands as an artifact amendment with
  WS2-pattern scenario proofs **before** any production work depends on it. Required coverage:
  consumer reach through a shared context; producer-intersection with a shared context in the
  topology; the private-edge-between-public-endpoints check re-proven with a context-homed edge.
- **Default personal team** — a loopback-self-reference team per profile, auto-existing and
  invisible, so a solo user's maps read their own contexts through the same intersection
  mechanics (no special case, no visibility-model bend). Binding constraint from charter Q4: if
  solo users must think about teams to wire their own maps, the implementation is wrong.

**Open residue.** The team↔context capability shape (read-only share vs contribute); whether a
cogmap can ever home in a context (current answer: no — maps join teams; map↔context stays
indirect, preserving the intersection discipline).

**Amended remap rule (2026-06-13, supersedes the "Binding remap rule" above).**
- Per production context → `owner_table`/`owner_id` carried verbatim; `name` = production name; `slug` =
  `sluggify(name)`, disambiguated on the rare per-owner slug collision; uuid newly minted (old→new id map
  threaded to the resource/home pass).
- For each **team-owned** context, synthesize an explicit `kb_team_contexts(context_id, owning_team_id)`
  row so the owning team still reaches its contents through the **unchanged** visibility function
  (`vis_team`/`resources_visible_to`). Owner stays purely namespace-scoping; reachability is never implied
  by ownership. Profile-owned contexts need no such row (their resources are owner-visible via homes/access).
- Every context-homed resource → home row `('kb_contexts', ctx)` carrying its current originator/owner
  (unchanged from the original rule).
- `temper-context` frontmatter key dies — derivable from the home row at render time (unchanged).
- Access-scaffold coverage: the access-scenario's context rows now carry an owner (a profile or team from
  the scenario `world`) + a derived slug. The leak-safety invariants are **unchanged** — they gate on
  resource visibility, not context naming — so this is a mechanical loader/model update, not a new proof.

## Adjudication 3 — Access remap

**Delta.** Production: `access_level (vault|mutable|immutable)` on `kb_team_resources` +
`can_modify_resource`. Artifact: capability booleans on `kb_resource_access` + role-ceiling
intersection. **Data: zero grant rows; `immutable` unused in any code path** — the remap is
code/model only.

**Call (ratified).**
- Remap table, recorded as documentation of intent (never executed against data):
  `vault → (read, write, delete, grant)`, `mutable → (read, write)`, `immutable → (read)` —
  intersected with the role-ceiling (access spec §6).
- Default at creation: home confers the owner's full capability; **no implicit team grants** —
  sharing is always an explicit grant. Public floor via root team (§2).
- Profile-direct grants ship in the schema (leak-safety proven, PR #129; consumer-axis only);
  surface UX for them is not a migration deliverable.
- `team_role` carries unchanged as `(management, ceiling)` pairs; `access_level` and
  `kb_team_resources` drop with nothing to port.

**Open residue.** None.

## Adjudication 4 — Edge taxonomy

**Delta.** Production already runs `edge_kind(4) + polarity + label` (May 22 cutover); the
8→4 mapping lives in `20260522100002` + `EdgeType::legacy_mapping()`. Legacy 8-vocab survives
only in archived events.

**Call (ratified).** The 8→4+label mapping is **final and frozen** — and under decision 0 it is
historical documentation only. No event-vocabulary remap exists anywhere in the migration.

**Binding remap rule.** Per live edge: synthesize `relationship_asserted` from the
`kb_resource_edges` row — kind, polarity, label, weight verbatim; home anchor per §1c; the one
folded edge synthesizes as an assert+fold pair.

**Open residue.** None.

## Adjudication 5 — Slug-retirement surface contract

**Delta.** Kernel slug drop is settled; every surface still speaks slug (CLI positional slugs,
MCP `slug + context_name` — except delete, already UUID-only; vault filenames `<slug>.md`;
show-edit-cat). No title-based lookup exists anywhere. This contract gates surface cutover and is
the largest workflow-simplicity exposure (charter Q4).

**Call — the identifier contract.**
- **Identity in:** every surface accepts a bare UUID or the decorated form
  `sluggify(title)-<uuid>`. Resolution is **trailing-UUID-only** — the decoration half is parsed
  off and ignored, so a wrong or stale slug half is harmless by construction. Decorations are
  never stored, never authoritative, regenerated freely on title change.
- **Name fragments are never identity.** Ref slots do not accept fragments — no fuzzy-match
  resolution, no ambiguity behavior, because no ambiguous input is ever a ref. Fuzzy finding
  lives in explicit search/list affordances whose output is decorated refs (copy → paste closes
  the loop).
- **Identity out:** everything that prints a resource prints the decorated form. Vault projection
  filenames become `sluggify(title)-<uuid>.md` — every filename self-resolving.
- **One resolver:** a single resolve affordance in temper-workflow (UUID | decorated → resource),
  consumed by CLI, MCP, and the skill. The MCP/CLI drift heals by both dispatching through it.

**Binding consequence.** `ResourceRef::Scoped(owner, context, doctype, slug)` collapses; the
temper skill's command sequences rewrite against decorated refs; `kb_profiles.slug` /
`kb_teams.slug` owner sigils stay out of scope (per data-model §4), unchanged.

**Open residue.** Search/list ergonomics (how good fragment-finding is) — quality-of-surface
work for the cutover phase, not contract.

## Adjudication 6 — Domain-A operational tables

**Delta.** The artifact deliberately excludes ~8 operational tables + the FTS machinery
("not kernel," not "delete").

**Call (ratified, amended by §7).** Tables backing live features carry forward unchanged:
`kb_profile_auth_links`, `kb_system_settings` + `kb_join_requests`, `kb_team_invitations`,
`kb_blob_files` / `kb_ingestion_records` / `kb_transfers`, `kb_resource_search_index` + FTS
triggers (rebuilt against the new column reality). **Dropped as vestigial** (with §7):
`kb_device_sync_state`, the sync service/handler/SQL machinery.

**Open residue.** None beyond §7's plan-time caller check.

## Adjudication 7 — Manifests dissolution: the exhaustive key remap

**Delta.** `kb_resource_manifests` (1,184 rows; `managed_meta`/`open_meta`/three hashes) has no
artifact successor table. Exactly 16 managed keys exist in production.

**Call.** The manifests table **drops entirely** — no successor reads. Cloud-only pull is
hard-refresh-remote-overtop; there is no local-drift detection and no remaining sync mechanism,
so the whole sync apparatus retires with it: `sync_diff_for_device`, `sync_service.rs`,
`handlers/sync.rs`, `kb_device_sync_state`, device/sync wire types (plan-time gate: confirm zero
callers — temper-client already has no sync module — then delete; no stubs). The artifact's
`kb_resources.body_hash` **stays with a changed justification**: A1's sync rationale is
historical; the column is the content fingerprint whose first consumer is the migration's own
hash-parity gate.

**Binding remap rule (per key, exhaustive).**

| Key (count) | Fate |
|---|---|
| `temper-title` (1,184) | dies — already `kb_resources.title` |
| `temper-slug` (1,184) | dies — decision 5 (render-time decoration) |
| `temper-id` (6) | dies — derived from `kb_resources.id` |
| `temper-stage` (598), `temper-mode` (584), `temper-effort` (584), `temper-status` (65), `temper-seq` (111) | `kb_properties` rows (workflow fields) |
| `temper-goal` (363) | **edge**, using the kind+label the existing frontmatter-edge projection emits — verified against `graph.rs` at plan time, not guessed here |
| `temper-llm-run` (51), `temper-provenance` (51), `temper-branch` (24), `temper-pr` (11), `date` (5) | `kb_properties` rows verbatim |
| `temper-type` (12 strays) | reconciled against the doctype column — **column wins** (the general noise rule: where a stray manifest key conflicts with authoritative state, state wins; the stray dies in the archive) |
| `temper-context` (9 strays) | dies — derivable from the home row |
| `open_meta` keys (705 resources) | `kb_properties` rows verbatim |
| `managed_hash` / `open_hash` | die with the table |

**Open residue.** None beyond the `temper-goal` kind+label plan-time verification.

## Adjudication 8 — Content-tier remap

**Delta.** Production: flat per-resource chunk-sets (+ resource-grain revisions). Artifact:
resource ⊃ role-tagged blocks ⊃ chunks (+ block-grain revisions).

**Call.** **Single block per resource at migration** — every existing resource backfills as one
up-front content block containing its current chunk-set verbatim: chunks, sha256 content hashes,
and bge-768 embeddings carry as-is (embeddings are non-replayed derived state; carrying beats
recomputing). Block structure accretes through real edits (`block_mutate`) and future authoring —
the migration fabricates no structural claims the author never made. Historical chunks (76,861
total) and `kb_resource_revisions` (3,262) stay in the archive — state, not history.

**Binding remap rule.** Per resource: one `BlockManifest` (seq 0, no role) wrapping the current
chunk manifests in order; `body_hash` = merkle over the single block's hash, block hash over its
ordered chunk hashes — deterministic from carried content. **The per-resource hash-parity gate:**
recomputing the body text from synthesized blocks/chunks must reproduce the same content the
production read path serves today, per resource, before cutover proceeds.

**Open residue.** `block_chunks_at_revision`-style historical reads (content-block plan-Q2) are
explicitly post-migration; the archive holds history meanwhile.

## Adjudication 9 — Read-surface homes

**Delta.** The artifact has no graph/search/URI SQL; production surfaces need all three. The
artifact's silence on graph reads was a gap, not a rejection.

**Call (ratified), using the settled carve-out test** (*does the kernel interpret content?*):
- Graph traversal/neighbors over edges with visibility gating → **kernel** (temper-substrate
  access-gated reads — structure, not interpretation).
- FTS / unified search → **Domain-A operational**, rebuilt in the API tier against the new
  columns (title-only weight-A; doctype filters become property lookups — data-model §6).
- URI/addressing construction → **temper-workflow** (already settled).

**Migration-time floor (binding).** No functionality regression at cutover: today's FTS, vector
search, and graph reads carry, rebuilt against the new schema in the homes above. Cutover does
not wait on the successor design below.

**Named successor design unit — the agent information-access read surface.** The current
read tooling is query-shaped (terms, cosine, label filters); the converged substrate makes reads
**positional** — standing at resources already acquired, a region, or a charter question and
asking what's near, what coheres, where the boundaries are, what to trust. This is the
projection-class lineage taken up rigorously (research pair
`2026-05-23-projection-classes-as-functions-*`, temper context: orientation, recall, wayfinding,
recognition, composition, boundary-sensing, translation, trust-calibration as family-level
attention shapes; a projection-class is a function of substrate × position × lens × intent).
Inputs enumerated so none silently drop: neighborhood traversal from acquired
resources/engaged concepts; properties-as-salience at **both resource and edge grain**; regions
and lenses as retrieval surfaces; coherence/salience evaluation along traversal; tsvector +
pgvector as components of these functions, not the whole surface. Constraint inherited from the
carve-out test: the kernel serves structure and access-gated projections — interpretation stays
above it. This design unit is its own spec, after (or alongside) cutover — never settled by
migration-time implementation choice.

**Open residue.** The successor design unit above (tracked, not optional).

## §D — Deployment adjudication

**Delta.** The charter pre-committed "surface-at-a-time cutover with a short dual-write window."
Decision 0's synthesis-from-state changes the safety calculus: the new substrate can be rebuilt
from live production **at any moment, repeatedly** — a Neon branch + synthesis run is a complete,
free rehearsal. Blast radius is 2 (Pete + Claude); the real risk is silent divergence in the
system used to share working context.

**Call.** **Hard cutover with rehearsed synthesis. No dual-write window — this supersedes the
charter's framing line** (the charter's own question 1 anticipates exactly this kind of moved
call; the charter seed should be amended in the same change that lands this spec).

**Binding sequence.**
1. **Port in charter order as readiness sequencing** (api → cli/mcp → ui): each surface is built
   and proven against a rehearsal substrate — a Neon branch with the artifact schema +
   synthesis run — behind the **parity-read harness** (legacy reads answered identically from the
   new substrate).
2. **Rehearse repeatedly**: synthesis + per-resource hash-parity gate (§8) + parity-read harness
   on fresh branches until boring.
3. **One cutover**: brief write-freeze → final synthesis from live state → hash-parity gate →
   deploy new code (api+mcp together; CLI rebuilt same day; UI deploy) → legacy schema renamed
   into `legacy`, pre-cutover Neon branch taken.
4. **Rollback story** (boring by design): intact `legacy` schema in-place, the Neon branch, and
   re-runnable synthesis. Dual-write code is never written.
5. Crate extraction (`temper-substrate` / `temper-workflow`) comes **last**, against the stable
   post-cutover schema (unchanged from the goal's sequence).

**Why not dual-write:** its safety argument is strictly weaker than re-synthesis (comparing two
live write paths vs regenerating one store from the other and diffing), and it adds a second
write path that can itself diverge — the exact failure being guarded against.

**The cutover gate (decided 2026-06-13).** The mechanism behind "flip" in step 3 is an
**in-DB backend-selection config flag** — a single setting (a one-row config table, set/swapped
by a trivial migration) read at surface startup that chooses which substrate the surfaces
dispatch reads/writes to. Chosen over a connection-string swap or a separate-database promotion:
temperkb.io is single-tenant Vercel+Neon, synthesis writes additively into the **same** Neon
database, and the flag is environment-scoped so rehearsal-on-a-branch flips independently of
production. Cutover is then literally one config change + one redeploy.

**What this buys: incrementality holds until the flip.** The flag makes the merge boundary and
the production-behavior boundary separable, so the PR-over-PR model survives almost the whole arc:

- **Chunks 1–3 change zero production behavior** and merge freely. Chunk 1 is `temper_next`
  artifact-namespace only (prod's `public` never reads it). Chunk 2's migrations stay **strictly
  additive** — new tables/schema alongside the live ones, synthesis an explicitly-invoked
  operation, never a migrate-time side effect (a destructive migration here would move the
  blocker earlier — the discipline is load-bearing). Chunk 3 is read-only parity tooling.
- **Chunk 4 (surface ports) stays incremental *because* of the flag**: each surface lands able
  to read/write the new substrate but **gated OFF in production**, so every port PR is
  dead-pathed in prod and merges one-reviewed-at-a-time. Without the gate, the first repointed
  live surface would force all of them in one deploy — the surfaces share one Postgres and
  dual-write is rejected (split-brain: some writes to new tables, some to old, neither store
  complete). The gate is what avoids that.
- **The flip is the only irreducible atomic step**, and it is small: write-freeze → final
  synthesis → set the flag (one deploy, all surfaces switch together) → rename old schema aside.
  Rollback = flip back / old schema intact / Neon branch.

Two adjacent changes are *not* part of the flip: the `temper-core` shared-type changes (§5
`ResourceRef` collapse, `ManagedMeta` genericization) are **compile-time** atomic — one PR
updating all callers, not a runtime cutover; and crate extraction is post-cutover (step 5).

## Out of scope

- The migration plan's task-level decomposition (the successor plan, via writing-plans).
- `kb_team_contexts` DDL + scenario proofs (artifact amendment, lands access-scaffold-first per §2).
- Surface UX quality work (search ergonomics, command redesign) beyond the §5 contract.
- The agent information-access read-surface design (§9's named successor unit — its own spec).
- Crate-extraction internals (the data-model spec's topology stands; execution is post-cutover).
- The WS5 remainders (Lance-Williams perf; finding-4 watermark index rides into the migration
  schema work where natural).

## Connections

- Charter: `temper-convergence` seed (amend framing line 27 per §D)
- Delta master: `2026-06-01-data-model-reconciliation-design.md` (A1's sync rationale now
  historical per §7; "kb_cogmaps already exists" corrected to `kb_scopes` per Method)
- Access: `2026-06-02-access-capability-model-design.md`, `2026-06-11-access-scaffold-scenario-proof-design.md`
- Ledger: `2026-06-09-event-payload-formalization-design.md`
- Content: `2026-06-03-content-block-primitive-design.md`, `2026-06-08-…-event-firing-parity-design.md`
- Goal record: `substrate-kernel-to-cognitive-map` WS6 (update status line when this lands)
