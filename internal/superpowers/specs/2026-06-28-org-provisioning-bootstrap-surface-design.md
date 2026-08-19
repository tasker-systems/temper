# Org provisioning + bootstrap surface — design spec

**Status:** Design (plan/large). Grounded against current `main` 2026-06-28.
**Research direction:** `Org provisioning + bootstrap — design direction (self-hosted rollout)` (`019f1066`) — the seven decisions + two grounding passes are settled there; this spec turns them into a sequenced, code-anchored plan.
**Goal:** `substrate-kernel-to-cognitive-map` (Theme B: team provisioning → team corpus → manual stewardship → auto-growth).
**Template:** `docs/guides/l0-content-delivery.md` (PR #200) — the grant→reconcile→re-lock shape this generalizes.

## 1. Problem

A self-hosted temper install can be **stable but blank** — DB up + schema migrated, a compatible binary, MCP configured against Auth0/Okta — and still cannot reach a *usable org*. Resource **writes into** team contexts already work end-to-end (a team member can `temper resource create --context +team/ctx` today; `resolve_context_ref` is membership-gated, the substrate write path is owner-agnostic). The chain breaks **above** the write path: a team can't be created, a team-owned context can't be created, a non-L0 cognitive map can't be born, a cogmap can't be bound to a team, and gating/admin config is SQL-only.

This spec scopes **the admin/provisioning half of the WS7 operational surface** — the net-new layer parallel to the cogmap read/write surfaces already shipped. Multi-tenancy is out of scope (orgs self-host); **teams** are the unit that matters.

## 2. Grounded current state

Verified by four parallel code-reading passes on 2026-06-28. Every claim below carries its anchor.

### 2.1 Schema is ready; surfaces are not

- `kb_teams` (`migrations/20260624000001_canonical_schema.sql:182-187`): `id, slug, name, created`. **No owner column, no `auto_join_role`.**
- `kb_team_members` (`:191-198`): `team_id, profile_id, role, created`; PK `(team_id, profile_id)`. `team_role` enum = `owner|maintainer|member|watcher` (`:86`).
- `kb_teams_parents` (`:203-208`): DAG (`child_id, parent_id`, `CHECK child<>parent`).
- `kb_contexts` (`:159-167`): `owner_table VARCHAR(64) CHECK (owner_table IN ('kb_profiles','kb_teams'))`, `owner_id UUID`, `UNIQUE (owner_table, owner_id, slug)`. **The CHECK already admits `kb_teams`.**
- `kb_team_cogmaps` (`:254-259`): `(cogmap_id, team_id)` PK.
- `kb_system_settings` (`:345-354`): singleton (`id=1`), `access_mode ∈ {open, invite_only}` default `open`, `gating_team_slug` **NULL by default**.
- `kb_profiles.system_access` (`:122-131`): enum `system_access = none|approved|admin` (`:97-102`), default `none`.

### 2.2 RBAC DAG (confirmed direction)

- `team_ancestors(team)` (`20260624000002_canonical_functions.sql:29-39`): recursive walk **UP** (self ∪ ancestors).
- `profile_effective_teams(profile)` (`:49-52`): direct memberships only — no traversal. Root membership is a **real row**, not derived.
- `resources_visible_to` / `can_modify_resource` (`:125-188`): grants on an **ancestor** team inherit **DOWN** to descendants (via `team_ancestors` over each effective team). → **Decision #4 holds: the everyone-team must NOT be the DAG root**, else every sub-team over-shares; it is a flat parentless audience node.
- Cogmap reach (`resources_accessible_to_cogmap`, `:222-241`): interior ∪ **intersection** of all joined teams' `vis_team`; empty join → default-CLOSED.

### 2.3 The auto-join pattern already exists, hardcoded

`sync_system_membership()` (`canonical_functions.sql:58-81`, trigger `:79-81`) fires `AFTER INSERT OR UPDATE OF system_access ON kb_profiles`: on `system_access='none'` it DELETEs the membership; otherwise it UPSERTs a `kb_team_members` row in the `temper-system` root — `owner` if `admin`, else `watcher` — `ON CONFLICT DO UPDATE`. It **has no backfill** (temper-system predates all profiles). This is *exactly* the everyone-pool ≡ cogmap-audience Venn, already how L0's audience works. **Decisions #2/#3 = generalize this trigger**, not build it from scratch.

### 2.4 Admin gating

- `has_system_access(profile)` (`:1388-1407`): `open` → always true; `invite_only` → member of gating team.
- `is_system_admin(profile)` (`:1409-1425`): **OWNER of the gating team**. Does *not* read `kb_profiles.system_access`; returns false for everyone when `gating_team_slug IS NULL`. (The `system_access='admin'` → owner link is made by the trigger, not by this function — so the two compose: set `system_access='admin'` *and* point `gating_team_slug` at temper-system and `is_system_admin` resolves true.)
- `require_cogmap_write_admin` (`access_service.rs:46-91`): write requires `is_system_admin` when target is L0 (unconditionally) **or** the cogmap is joined to the gating team; otherwise ungated here. Fail-CLOSED: L0 immutable until an admin exists.
- Admin request review handlers exist (`handlers/access.rs:87-126`, `GET/PATCH /api/access/admin/requests`, gated by `is_system_admin`) but **no CLI binding**. Approval inserts a `'watcher'` row (`access_service.rs:350`) and does **not** touch `system_access`.

### 2.5 The surface gaps (net-new)

| # | Surface | Today | Missing |
|---|---------|-------|---------|
| Team create | none | only `temper team {join,status,leave}` (`commands/team.rs`); only team-write anywhere is approval's hardcoded watcher | create / member-assign / `--auto-join-role` at CLI/API/client; no Backend-trait command |
| Team-context create | blocked | `context_service::create(pool, profile_id, name)` hardcodes `'kb_profiles'` (`:259-260`); slug check hardcoded (`:228`); `ContextCreateRequest { name }` only | owner param threaded through CLI→client→handler→service |
| Cogmap genesis | substrate-only | `cogmap_genesis` (`temper-substrate/src/events.rs:328`) called by migrations + scenario loaders; surfaces = reads + `reconcile` (PUT) | `POST /api/cognitive-maps` + `temper cogmap create` + MCP, mirroring reconcile's client-embed |
| Cogmap↔team bind | scenario-only | `kb_team_cogmaps` written by L0 migration + `scenario/access/loader.rs:363-369` | bind surface (API/CLI/MCP) |
| Admin/settings | SQL-only | `kb_system_settings` + `system_access` set by seed/manual SQL | admin-gated settings write + promote-admin; first-admin stays irreducible root |

### 2.6 Write-path architecture (a real choice for Chunk 2)

Writes are supposed to route **surface → operations command → `DbBackend` → substrate persistence**; the Backend trait lives at `crates/temper-workflow/src/operations/backend.rs` and has resource/edge/cogmap/invocation commands but **no team command**. Two precedents diverge:
- **Resource/edge/cogmap**: through the Backend trait, event-emitting, invocation-enveloped.
- **Context create**: deliberately **service-direct**, *no* event emission — "contexts are infrastructure, product decision 5" (`context_service.rs:11`). The only existing team write (approval) is likewise service-local in `access_service.rs`.

**Settled here:** team lifecycle and team-context creation are **provisioning/infrastructure**, not knowledge-graph mutations, and follow the **context precedent — service-direct, no event emission**. They do *not* get Backend-trait commands. Rationale: they create *containers*, not graph content; the Backend trait's value (event sourcing, invocation envelopes, merkle diffing) buys nothing here, and the approval write already sets the service-direct precedent for `kb_team_members`. Cogmap **genesis** is the exception — it *is* graph content (a telos resource + map), already flows through substrate events, and gets a Backend-trait command mirroring `reconcile`.

## 3. Settled decisions (from `019f1066`, condensed)

1. **Capability model = reuse team roles, no new schema.** Anyone may create a **root (parentless)** team. Creating a **child** team or a **team-owned context** requires `owner`/`maintainer` on the parent. Pure authz over `kb_team_members.role` + the `kb_teams_parents` DAG.
2. **Auto-join teams** via a `auto_join_role team_role` column (NULL = not auto-join; default applied `watcher`), admin-gated to set. Purpose: an always-complete "everyone" pool = the whole-Venn audience for org-wide cogmaps (bound via existing `kb_team_cogmaps`).
3. **Enrollment is idempotent, multi-sited.** `ensure_auto_join_memberships(profile)` (insert watcher rows for every auto-join team, `ON CONFLICT DO NOTHING`) at profile-provision **and** access-grant; `backfill_auto_join_team(team)` on enable. Idempotent everywhere → open/invite_only difference stops mattering.
4. **The everyone-team is NOT the DAG root** (flat parentless audience node; org cogmaps bound explicitly). Confirmed by §2.2.
5. **Bootstrap = a declarative "install profile"** applied through the same idempotent reconcile/create machinery — L0 manifest + org-identity manifest + auto-join team spec + cogmap↔team bindings.
6. **The bootstrap orchestrator lives OUTSIDE temper-cli but utilizes it** — YAML desired-state + a thin external applier looping idempotent `temper` commands + the SQL root. Not a temper-cli subcommand.
7. **Sequencing: SoP-first** (the runbook doubles as the applier spec). (a) surface the admin primitives; (b) write the bootstrap SoP using them + existing L0 reconcile; (c) optionally graduate to a guided applier later.

## 4. The roadmap — seven chunks

Each chunk is sized for a single build session. Role-gating, surfaces, and the validation gate are stated per chunk. Anchors are current-state; the "Change" is the net-new.

### Chunk 1 — Auto-join generalization (substrate)
**Depends on:** nothing. **Surfaces:** none (DB only). **Gating:** N/A (mechanism).

- **Change:** additive migration. Add `auto_join_role team_role` to `kb_teams` (NULL = not an auto-join team) and set it `'watcher'` on `temper-system` (an ordinary auto-join team — resolved Q-A, no special case). **Generalize** `sync_system_membership()` from the hardcoded `temper-system` slug to *every* team with `auto_join_role IS NOT NULL`, **keeping** the `admin→owner` mapping uniformly (`system_access='admin'` → `owner`, else the team's `auto_join_role`) — preserves the 2-UPDATE root step + the test-harness admin-minting (`cogmap_authz_test.rs:33-41`). Gate enrollment on `has_system_access(profile)` (computed), so open-mode profiles auto-join (decision #3's everyone-pool) — a deliberate behavior change rippling through visibility tests. On losing access, DELETE the profile's memberships across all auto-join teams (resolved Q-C). Add `ensure_auto_join_memberships(profile)` (idempotent, `DO UPDATE`, called by the trigger and the future access-grant site) and `backfill_auto_join_team(team)` (`DO NOTHING`, enroll all `has_system_access` profiles when a flag is newly enabled — the gap the trigger lacks); call `backfill_auto_join_team` for temper-system in this migration. **No seed change.** Defer the invite_only access-grant call site (`review_request` → `ensure`) to Chunk 6 where approval is surfaced.
- **Anchors:** `canonical_functions.sql:58-81`, `:1388-1425`; `canonical_schema.sql:182-187`.
- **Gate:** new `#[sqlx::test]` (ephemeral DB) — flag a team auto-join, provision a profile → membership appears; enable the flag on a team with pre-existing eligible profiles → backfill enrolls them; revoke access → auto-join rows removed; all operations idempotent on re-run. Existing temper-system behavior unchanged (regression).

### Chunk 2 — Team lifecycle surface
**Depends on:** Chunk 1 (for `--auto-join-role`). **Surfaces:** CLI + API + client (service-direct per §2.6). **Gating:** root = any authenticated profile; child = `owner`/`maintainer` on the parent (new `can_create_child_team(parent, profile)` check); setting `auto_join_role` = `is_system_admin`.

- **Change:** `temper team create <slug> [--name] [--parent +team/…] [--auto-join-role <role>]` and `temper team add-member <team> <profile> --role <role>` (+ `team list`). New `team_service` with `create_team` (insert `kb_teams`, link `kb_teams_parents` if child, insert creator as `owner` in `kb_team_members`) and `add_member`. New `POST /api/teams`, `POST /api/teams/{id}/members`, `GET /api/teams`. New client `TeamsClient`. Auth-before-writes: role check precedes any insert.
- **Anchors:** `commands/team.rs` (current join/status/leave), `access_service.rs:305-365` (the existing team-write precedent), `routes.rs`.
- **Gate:** e2e through real Axum + Postgres — a profile creates a root team (becomes owner); a non-owner is `Forbidden` from creating a child; an owner creates a child; `--auto-join-role` rejected for non-admin, accepted for admin and visibly enrolls via Chunk 1.

### Chunk 3 — Team-context creation
**Depends on:** Chunk 2. **Surfaces:** CLI + API + client (service-direct). **Gating:** `owner`/`maintainer` on the owning team.

- **Change:** parameterize `context_service::create` to take an owner descriptor (`ContextOwnerRef` — profile or team) instead of hardcoding `'kb_profiles'`; generalize the slug-collision check (`:228`) to the parameterized owner. Extend `ContextCreateRequest` with an optional owner ref (default = caller's profile, preserving today's behavior). Add `temper context create <name> [--owner +team/…]`. Role-gate team-owned creation.
- **Anchors:** `context_service.rs:228,252,259-260`; `types/context.rs:55-57`; `cli.rs` ContextAction; `handlers/contexts.rs:43-51`.
- **Gate:** e2e — owner/maintainer creates a `+team/ctx` context; member/non-member is `Forbidden`; a team member can then `resource create --context +team/ctx` (the already-working write path) into the freshly-created context; profile-owned creation still works unchanged.

### Chunk 4 — Cognitive-map genesis surface
**Depends on:** nothing (parallel to 1-3). **Surfaces:** CLI + API + MCP + new Backend-trait command (§2.6 exception). **Gating:** `is_system_admin` (matches L0/reconcile posture; revisit in Deferred).

- **Change:** `POST /api/cognitive-maps` + `temper cogmap create --manifest <telos.yaml> [--name]` + MCP `cogmap_create`, all reaching `cogmap_genesis` through a new `create_cognitive_map` Backend command that mirrors `reconcile_cognitive_map` (`db_backend.rs:1202-1267`): client-side embed of the telos manifest (like `cogmap reconcile`), invocation-enveloped substrate write. Returns `(cogmap_id, telos_resource_id)`.
- **Anchors:** `temper-substrate/src/events.rs:328-463` (genesis fn + callers), `handlers/cognitive_maps.rs:46-71` (reconcile template), `commands/cogmap.rs:65-90`, `cli.rs:443-476`, `tools/cognitive_maps.rs`.
- **Gate:** e2e (embed feature) — admin genesis's a new map from a telos manifest; non-admin is `Forbidden`; the new map is immediately reconcilable; re-running genesis is rejected/idempotent (Open Question B).

### Chunk 5 — Cogmap↔team bind surface
**Depends on:** Chunks 2 + 4. **Surfaces:** CLI + API + MCP. **Gating:** `is_system_admin` (steady-state revisit in Deferred).

- **Change:** `POST /api/cognitive-maps/{id}/teams` (+ DELETE to unbind) writing `kb_team_cogmaps`; `temper cogmap bind <cogmap> +team/…` / `unbind`; MCP `cogmap_bind`. Small, additive.
- **Anchors:** `kb_team_cogmaps` (`canonical_schema.sql:254-259`), `scenario/access/loader.rs:363-369` (the only current writer), `l0_kernel_cogmap.sql:47-54` (the L0 bind precedent).
- **Gate:** e2e — admin binds a Chunk-4 map to a Chunk-2 everyone-team; `resources_accessible_to_cogmap` then reflects the team's `vis_team`; unbind reverses it; non-admin `Forbidden`.

### Chunk 6 — Admin / system-settings surface
**Depends on:** nothing (parallel). **Surfaces:** CLI + API + client. **Gating:** `is_system_admin`; the **first** admin + gating stays the irreducible operator-with-DB-credentials root step.

- **Change:** admin-gated `PATCH /api/access/admin/settings` (set `access_mode`, `gating_team_slug`, `instance_name`, terms) + `temper admin settings …`; "promote another admin" = admin-gated set of another profile's `system_access` (or owner-grant on the gating team) — `temper admin promote <profile>`. Also bind the existing admin request-review handlers to a CLI (`temper admin requests {list,review}`). The first admin + initial `gating_team_slug` remain the two-UPDATE SQL root step (documented, not surfaced — nothing to authenticate against yet).
- **Anchors:** `kb_system_settings` (`canonical_schema.sql:345-354`), `handlers/access.rs:87-126`, `access_service.rs`, `docs/guides/l0-content-delivery.md:108-125`.
- **Gate:** e2e — the 2-UPDATE root step promotes the first admin; that admin then promotes a second admin via the surface; settings round-trip; non-admin `Forbidden` on all.

### Chunk 7 — Bootstrap install-profile + SoP runbook (capstone)
**Depends on:** Chunks 1-6 (each surface replaces a manual step). **Surfaces:** docs + an external applier (script), **not** a temper-cli subcommand.

- **Change:** a declarative `install-profile.yaml` (L0 kernel manifest ref + org-identity manifest + auto-join team spec + cogmap↔team bindings) and a thin external applier that loops the now-surfaced idempotent `temper` commands + the SQL root step. The SoP runbook (`docs/guides/org-bootstrap.md`) is authored **first** (SoP-first, decision #7) and doubles as the applier spec; it starts as the manual sequence and each chunk's surface replaces a manual step. Idempotency is inherited from the primitives — no state backend initially; plan/diff (TF-like) deferred.
- **Anchors:** template = `docs/guides/l0-content-delivery.md`; same shape as `schema-artifact/` scenario-apply.
- **Gate:** an operator (or CI harness) takes a blank install to a usable org by following the SoP end-to-end: 2-UPDATE root → `temper team create` the everyone team (`--auto-join-role watcher`) → genesis + reconcile an org-identity cogmap → bind it to the everyone team — every step a surfaced command except the irreducible root. Acceptance criteria of the task met.

## 5. Dependency graph & sequencing

```
Chunk 1 (auto-join) ──┐
                      ├─▶ Chunk 2 (team create) ──▶ Chunk 3 (team-context)
                      │                          └──▶ Chunk 5 (bind) ◀── Chunk 4 (genesis)
Chunk 4 (genesis) ────┘
Chunk 6 (admin settings) ── parallel ──────────────────────────────────┐
                                                                        ▼
                          Chunk 7 (SoP runbook + applier) ◀── needs 1,2,3,4,5,6
```

**Topological head = Chunk 1** (zero dependencies; Chunk 2's `--auto-join-role` writes the column it adds). Chunks 4 and 6 can run in parallel with 1-3. Chunk 7 is the capstone.

**First build task = Chunk 1 — Auto-join generalization.** It is the lowest-risk, most self-contained piece (additive migration, generalizes an existing proven trigger, no surface stack), it sits at the head of the topological order, and it de-risks the load-bearing trigger change *before* surfaces are built on top of it.

## 6. Open questions — resolved 2026-06-29

- **A. temper-system follows the auto-join convention — no special case (RESOLVED; corrected after grounding 2026-06-29).** Set `kb_teams.auto_join_role='watcher'` on `temper-system`; it is an ordinary auto-join team. Generalize the trigger to loop over **every** team with `auto_join_role IS NOT NULL` (dropping the hardcoded `'temper-system'` slug — *that* is the special case being removed). **KEEP the `admin→owner` mapping, applied uniformly to all auto-join teams**: `system_access='admin'` → `owner`, else the team's `auto_join_role`. Gate enrollment on `has_system_access(profile)` (computed) not the raw column, so in **open** mode every profile auto-joins every auto-join team — decision #3's everyone-pool (a deliberate behavior change: visibility tests that assume profiles are *not* temper-system members shift, and are reconciled as intended-new-behavior). On losing access, DELETE the profile's memberships across all auto-join teams (Q-C). **No seed change.**
  - *Correction:* an earlier draft said "remove `admin→owner`; seed grants `system` owner explicitly." That was wrong — it assumed the seed makes `system` an owner via the trigger. Grounding shows the seed (`20260624000003`) runs **before** `temper-system` exists (created in `20260625000001`), so the trigger no-ops at seed time and `temper-system` has **zero members** on a fresh install until the operator's explicit grant. Removing `admin→owner` would break `cogmap_authz_test.rs:33-41` (`admin_profile` mints an admin via `UPDATE system_access='admin'`, relying on trigger→owner) + the e2e access-gate tests. Keep it.
- **B. Genesis identity = manifest-supplied uuidv7 (RESOLVED — not a real open question).** `cogmap_genesis` already takes the cogmap id *and* telos-resource id as **inputs** (`Fired::CogmapGenesis { cogmap, telos_resource }`, `events.rs:328`), exactly as L0's migration supplies hardcoded ids. Genesis is therefore "create-if-absent at the manifest's uuid"; a second genesis with the same id no-ops; all content edits flow through **reconcile** keyed on that identity. Chunk 4 only nails the create-path behavior (existing id → no-op vs error) and where the manifest declares the id.
- **C. Revoke must remove ALL auto-join rows (RESOLVED → acceptance criterion).** The generalized trigger's `system_access='none'` branch must DELETE across *every* auto-join team, not just temper-system. Verify in code + assert in the Chunk 1 `#[sqlx::test]`.
- **D. Owner descriptor = typed `ContextOwnerRef` end-to-end (RESOLVED).** Parse-don't-validate; no flat `(owner_table, owner_id)` wire pair. The remaining "how" is a Chunk 3 implementation detail.

## 7. Deferred (named, not in this plan's first cut)

- **Cogmap-write gate vs team roles.** `require_cogmap_write_admin` is admin-only when a cogmap is root/gating-joined. Org-wide cogmaps bound to the everyone-team will eventually want **maintainers of that team** to write, not only system admins — a real reconciliation downstream of provisioning. Chunks 4/5 use `is_system_admin` as the interim gate.
- **Plan/diff (Terraform-like) applier semantics** for Chunk 7 — start stateless/idempotent, grow later.
- **Graduating Chunk 7 to a guided `temper admin init` subcommand** — decision #6 keeps it external first.

## 8. Acceptance criteria (the eventual build, from the task)

- An operator takes a blank install to a usable org via: 2-UPDATE root step → `temper team create` the everyone team (auto-join watcher) → genesis + reconcile an org-identity cogmap → bind it to the everyone team — all surfaced commands except the irreducible root.
- Role-based gating enforced (root vs child team / team-context creation).
- Auto-join enrollment idempotent across open/invite_only + backfills on enable.
- A bootstrap SoP runbook exists (the install-profile applier spec).
