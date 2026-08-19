# Democratize cogmap creation: non-admin create + bind to grant-held teams

**Date:** 2026-07-07
**Task:** `019f37fb` (temper context) · **Goal:** teach-temper-to-agents (`019f373e`)
**Mode:** plan / medium

## Problem

`cogmap_create` (genesis) and `cogmap_bind` are **system-admin only** today. The gate is
`is_system_admin`, checked on each surface (MCP `cognitive_maps.rs`, HTTP
`handlers/cognitive_maps.rs`; the CLI `temper cogmap create/bind` routes through the API and
inherits it). This blocks any non-admin team from standing up its own non-system-root cognitive
map — surfaced while designing the telos-differentiation experiment (2026-07-06), where a team
needed to create + bind its own maps without root involvement.

Two facts make the naive "drop the gate" wrong:

1. **Genesis fires under the *system* actor and mints no grant for the creator.** After a
   non-admin creates a map they cannot author it — `cogmap_service::authorable_by_profile`
   requires an explicit `can_write` grant on the map. Democratized create is inert without a
   creator-grant.
2. **Genesis accepts a caller-supplied `cogmap_id` / `telos_resource_id`.** Today only the
   idempotent-no-op protects *already-seeded* reserved ids (L0 kernel
   `00000000-0000-0000-0005-000000000001`, its telos `…0002`); nothing rejects an *unseeded*
   reserved-range id supplied by a non-admin.

## What's already done (no work)

`cogmap_grant` / `cogmap_revoke` already gate on `is_system_admin OR can('grant', kb_cogmaps, …)`
via `access_service::can_administer_grant`. The grant surface is already democratized; this spec
does not touch it.

## Model facts this design rests on

- **`kb_access_grants` subjects are `kb_resources` / `kb_contexts` / `kb_cogmaps` only — never
  `kb_teams`.** Teams can *receive* grants (principal) but cannot be the *subject* of one, so
  there is no "can_grant **on** a team" mechanism. Team authority is the role ladder
  **Owner > Maintainer > Member > Watcher**, with the existing `team_service::can_manage(role)` =
  `Owner | Maintainer`.
- **`require_cogmap_write_admin`** already gates writes to the reserved L0 kernel map and to any
  map **joined to the gating/root team** (fail-closed). Binding a map to the root team is
  therefore an escalation: it flips the map into the admin-write regime.
- The `DbBackend` carries the caller `profile_id`; `create_cognitive_map` runs one serializable
  transaction, so a creator-grant can be minted atomically with genesis.

## Design

Three behavioral changes, all enforced **server-side** so MCP / HTTP / CLI inherit them.

### ① Create (genesis) — democratized, server-mint-only for non-admins

- **Drop the `is_system_admin` surface gate** on create (MCP + HTTP). `require_profile()` /
  authenticated-caller remains the only bar: **any authenticated profile may genesis a
  non-reserved map.**
- **Explicit-id policy (reserved-id hardening):** the backend accepts a caller-supplied
  `cogmap_id` / `telos_resource_id` **only when the caller is `is_system_admin`**. For a
  non-admin, any supplied id is **ignored and the server mints** fresh uuidv7 ids. This removes
  the reserved-range attack surface without range-checking logic — a non-admin can never place a
  map at a chosen id. Explicit-id genesis (reproducible/migration seeding) stays operator work.

### ② Creator grant — atomic with genesis

- Inside the genesis transaction, mint a `kb_access_grants` row on the new map:
  `subject = ('kb_cogmaps', new_cogmap_id)`, `principal = ('kb_profiles', caller)`,
  `can_read + can_write + can_grant` (all true; `can_delete` false). The map is authorable and
  further-grantable by its creator the instant it exists.
- Cross-references the settled `can_write` precedence decision (task `019f3739`): the creator
  holds an explicit grant, which is exactly what `authorable_by_profile` checks.
- On the **admin explicit-id path** the creator-grant is still minted for the acting admin
  (harmless; keeps one code path). Genesis remains idempotent: re-genesis at an existing id is a
  no-op and mints no second grant.

### ③ Bind — democratized with a two-sided gate

Replace the `is_system_admin` check at the top of `cogmap_service::bind_team` with:

