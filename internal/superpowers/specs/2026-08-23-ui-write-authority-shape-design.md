# How a reader's authority to change a thing reaches the surface that offers the change

**Task:** [How does a reader's authority to change a thing reach the surface that offers the change?](https://temperkb.io/r/01a03044-5767-7d90-9984-ee1b697ae08c) · `01a03044-5767-7d90-9984-ee1b697ae08c` — `enables`, plan/medium
**Goal:** `01a0303a-0ee3-72e3-b2e6-6bed992dd22e` — *The reader changes their own work where they read it*
**Date:** 2026-08-23

> The durable copy of this spec is research resource `01a0304f-8117-7a60-be29-ad4ae50cdd19` in `@me/temper`; this file is the
> branch-reviewable twin. Where they disagree, the resource is authoritative.

## Problem statement

The goal's clause `a-change-is-offered-only-where-the-reader-holds-authority-to-make-it` has no
mechanism. Whether a reader may change a resource is computed at the write gate and answered nowhere
a surface can ask *before* offering. The register named two candidate shapes and chose neither,
recording the shape as an open question and noting: *"If the research says a third shape is better,
that is a good outcome and this task's framing was the thing that was wrong."*

That is what happened. The framing — *"how do we carry authority per resource"* — presupposes that
authority is a per-resource fact that must travel per resource. It is not.

## Grounding

Evidence first; the proposal cites it.

### The write gate is a floor plus a four-arm union

`migrations/20260804000020_profile_reachable_teams_write_gates.sql:75`:

```sql
CREATE OR REPLACE FUNCTION can_modify_resource(p_profile uuid, p_resource uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    -- Soft-delete WRITE floor: a tombstone is unmodifiable on every axis.
    SELECT EXISTS (SELECT 1 FROM kb_resources r WHERE r.id = p_resource AND r.is_active)
       AND EXISTS (
        WITH reachable_teams AS (SELECT team_id FROM profile_reachable_teams(p_profile))
        -- owned (the home confers modify to its OWNER; originator is provenance only, not access)
        SELECT 1 FROM kb_resource_homes h
         WHERE h.resource_id = p_resource AND h.owner_profile_id = p_profile
        UNION ALL  -- direct profile-anchored WRITE grant
        SELECT 1 FROM kb_access_grants g
         WHERE g.subject_table = 'kb_resources' AND g.subject_id = p_resource
           AND g.principal_table = 'kb_profiles' AND g.principal_id = p_profile AND g.can_write
        UNION ALL  -- team-anchored WRITE grant on a reachable (self-or-ancestor) team
        SELECT 1 FROM kb_access_grants g
         JOIN reachable_teams rt ON g.principal_id = rt.team_id
         WHERE g.subject_table = 'kb_resources' AND g.subject_id = p_resource
           AND g.principal_table = 'kb_teams' AND g.can_write
        UNION ALL  -- container-write cascade
        SELECT 1 FROM kb_resource_homes h
         WHERE h.resource_id = p_resource
           AND CASE h.anchor_table
                 WHEN 'kb_cogmaps'  THEN cogmap_authorable_by_profile(p_profile, h.anchor_id)
                 WHEN 'kb_contexts' THEN context_authorable_by_profile(p_profile, h.anchor_id)
                 ELSE false END
    );
$$;
```

### Arm 1 is already on the wire, byte-identically

`crates/temper-substrate/src/readback/mod.rs:575` selects `h.owner_profile_id` from
`JOIN kb_resource_homes h ON h.resource_id = r.id` (line 584), mapped to
`ResourceView.owner_profile_id` at line 610. That is the **same column, same join** as arm 1's
`h.owner_profile_id = p_profile`.

`ResourceView.is_active` (`crates/temper-core/src/types/resource_view.rs:132`) is the floor.

### The surface already holds the reader's identity

`packages/temper-ui/src/hooks.server.ts:143-145` fetches `/api/profile` on every request and sets
`event.locals.profile`. `session.ts:10` states the design reason: *"We do NOT store the temper
Profile in the cookie — it's fetched fresh on every request … to keep entitlements current."*

### Arm 4 is per-container, not per-resource

`context_authorable_by_profile(profile, anchor)` takes the **home**, not the resource. For any set of
resources sharing a home, it is one boolean.

### The oracle-of-existence pattern already exists in this repo

`crates/temper-services/src/backend/db_backend.rs:3396`:

```sql
SELECT context_authorable_by_profile($2, $1) AS "can_write!",
       shape_materialized_event_id AS "watermark: uuid::Uuid"
  FROM kb_contexts
 WHERE id = $1 AND anchor_readable_by_profile($2, 'kb_contexts', $1)
```

`.fetch_optional()` → `None` → `TemperError::NotFound`. Authority is computed **inside** a
read-gated query, so an unreadable container and a nonexistent one produce the identical answer.

The system-wide rule is stated as a CONFORM constraint at `readback/mod.rs:16`: *"CONFORMing to
production's `resources_visible_to(profile)` JOIN and its not-visible→404 deny."*

## Where each condition is answerable today

| Condition | Answerable from what the surface has? |
|---|---|
| Soft-delete floor (`is_active`) | **Yes** — already on `ResourceView` |
| Arm 1 · home owner | **Yes** — `owner_profile_id` on the wire + `locals.profile` |
| Arm 2 · direct write grant | No |
| Arm 3 · team write grant | No |
| Arm 4 · container cascade | No — **but one boolean per container** |

## Chosen approach

**Carry container authoring authority on the container read; compute the floor and arm 1 locally.**

1. **EXTEND** — the context read answers whether the reader may author into that context.
   *Authorized by:* the goal's clause `a-change-is-offered-only-where-the-reader-holds-authority-to-make-it`,
   which has no mechanism and requires one. The field goes on a response that **is already about
   containers** (`ContextRow` / `ContextRowWithCounts`, `crates/temper-core/src/types/context.rs:17,37`),
   which is why this does not incur the "authority on answers that never want it" cost.

2. **CONFORM** — the authority value is computed inside a query already gated on
   `anchor_readable_by_profile`, exactly as `db_backend.rs:3396` does, so a container the reader
   cannot read is absent rather than reported as unauthorized.
   *Cites:* `db_backend.rs:3396`, `readback/mod.rs:16`.

3. **CONFORM** — the surface derives per-resource offerability as
   `is_active && (owner_profile_id === locals.profile.id || containerCanWrite)`, from fields already
   on the wire. No per-resource authority field is added anywhere.
   *Cites:* `readback/mod.rs:575,584,610`; `resource_view.rs:132`; `hooks.server.ts:143-145`.

4. **CONFORM** — the write gate stays the sole authority. The surface's derivation decides only
   whether to *offer*; every write is still refused or admitted by `can_modify_resource` at
   `db_backend.rs:1929`. The surface is never the security boundary.

### Why not the two shapes the register named

- **Authority on every resource-bearing answer** — rejected. It changes wire shapes across many
  responses that will never want it, to carry a fact that is per-container for arm 4 and already
  present for arm 1.
- **A second question about a set of already-rendered resources** — rejected *for now*. It buys only
  arms 2 and 3 over the chosen approach, at the cost of a round trip, a first paint that does not
  yet know, and a new sensitive endpoint. It becomes correct when the residual below becomes
  material; it is not correct as the first step.

## Accepted remainder

**A reader whose only authority comes from arms 2 or 3 — an explicit per-resource write grant, or a
team grant on a specific resource — will be shown no control although they may write.**

- This is a **false negative**: it under-offers. The clause reads *"offered **only** where the reader
  holds authority"*, so the clause is not violated. `no-affordance-overstates-what-it-does` is not
  violated either — nothing overstates.
- **It is not measured.** How many resources are write-reachable *only* via arms 2/3 is a single
  query nobody has run. Naming it as an accepted remainder rather than a solved problem is the point.
- **It is the trigger condition for the second shape.** When per-resource grants are common enough
  that hiding controls from grant-holders is a real complaint, the set-shaped question is the answer,
  and this spec's approach is what it would extend rather than replace.

## Components affected

| Component | Change |
|---|---|
| `crates/temper-core/src/types/context*.rs` | A container-authority field on the context read shapes |
| Context read service | Compute it inside the existing read-gated query (pattern: `db_backend.rs:3396`) |
| `crates/temper-api` context handlers | Carry the field; regenerate OpenAPI + ts-rs artifacts |
| `packages/temper-ui` server layer | Derive offerability from `is_active`, `owner_profile_id`, `locals.profile.id`, container authority |

Generated artifacts are involved — `openapi.json`, the ts-rs tree, and the TS schema all drift from a
response-DTO change. See the `generated-artifacts` skill before claiming the work is done.

## Open questions and risks

- **Cogmap-homed resources.** Arm 4 branches on `anchor_table`; this spec addresses the context arm.
  Whether the cogmap arm needs the same treatment now is unresolved and depends on whether the write
  surface reaches cogmap-homed resources at all — the goal does not say.
- **Where the derivation lives.** One place in the UI's server layer, or a shared helper. It must be
  one copy: the goal's `no-surface-restates-a-vocabulary-it-could-read` is about vocabularies, but the
  same drift argument applies to a predicate transcribed from SQL into TypeScript. **This derivation
  is a transcription of arm 1 plus the floor, and transcriptions drift.** A test asserting the TS
  derivation against the SQL gate's behaviour is the mitigation, and it belongs to the build.
- **Staleness.** Container authority is read once per page load. A grant revoked mid-session leaves a
  control on screen. The write still refuses, so this is a false positive in *display* only — which is
  the direction that matters least, but it should be visible rather than silent.

## What this enables

This is an `enables` task: it builds the mechanism, and is not evidence for any clause.

| Clause | How this makes it witnessable |
|---|---|
| `a-change-is-offered-only-where-the-reader-holds-authority-to-make-it` | Gives the surface something to gate on before offering — today there is nothing |
| `no-authority-answer-reveals-what-a-reader-cannot-read` | Becomes falsifiable: an unreadable container's authority answer can be probed and must be indistinguishable from nonexistence |

Witnesses for both are authored **inside the build**, not here.
