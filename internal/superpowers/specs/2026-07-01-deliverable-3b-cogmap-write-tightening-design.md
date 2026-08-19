# Deliverable 3b — Cogmap-Write Tightening (Q-A): the first behavior-changing step of the access-capability arc

**Date:** 2026-07-01
**Status:** Draft → for review
**Goal:** `generalized-access-capability-model` (this arc).
**Task:** `019f1d45-ab36-7dc2-b8f3-03ef3a758fe1` (plan/medium — design pass, then build).
**Parent design:** [2026-06-30 generalized-access-capability-model](2026-06-30-generalized-access-capability-model-design.md)
(§3.5 the `cogmap_authorable_by_profile` rewrite, §3.6 open-item E the granting semantics, §4 step 2 the write half).
**Prior deliverables shipped (PR #221):** D2 seam + table (`20260630000001_access_grants_seam.sql`), D3a explicit
context/cogmap **read** grants (`20260630000002_access_grants_read_wiring.sql`).

---

## §1 — Problem

`cogmap_authorable_by_profile` today is the flat read stub — authorship = flat team-cogmap membership (via
`cogmap_readable_by_profile`, `20260630000002:29-38`). Q-A (locked in the parent design, §3.2) splits read from write:
cogmap **reads** stay broad (membership baseline + explicit grant); cogmap **writes** become **narrow + accountable**
— an explicit `can_write` grant only, no membership inheritance. This is the first predicate whose behavior *changes*
(it strips implicit write from current members), so it cannot land as a pure widening; it needs the co-committed
backfill and the surface to mint grants.

### The grounding that shapes the build (verified 2026-07-01)

Three as-built facts reframe the parent design's "seed the creator at cogmap_create time" into concrete mechanics:

1. **Cogmap genesis is system-admin-only and fires as the _system actor_.** `DbBackend::create_cognitive_map`
   (`crates/temper-services/src/backend/db_backend.rs:1352`) fires genesis under `readback::system_actor` (owner =
   `handle='system'` profile, emitter = system entity), **not** the caller. The surfaces gate on `is_system_admin`
   (MCP `crates/temper-mcp/src/tools/cognitive_maps.rs:156-165`; HTTP the same). But the invoking human's profile
   *is* known on the surface and inside the command (`self.profile_id`).
2. **Genesis does NOT bind the map to a team.** The only `INSERT INTO kb_team_cogmaps` at runtime is the separate,
   also-admin-only `cogmap_service::bind_team` (`crates/temper-services/src/services/cogmap_service.rs:26-55`). A
   freshly-created map is **unbound** — its creator has no membership-derived authoring until a later `cogmap_bind`.
3. **`temper-system` is a universal auto-join team.** `20260629000002_auto_join_team_generalization.sql:32` sets
   `kb_teams.auto_join_role='watcher' WHERE slug='temper-system'`, and in `open` mode (default) every profile
   auto-joins every auto-join team. The **L0 kernel** (`system-default`) is joined to `temper-system`
   (`20260625000001_l0_kernel_cogmap.sql:47-50`). So under today's flat stub, `cogmap_authorable_by_profile(anyone,
   L0) = true` — **the entire userbase is a current "author" of the kernel map.** A naive "snapshot current authors"
   backfill would grant every user write to the operator-governed kernel. It must not.

---

## §2 — The layering (no new capability function)

The generalization is **already shipped** as the general seam — we do not add a cogmap-specific
capability-parametrized predicate. After D2:

- **`can(principal_table, principal_id, capability, subject_table, subject_id)`** (`20260630000001:102`) — the
  fully-general, capability-parametrized authority seam; `profile_explicit_grant(p, capability, subject_table,
  subject_id)` (`:50`) is its explicit-grant half. Both already take `capability` as text (`read|write|delete|grant`).
- **The per-subject named predicates** are the **derived floors** — the subject-specific
  membership/ownership/share logic explicit grants don't capture.

A collapsed `cogmap_capable_of_by_profile(p, c, cap)` was considered and **rejected**: the floors differ per
capability and only **read** has one (flat membership + the 3a explicit-read-grant). **write/delete/grant are
floorless** — each is exactly `profile_explicit_grant(p, cap, 'kb_cogmaps', c)`, nothing cogmap-specific to encode.
So a collapsed function is `CASE cap WHEN 'read' THEN <floor OR grant> ELSE profile_explicit_grant(…) END`, whose
`ELSE` arm *is* the general function — a redundant third layer that would also force by-name churn on the read
function's delegators (`anchor_readable_by_profile`, `endpoint_readable_by_profile`, `cogmap_scope_ids` all call
`cogmap_readable_by_profile`) for zero behavior gain. Consequence for 3b: **no proliferation to prevent** — keep the
one-line named write seam, and route all floorless authority (grant/revoke/delete) through `can()` directly.

---

## §3 — Design

### §3.A — The Q-A predicate flip (core)

One `CREATE OR REPLACE` (parent §3.5 verbatim):

```sql
-- Q-A: cogmap authorship = explicit write grant only (no membership-implies-write). Cogmaps have no owner
-- column, so there is no ownership floor; authority is wholly explicit. Reads stay membership-broad
-- (cogmap_readable_by_profile, unchanged); writes are narrow + accountable.
CREATE OR REPLACE FUNCTION cogmap_authorable_by_profile(p_profile uuid, p_cogmap uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT profile_explicit_grant(p_profile, 'write', 'kb_cogmaps', p_cogmap);
$$;
```

`derived_access_profile`'s `kb_cogmaps`/`write` arm (`20260630000001:85-86`) already delegates to this by name, so
`can('kb_profiles', p, 'write', 'kb_cogmaps', c)` follows automatically. No edit there. Membership no longer confers
authoring; `cogmap_readable_by_profile` is untouched (read stays broad).

### §3.B — Creator seeding (write layer, not the SQL mold)

In `DbBackend::create_cognitive_map`, on the **create path only** (after genesis fires, before commit, inside the
existing serializable txn), insert the creator's bootstrap grant:

```
(subject_table, subject_id)   = ('kb_cogmaps', new_cogmap_id)
(principal_table, principal_id) = ('kb_profiles', self.profile_id)   -- the INVOKING admin (caller), not the system actor
can_read = can_write = can_grant = true,  can_delete = false
granted_by_profile_id = self.profile_id                              -- self-grant, the bootstrap event
ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING
```

- **Why the command, not `cogmap_genesis` SQL:** genesis is the reusable event mold (also called by the scenario
  loader / L0 birth migration) and fires as the system actor; threading a human creator through it would pollute the
  mold and mis-seed scenario/migration-born maps. The command layer knows the real caller (`self.profile_id`) and is
  the write layer surfaces already dispatch through. Scenario/migration maps (L0) go through the substrate loader
  (`fire_with(CogmapGenesis…)`), **not** this command, so they get **no** creator seed — correct; L0 stays
  operator-governed.
- **Why `can_grant` too:** cogmaps have no ownership floor (§3.6-E.2), so without a seeded `can_grant` the creator
  could author but never add a co-author. Seeding `can_grant` makes delegated administration (§3.C) work from the map's
  birth.
- **Idempotency:** the create path is already guarded by the re-genesis no-op pre-check (`db_backend.rs:1377`), and
  `ON CONFLICT DO NOTHING` is the belt-and-suspenders backstop, so a retried create never double-seeds.
- **Unbound-map correctness:** the creator gets write independent of any `kb_team_cogmaps` binding — they can author
  immediately after creating, before/without binding to a team.

### §3.C — Grant / revoke primitive + full-parity surface

New `access_service` functions — the **only** writers of `kb_access_grants` on the surface path (creator-seed §3.B and
backfill §3.D are the other two writers, both system-internal):

```
grant_capability(pool, caller, GrantCapabilityRequest)   -> upsert one kb_access_grants row
revoke_capability(pool, caller, RevokeCapabilityRequest) -> delete one kb_access_grants row
```

- **Subject-polymorphic at the service layer** (the table is general); the **surface verbs are scoped to cogmap
  subjects for 3b** (`cogmap grant/revoke`). The subject-polymorphic service fn means a general `access grant/revoke`
  surface for resources + contexts is a cheap follow-on (D4/D5), with no service rewrite.
- **Request shape (typed struct, not inline JSON):** `GrantCapabilityRequest { subject_table, subject_id,
  principal_table, principal_id, can_read, can_write, can_delete, can_grant }`. The DB coherence CHECK
  (`write|delete|grant ⇒ read`) is the integrity backstop; the surface may also normalize (e.g. `--write` implies
  `--read`) before calling.
- **Auth before write:** gate on `is_system_admin(caller) OR can('kb_profiles', caller, 'grant', subject_table,
  subject_id)`. Deny → `Forbidden` (403). Follows the **`bind_team` precedent** (`cogmap_service.rs:33`) — grants are
  **admin events** (parent §3.7), firewalled from cognition, so they call `access_service` **directly from surfaces**,
  NOT through the cognitive `DbBackend`/operations trait (which is for cogmap/resource substrate writes). No `sqlx`
  inlined on a surface; the SQL lives in `access_service`.
- **Grant-administration ≠ authoring bypass (see §3.E).**

**Surfaces (full parity):**

| Surface | Grant | Revoke |
|---|---|---|
| CLI | `temper cogmap grant <cogmap-ref> --to-profile <ref> \| --to-team <uuid> [--read] [--write] [--grant]` | `temper cogmap revoke <cogmap-ref> --from-profile <ref> \| --from-team <uuid>` |
| MCP | `cogmap_grant` tool | `cogmap_revoke` tool |
| HTTP | `POST /api/cognitive-maps/{id}/grants` | `DELETE /api/cognitive-maps/{id}/grants` (principal in body) |

- **Principal resolution:** explicit `--to-profile`/`--to-team` (and MCP/HTTP typed fields) rather than one ambiguous
  ref — a profile principal takes a profile ref/handle, a team principal takes a team UUID (mirroring `cogmap_bind`'s
  raw-UUID team wire shape, `cognitive_maps.rs:211`). Revoke deletes the `(subject, principal)` row; absent row is a
  no-op success (idempotent, mirrors `bind_team`).
- **Cogmap ref** parses trailing-UUID-only (`temper_workflow::operations::parse_ref`), as the create/ingest paths do.

### §3.D — Backfill migration (per-member snapshot, exclude auto-join)

Co-committed with §3.A (see §3.F). Snapshots today's *deliberate* authors as per-profile grants:

```sql
-- One-time snapshot: current flat authors of each cogmap gain an explicit can_write grant, so #221's
-- multi-author authoring survives the Q-A tightening. PER-PROFILE (a true snapshot — no ongoing
-- membership-inheritance, which Q-A forbids). Auto-join teams (temper-system → the L0 kernel) are
-- EXCLUDED: their membership is the universal "everyone" pool, so snapshotting them would grant the whole
-- userbase write to the operator-governed kernel. granted_by = the system profile (an accountable one-time
-- admin event).
INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id,
                              can_read, can_write, granted_by_profile_id)