```
is_system_admin
  OR ( can_manage(role_on_team(caller, team))        // Owner | Maintainer, direct membership
       AND can('grant', 'kb_cogmaps', cogmap_id)     // caller must administer the MAP too
       AND team_id != gating/root team )             // escalation guard
```

- **Team side — `can_manage` (Owner|Maintainer), direct membership only.** Same authority bar as
  `add_member` / child-team creation. No `kb_teams_parents` ancestor expansion in v1 — matches
  `role_on_team`'s direct lookup.
- **Map side — `can('grant', kb_cogmaps, cogmap)`.** Without it, any team Owner could attach
  *someone else's* map to their team, silently widening their team's reach to a map they don't
  control. The creator-grant from ② satisfies this for maps you made; `is_system_admin` short-circuits
  the whole predicate for operators.
- **Root-team guard.** Binding to the gating team stays admin-only: a non-admin Maintainer of the
  root team must not be able to flip a map into the `require_cogmap_write_admin` regime.

### ③′ Unbind — symmetric

`cogmap_service::unbind_team` gets the **same two-sided gate** (a principal who could bind can
unbind), with the **same root-team guard** (unbinding from the gating team stays admin-only).

### Preserved unchanged

- `require_cogmap_write_admin` (L0 + root-join, fail-closed).
- `cogmap_grant` / `cogmap_revoke` (already democratized).
- Genesis idempotent-no-op and the serializable-transaction shape.

## Where the code changes land

| Layer | File | Change |
|---|---|---|
| Backend | `temper-services/src/backend/db_backend.rs` `create_cognitive_map` | admin-only explicit-id (else server-mint); mint creator `read+write+grant` in-tx |
| Service | `temper-services/src/services/cogmap_service.rs` `bind_team` / `unbind_team` | two-sided gate + root-team guard replacing `is_system_admin` |
| Service (helper) | `temper-services/src/services/access_service.rs` or `team_service.rs` | reuse `can_manage` / `role_on_team` / `can(...)`; add a small `can_bind_cogmap_to_team` seam if it reads cleaner |
| MCP surface | `temper-mcp/src/tools/cognitive_maps.rs` `cogmap_create` | drop `is_system_admin` gate; correct doc-comment |
| HTTP surface | `temper-api/src/handlers/cognitive_maps.rs` genesis (~L105) | drop `is_system_admin` gate; correct doc-comment |
| CLI help/docs | `temper-cli/src/cli.rs` / `commands/cogmap.rs` | correct stale "admin-only" help text on `create` / `bind` |

The gate logic centralizes in the **service layer** (one `bind_team`/`unbind_team` both surfaces
call, mirroring how `grant_capability` centralizes `can_administer_grant`). Surfaces only drop
their local admin check and translate errors.

## Testing (e2e is the honest layer — spans auth + service + DB)

1. Non-admin creates a map → succeeds; creator can immediately author it (write a node/facet) —
   proves the creator-grant.
2. Non-admin supplies an explicit / reserved-range id → id ignored, server-minted id returned.
   Admin supplies an id → honored.
3. Team **Maintainer** binds their own map (holds `can_grant`) to their team → succeeds;
   **Member** / **Watcher** → forbidden.
4. Non-admin binds a map they do **not** hold `can_grant` on → forbidden (map-side gate).
5. Non-admin binds any map to the **gating team** → forbidden (escalation guard).
6. Unbind mirrors: Maintainer unbinds their map → succeeds; unbind from gating team → admin-only.
7. Regression: L0 kernel and root-joined maps remain admin-only to write
   (`require_cogmap_write_admin` untouched).

Unit tests: `bind_team`/`unbind_team` gate predicate; the admin-vs-non-admin id-mint branch in
`create_cognitive_map`.

## Risks

- **Grant proliferation:** every democratized create writes one `kb_access_grants` row. Expected
  and bounded (one per map); no cleanup path needed beyond existing revoke.
- **Map-side `can_grant` is the load-bearing check** for bind safety — the e2e in test 4 is the
  guard against a regression that would let team owners annex foreign maps.

## Out of scope

- Per-profile create quotas / a dedicated `create_cogmap` capability (YAGNI — revisit if abuse
  appears).
- Team-hierarchy inheritance of bind authority (`kb_teams_parents` expansion).
- Any change to `cogmap_grant` / `cogmap_revoke` (already democratized).