SELECT DISTINCT 'kb_cogmaps', tc.cogmap_id, 'kb_profiles', tm.profile_id, true, true,
       (SELECT id FROM kb_profiles WHERE handle = 'system')
FROM kb_team_cogmaps tc
JOIN kb_teams t         ON t.id = tc.team_id
JOIN kb_team_members tm ON tm.team_id = tc.team_id
WHERE t.auto_join_role IS NULL
ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING;
```

- **Per-binding-row filter:** `WHERE t.auto_join_role IS NULL` filters *the binding's team*, so a map joined to
  **both** a real team and `temper-system` snapshots only the real-team members (the deliberate authors); the
  auto-join-only members are excluded. A map joined **only** to auto-join teams (L0) contributes zero rows.
- **Snapshot semantics:** profile-principal grants ⇒ a member who *later* joins a snapshotted team does **not**
  inherit write (Q-A honored). Only profiles that were members at migration time are granted.
- `granted_by` = the `handle='system'` profile (the same actor genesis fires under; guaranteed to exist since the L0
  birth migration).

### §3.E — Grant-administration is a distinct axis from authoring

- **Authoring** (`cogmap_authorable_by_profile`, §3.A) stays **wholly explicit — no `is_system_admin` OR-arm** (Q3
  decision). An admin authors a map only via the creator-seed grant (§3.B) or an explicit grant.
- **Grant administration** (the §3.C gate) **does** admit `is_system_admin`. This is operationally necessary and a
  *different verb*: it covers **pre-existing maps** (created before creator-seeding existed, so they have no seeded
  `can_grant` holder) and repair. An admin can bootstrap the first grant on any map; from there the holder's `can_grant`
  carries delegated administration. Distinguishing the two axes keeps Q-A pure (no implicit *write*) while keeping the
  grant surface operable on day one.

### §3.F — Coupling & sequencing

- **§3.A (flip) + §3.D (backfill) land in ONE forward migration**, ordered **backfill-first, then flip**, committing
  atomically. There is never a committed state where a real author lacks their grant. "Behavior-changing but not
  big-bang": a single forward `CREATE OR REPLACE` + a bounded snapshot INSERT, green under `test-artifacts` + e2e —
  consistent with the additive-only-on-`main` invariant (schema-additive; the tightening is guarded by the
  co-committed backfill).
- **§3.B (creator seed)** is code (DbBackend) — ships with the same PR.
- **§3.C (grant surface)** is service + CLI + MCP + HTTP code — same PR.
- The parent design's later steps (D4 `kb_resource_access` → `kb_access_grants`; the §3.4 three-function cogmap-read
  UP-flip) are **out of scope** here.

---

## §4 — Files touched

| Area | File | Change |
|---|---|---|
| Migration | `migrations/20260701000001_cogmap_write_tightening.sql` (new) | §3.A flip + §3.D backfill, atomically |
| Creator seed | `crates/temper-services/src/backend/db_backend.rs` (`create_cognitive_map`) | §3.B grant insert in the create txn |
| Grant service | `crates/temper-services/src/services/access_service.rs` | `grant_capability` / `revoke_capability` + request structs + the `can`/`is_system_admin` gate |
| CLI | `crates/temper-cli/src/commands/cogmap.rs` + action | `cogmap grant` / `cogmap revoke` subcommands |
| MCP | `crates/temper-mcp/src/tools/cognitive_maps.rs` | `cogmap_grant` / `cogmap_revoke` tools + registration |
| HTTP | `crates/temper-api/src/handlers/…` (cognitive-maps) + routes | `POST/DELETE /api/cognitive-maps/{id}/grants` |
| Wire types | `temper-core` (ts-rs) | grant request/response types shared across surfaces |
| Tests | `crates/temper-api/tests/cogmap_home_test.rs:181,405` | flip membership⇒authorable assertions |
| e2e | `tests/e2e/tests/…` (new) | §5 scenarios |

---

## §5 — Tests

**Flip the membership⇒authorable assertions** (`cogmap_home_test.rs:181`, `:405`): membership alone now **denies**
authoring; an explicit `can_write` grant authorizes.

**New e2e (the access-semantics tier — #219's lesson: e2e catches hazards isolated-DB tests miss):**

1. **Non-member gains write ONLY via an explicit grant.** A profile with no membership and no grant is denied cogmap
   authoring (403); after a `grant_capability(can_write)` through the **production grant caller** (CLI/MCP/HTTP), the
   same profile authors successfully. Revoking removes it again.
2. **Creator authors their freshly-created, unbound map.** Admin creates a map (no `cogmap_bind`); the creator-seed
   (§3.B) lets them author a resource homed in it immediately.
3. **A backfilled member still authors.** A member of a non-auto-join team joined to a map before the migration
   retains authoring after it (their snapshot grant).
4. **An arbitrary user cannot author L0.** A non-admin, non-granted profile is denied authoring the `system-default`
   kernel map — proving the auto-join exclusion held (they'd have passed under the old flat stub).
5. **Grant-admin axis:** a holder of `can_grant` (the creator) can grant a co-author write; a profile with only
   `can_write` (no `can_grant`) **cannot** grant further (the coherence between the surface gate and the seeded caps).

**Gates:** green under `test-artifacts` **and** e2e; `cargo make check` clean; per-crate sqlx prepare rituals after
the SQL/service changes — `cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services`,
`cargo make prepare-api`, `cargo make prepare-e2e`.

---

## §6 — Risks & open items

- **Pre-existing maps have no `can_grant` holder.** Resolved by §3.E (admin grant-administration bypass). Not a code
  gap; a deliberate operability seam.
- **Backfill sizing.** Bounded by `kb_team_cogmaps ⋈ (non-auto-join) kb_team_members`; in practice small on a young
  system (mostly L0 + a few real maps). No cap needed; if it were large, the snapshot is still a one-time INSERT.
- **CONTEXT-homed-edge read-grant arm** noted as a follow-on in D3a (`20260630000002:18-20`) is **unrelated** to this
  write-tightening — left as-is.
- **`is_system_admin` in the grant gate** must be read as *grant-administration*, not authoring — §3.E states this
  explicitly so a future reader does not mistake it for a Q-A regression.
