# Generalized Access-Capability Model (rwx grants) + Team-Hierarchy Visibility-Direction Reconcile

**Date:** 2026-06-30
**Status:** **Reviewed** — ready for `approved`/`pending-plan`. §1 (prior-art survey) and §2 (visibility-direction
truth-table) are grounded and done; §3 (unified model) carries three **locked decisions** (Q-A/Q-B/Q-C) and now the
**concrete DDL + predicate rewrites** (`kb_access_grants`, the three-function lockstep flip, the
`cogmap_authorable_by_profile` rewrite, the `can()` seam) with the five open items resolved (§3.6); §4 (sequencing)
is concrete enough to start deliverable 2 without re-design. Every as-built claim cites a verbatim `file:line` and
is confirmed against the **live dev DB** (`pg_get_functiondef`, GD-2); §5 is the grounding appendix.
**Goal:** `generalized-access-capability-model` (this arc). Likely sits under `substrate-kernel-to-cognitive-map`
(the [2026-06-02 access-capability design](2026-06-02-access-capability-model-design.md) was that goal's Arc 1).
**Task:** `019f1a27-e99d-75a2-bdb7-43f1b4e86bdf` (plan/large).
**Surfaced by:** PR #219 (Surface B Half 2). Subsumes the deferred `--cogmap` producer **write** predicate left
by parent task `019f15b1`.

> **Grounding note.** §1–§2 are written against the **as-built migrations** (`migrations/20260624000002_canonical_functions.sql`
> and the `…0627*` / `…0629*` additive migrations), cross-checked by an 8-agent prior-art survey over the design
> specs + migrations + project memories, and independently re-read at the SQL level. Every direction claim cites a
> verbatim `file:line`. Where this doc says "as-built," it has been checked against the migration body, not inferred
> from a spec.

---

## Context

Access in Temper accretes three ways: **explicit grants** (`kb_resource_access`, principal-polymorphic, rwx
booleans), **home-derived** (you own / originated / are-in the home), and **membership/share-derived** (the team
graph via `kb_team_contexts` / `kb_team_cogmaps`). Contexts and cogmaps have **no explicit-grant rows** — their
access is purely membership-derived, so *"grant profile X read on cogmap Y without team membership"* is
**inexpressible today** (the `kb_resource_access.anchor_table` CHECK is `('kb_teams','kb_profiles')` and its subject
is resources-only). PR #219's A0 bolted cogmap-membership read into `resources_visible_to` as one more UNION branch
and left the producer-side `--cogmap` **write** predicate deferred. This doc generalizes into one rwx capability
model — **after** reconciling a visibility-direction question that turned out to be the crux.

---

## §1 — Prior art (surveyed, grounded)

| Source | What it establishes | Status |
|--------|---------------------|--------|
| [2026-06-02 access-capability](2026-06-02-access-capability-model-design.md) | The decomposed-capability model: `read/write/delete/grant` booleans; **CS-1** principal sum type (`Profile`/`Cogmap`); **A2-2** map-home-confers; **A2-3** "DAG down-only"; **A2-4** edge-gate AND; **A4-1** priming-vs-material. `kb_resource_access` anchor CHECK `('kb_teams','kb_profiles')`; cogmaps/contexts **deliberately excluded** as grantees. | The major prior design |
| [2026-06-01 data-model-reconciliation](2026-06-01-data-model-reconciliation-design.md) | The homes/access two-table split; the polymorphic `(anchor_table, anchor_id)` + CHECK + no-subject-FK idiom; `access_level` flagged PROVISIONAL (→ replaced by rwx booleans). | Stable backbone |
| [2026-06-16 WS2 access-scoping](2026-06-16-ws2-access-scoping-over-temper-next-design.md) | The read-scoping flip: `JOIN resources_visible_to($1)`, deny-split (reads→404, writes→403), `can_modify_resource` write gate. | Shipped (#140–#142) |
| [2026-03-27 R4 crate-arch auth](2026-03-27-r4-crate-architecture-auth-access-control-design.md) | Authz as data-layer composable `STABLE` SQL functions; the DB is the authority, Rust is the caller. **Pre-cogmap, flat teams** (no DAG). | Superseded baseline |
| [2026-06-11 access-scaffold scenario-proof](2026-06-11-access-scaffold-scenario-proof-design.md) | The leak-safety scenario proof (S1 consumer reach, S2 producer intersection + profile-grant leak-safety). | Validated (#129) |

**Grounding gap flagged:** the task's §1 named a memory `project_authorial_rbac_undefined_contexts_cogmaps`, and
`migrations/20260629000005_cogmap_home_authz_and_scope.sql:4` **cites it in a code comment** — but that memory file
**does not exist** on disk (confirmed). The code references a memory that was never written or was deleted. The
nearest live referent is the deferred `cogmap_authorable_by_profile` stub it annotates.

---

## §2 — Visibility-direction truth-table (the crux)

### The headline: "DAG down-only" is **not** opposite to "strictly upward" — it's the same relation, two vocabularies

The task suspected the 2026-06-02 **A2-3 "DAG down-only"** decision pointed the opposite way from the owner's
intended **strictly-upward** model. It does not. They are the same edge named from opposite ends:

- **Grant-vantage** (`2026-06-02:281`): *"`kb_teams_parents` inherits **down**: a descendant team sees its ancestor
  team's grants; an ancestor gains **no** visibility into a descendant's private material."*
- **Member-vantage** (owner, 2026-06-30): a member sees own team + all **ancestors**, never descendants/siblings.
- **As-built** (`canonical_functions.sql:29`): `team_ancestors` is a child→parent recursive walk (`WITH RECURSIVE
  up … JOIN kb_teams_parents tp ON tp.child_id = up.team_id` selecting `tp.parent_id`).

All three yield the identical worked example: Ecommerce-Frontend member sees Engineering + EPD (ancestors), **not**
B2B-Backend (sibling); a parent-only member sees **no** child docs. **"Down-only" was not a mis-decision, not stale,
not a different concern.** It is the upward model, and `team_ancestors` implements exactly it.

### The two principal axes are **deliberately divergent** (the real shape)

CS-1 (`canonical_functions.sql:14`): a substrate read carries **one** principal. The two want **opposite** set-algebra
— this is a feature, not the inconsistency to fix:

- **`Profile` (Case 1 — human / human+agent, team-authed)** → **UNION, transitive-up.** Broadest reachable view.
  `reachable_teams = profile_effective_teams ⋈ team_ancestors`; union over home / membership(up) / grant / share.
- **`Cogmap` (Case 2 — steward/persona launched *into* a map, no profile identity)** → **INTERSECTION,
  least-privilege.** Narrowest common ground. `resources_accessible_to_cogmap = ⋂ vis_team(T)` over `teams(M)`
  (`:222`); `cogmaps_share_a_team` (`:323`) as the priming/edge-creation gate. The intersection is what stops a
  higher-specificity team's private material poisoning a broader frame.

So the task's §3 goal "resolve to **one** uniform direction across all subject types" is subtly wrong: it is uniform
**per axis**, and the two axes diverge on purpose.

### The truth-table

`…02` = `20260624000002_canonical_functions.sql`; `…06` = `20260629000006`; `…07` = `20260629000007`;
`…05` = `20260629000005`.

| Access path | As-built (`file:line`) | Direction | Model wants | Verdict |
|---|---|---|---|---|
| **Profile** — resource team grant (`resources_visible_to`) | `…02:125`,`…06:45` | UP | UP (union) | ✓ |
| **Profile** — resource write (`can_modify_resource`) | `…02:164` | UP | UP | ✓ |
| **Profile** — context **share** (`kb_team_contexts`) | `…06:50` | UP | UP | ✓ |
| **Profile** — team-**owned** context | `…06:60`,`20260627000001:29` | **FLAT** (direct membership) | FLAT | ✓ **correct** — ownership ≠ grant (§3.6 open-item C); cross-team reach is via an explicit *share* (`kb_team_contexts`, which *does* go up), not via ownership |
| **Profile** — profile-owned context | `20260627000003:31` | NONE (self) | n/a | ✓ |
| **Profile** — read cogmap **shape** (`cogmap_readable_by_profile`) | `…02:259` | **FLAT** | UP | ✗ **mismatch** |
| **Profile** — read cogmap-homed **resource** (A0 branch) | `…06:67` | **FLAT** | UP | ✗ **mismatch** (paired) |
| **Profile** — cogmap search admission (`cogmap_visible_maps` → `wayfind_scope_ids`) | `…07:10`,`…07:23` | **FLAT** | UP | ✗ (follows cogmap read) |
| **Profile** — cogmap **write** (`cogmap_authorable_by_profile`) | `…05:5` | **FLAT stub → read** | explicit-grant (Q-A) | ✗ **deferred** |
| **Cogmap** — producer reach (`resources_accessible_to_cogmap` = ⋂ `vis_team`) | `…02:200`,`…02:222` | UP-per-team **+ ⋂** | least-privilege ⋂ | ✓ (CS-1, settled) |
| Edge traversal (`edges_visible_to`) | `…02:305` | inherits each gate (AND) | follows gates | ✓ (A2-4) |
| Edge homed in **profile-owned** context | `anchor_readable_by_profile` `20260627000003:30` | owner-arm present | owner should see | ✓ **closed** (`20260627000003`, post-dates the §1 memory; §3.6 open-item D) |
| Delegation (`cogmaps_share_a_team`) | `…02:323` | FLAT (shared team) | priming, weaker-by-design | ✓ (A4-1) |
| System gate (`has_system_access`/`is_system_admin`) | `…02:1388`,`…02:1409` | FLAT (root is a real membership) | — | ✓ |

> **Two rows re-grounded 2026-06-30 (were stale in the seed).** The seed cited the *original* `…02` definitions of
> two functions that later migrations `CREATE OR REPLACE`d, so they read as open problems they no longer are:
> (a) **team-owned context** is *correct* flat, not a `UP?` to settle — ownership confers to the owning team's direct
> members only; cross-team reach is the explicit *share* (`kb_team_contexts`), which already inherits up (open-item C).
> (b) the **profile-owned-edge-home gap** was *closed* in `20260627000003` (the `anchor_readable_by_profile` `kb_contexts`
> arm gained the profile-owned clause, `:30-37`), post-dating the §1 memory that still describes it as open (open-item D).
> The genuine, still-open mismatch is the **`Profile` ↔ cogmap** cluster below.

**The mismatches cluster on one axis: `Profile` ↔ cogmap.** Shape-read, homed-resource-read, and search-admission
are all flat-membership, and they're flat *together on purpose* — the A0 branch (`…06:67`) deliberately mirrors
`cogmap_readable_by_profile`'s flatness *"so map-read and resource-read agree by construction."* This flatness
**undercuts Case 1**: a child-team member gets the ancestor team's *resource* grants but **not** its *cogmaps* into
the wayfind admission set, silently truncating the "broader view by unioning the maps I reach."

**The genuine fix is intra-Case-1, not cross-axis:** make `Profile` cogmap **reads** UP+union to match resource
grants. Flipping requires **three functions in lockstep** to preserve the "map-read = resource-read" invariant:
`cogmap_readable_by_profile` (`…02:259`), the A0 resource clause (`…06:67`), and `cogmap_visible_maps` (`…07:10`).
**Case 2 (the agent intersection) is untouched.**

---

## §3 — The unified model (concrete DDL + predicate rewrites)

### §3.1 — Settled frame

1. **Two principals, divergent algebra** (CS-1): `Profile` → union/transitive-up; `Cogmap` → intersection/least-privilege.
2. **The Case-1 cogmap-read bug**: `Profile` reads of cogmaps (shape + homed-resource + search-admission) become
   **UP + union**; the three functions flip in lockstep.
3. **Explicit grants widen, never replace**: on the `Profile` axis, `home-derived OR membership-derived(up) OR
   explicit-grant` compose by union — exactly as resources already do.
4. **Write-safety grounds in the edge-home, not the endpoints** (A2-4 + producer-bound): edge creation has two
   independent gates — *can I author in this home?* × *can I read the endpoint I point at?* — and the resulting edge
   can't leak, because its visibility is governed by its home (`edges_visible_to`, `…02:305`). A steward (Case 2) can
   freely wire edges **out** of its frame to any resource it reaches (via the intersection, or a resource that
   entered through an inbound event and is context-visible-through-team-visible) without ever creating an
   access-escalation surface.

### §3.2 — Locked decisions

- **Q-A — read-up does NOT drag write-up.** `Profile` cogmap **reads** go union-up; **authorship** does not inherit
  up. Authorship moves to the explicit-grant polymorphism (`kb_access_grants.can_write`), not membership-inheritance.
  Reads are **broad** (membership baseline); writes are **narrow + accountable** (explicit grant). This is the
  read/write split that subsumes the deferred `cogmap_authorable_by_profile` (`…05:5`). *(The Case-2 steward write
  path stays map-home-confers-write per A2-2 — unchanged.)*
- **Q-B — leak-safety invariant, generalized.** Explicit grants apply to the **`Profile`/consumer axis only**;
  they **never** enter the `Cogmap` producer intersection. Humans + explicit grants = **accountability**; the
  agent-producer keeps its least-privilege guarantee so it never "in good faith reaches for info it should not
  link-and-share." This is the generalization of the existing rule *"profile-anchored grants never enter `vis(T)`"*
  (`…02:196`, `2026-06-01:234`). *(Holding "persons" = humans-with-accountability; deeper personhood/agency/selfhood
  questions are explicitly out of scope for this arc.)*
- **Q-C — dual-polymorphic `kb_access_grants`.** One table: `(subject_table, subject_id) × (principal_table,
  principal_id) × {can_read, can_write, can_delete, can_grant}` with the `write|delete|grant ⇒ read` CHECK (carried
  from 2026-06-02 OQ-1). Subjects `{kb_resources, kb_contexts, kb_cogmaps}`; principals `{kb_teams, kb_profiles}`.
  Matches the house `(anchor_table, anchor_id)` + CHECK + no-subject-FK idiom. **Eventually subsumes and replaces
  `kb_resource_access`.** Split-per-subject rejected (3× rwx duplication doesn't pay for the subject-FK win).

### §3.3 — `kb_access_grants` DDL (concrete) — AMEND of the `kb_resource_access` idiom

> **CONFORM** to the house polymorphic-anchor idiom — `kb_resource_homes` (`20260624000001_canonical_schema.sql:276`)
> and `kb_resource_access` (`:296`): `VARCHAR(64)` discriminator + `CHECK (… IN (…))` + **no real FK** on the
> polymorphic columns ("no real FK (can't FK two tables); integrity is the CHECK + the seeding path," `:274-275`),
> the four rwx booleans, and the coherence CHECK. **AMEND** (authorized by Q-C): the single `resource_id` →
> `kb_resources` FK becomes a *dual*-polymorphic `(subject_table, subject_id)`, so the subject loses its real FK too.

```sql
-- Generalized rwx access grants (access-capability arc; Q-C). Dual-polymorphic:
-- (subject_table, subject_id) × (principal_table, principal_id) × {r,w,d,grant}.
-- Subsumes kb_resource_access (subject='kb_resources') and extends the grantable
-- subject set to contexts + cogmaps (inexpressible in kb_resource_access, whose
-- subject was resources-only). No real FK on either polymorphic pair — integrity
-- is the CHECK + the granting path (mirrors kb_resource_homes :274-275).
CREATE TABLE kb_access_grants (
    id                    UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    subject_table         VARCHAR(64) NOT NULL CHECK (subject_table   IN ('kb_resources','kb_contexts','kb_cogmaps')),
    subject_id            UUID NOT NULL,
    principal_table       VARCHAR(64) NOT NULL CHECK (principal_table IN ('kb_teams','kb_profiles')),
    principal_id          UUID NOT NULL,
    can_read              BOOLEAN NOT NULL DEFAULT false,
    can_write             BOOLEAN NOT NULL DEFAULT false,
    can_delete            BOOLEAN NOT NULL DEFAULT false,
    can_grant             BOOLEAN NOT NULL DEFAULT false,
    granted_by_profile_id UUID NOT NULL REFERENCES kb_profiles(id),
    granted_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (subject_table, subject_id, principal_table, principal_id),
    -- §2 coherence, carried verbatim from kb_resource_access:309 (2026-06-02 OQ-1):
    -- you cannot mutate or re-share what you cannot read.
    CHECK ((can_write OR can_delete OR can_grant) <= can_read)
);
CREATE INDEX idx_kb_access_grants_subject   ON kb_access_grants(subject_table, subject_id);
CREATE INDEX idx_kb_access_grants_principal ON kb_access_grants(principal_table, principal_id);
```

The `granted_by_profile_id` + `granted_at` columns carry over verbatim (`:305-306`): every grant row records its
admin-event provenance (see §3.7). `kb_resource_access`'s `resource_id REFERENCES kb_resources … ON DELETE CASCADE`
is **lost** by polymorphizing the subject; the row-cleanup it gave (drop grants when the resource is deleted) moves
to the granting/deletion path, exactly as `kb_resource_homes`'s anchor already has no cascade (`:280`).

### §3.4 — The three-function lockstep flip (concrete `CREATE OR REPLACE`)

> **AMEND** of the three flat cogmap-read functions (the §2 mismatch cluster), authorized by §3 item 2. The flip
> replaces the **flat** `profile_effective_teams(p)` join with the **up** expansion
> `profile_effective_teams(p) ⋈ team_ancestors(·)` — *the identical `reachable_teams` CTE already used by
> `resources_visible_to`'s grant branches* (`…06:32-36`). After the flip all three key on the same up-expanded team
> set over `kb_team_cogmaps`, so **map-read = resource-read holds by construction** — now at the up-expanded level
> instead of the flat level. Leak direction: reads get *broader* (a child-team member now reads an ancestor-team-joined
> map), which is the member-vantage upward model (§2), **not** a leak; Case 2 (`resources_accessible_to_cogmap`,
> `vis_team`) calls none of these three and is untouched (Q-B preserved).

```sql
-- (1) cogmap_readable_by_profile — was FLAT (…02:259), now UP+union.
CREATE OR REPLACE FUNCTION cogmap_readable_by_profile(p_profile uuid, p_cogmap uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
        FROM kb_team_cogmaps tc
        JOIN (SELECT DISTINCT a.team_id
              FROM profile_effective_teams(p_profile) e
              CROSS JOIN LATERAL team_ancestors(e.team_id) a) rt ON rt.team_id = tc.team_id
        WHERE tc.cogmap_id = p_cogmap
    );
$$;

-- (2) resources_visible_to — the cogmap-membership UNION branch (was FLAT …06:67-76).
--     Only that branch changes: JOIN the already-present reachable_teams CTE instead
--     of profile_effective_teams. (Full function re-emitted in the migration.)
    -- cogmap membership: resources homed in a map joined to a REACHABLE (self-or-ancestor)
    -- team — UP+union to match the team-grant reach two branches above.
    SELECT h.resource_id
    FROM kb_team_cogmaps tc
    JOIN reachable_teams rt ON rt.team_id = tc.team_id          -- was: profile_effective_teams(p_profile) e
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = tc.cogmap_id;

-- (3) cogmap_visible_maps — was FLAT (…07:10), now UP+union.
CREATE OR REPLACE FUNCTION cogmap_visible_maps(p_principal uuid)
RETURNS SETOF uuid LANGUAGE sql STABLE AS $$
    SELECT DISTINCT tc.cogmap_id
    FROM kb_team_cogmaps tc
    JOIN (SELECT DISTINCT a.team_id
          FROM profile_effective_teams(p_principal) e
          CROSS JOIN LATERAL team_ancestors(e.team_id) a) rt ON rt.team_id = tc.team_id;
$$;
```

**Two more functions follow the flip for free** (no edit — they *delegate* to (1)): `anchor_readable_by_profile`'s
`kb_cogmaps` arm (`20260627000003:29`) and `endpoint_readable_by_profile`'s `kb_cogmaps` arm (`…02:296`) both call
`cogmap_readable_by_profile`, so edge-home and endpoint admission inherit the up-flip automatically. The lockstep is
genuinely **three** `CREATE OR REPLACE`s. The `cogmap_scope_ids` single-map path (`…05:12`) also follows for free
(it gates on `cogmap_readable_by_profile`). The stale "NOT ancestor-expanded … agree by construction" comments at
`…06:11-13`, `…07:8-9` must be re-emitted in the flip migration to read "**both** ancestor-expanded … agree by
construction."

### §3.5 — `cogmap_authorable_by_profile` rewrite (Q-A) + the `can()` seam

> **AMEND** of the stub `cogmap_authorable_by_profile` (`…05:5`, confirmed live = `SELECT cogmap_readable_by_profile(…)`),
> authorized by Q-A: authorship is **explicit grant**, not read-up. **EXTEND** for the new `explicit_grant` /
> `profile_explicit_grant` / `can()` functions (no prior referent; authorized by §3).

`profile_explicit_grant` is the subject-polymorphic generalization of `resources_visible_to`'s two grant UNION
branches (`…06:42-48`): a **direct profile-anchored** grant ∪ a **team-anchored** grant on a **reachable
(self-or-ancestor)** team. Q-A makes *write* require such an explicit grant (narrow + accountable), where *read*
takes the membership baseline (broad).

```sql
-- A profile's explicit-grant reach for ACTION on SUBJECT (subject-polymorphic
-- generalization of resources_visible_to …06:42-48): direct profile grant, OR a
-- team grant on a reachable (self-or-ancestor) team.
CREATE FUNCTION profile_explicit_grant(
    p_profile uuid, p_action text, p_subject_table text, p_subject_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    )
    SELECT EXISTS (
        SELECT 1 FROM kb_access_grants g
        WHERE g.subject_table = p_subject_table AND g.subject_id = p_subject_id
          AND CASE p_action WHEN 'read'  THEN g.can_read  WHEN 'write'  THEN g.can_write
                            WHEN 'delete' THEN g.can_delete WHEN 'grant' THEN g.can_grant
                            ELSE false END
          AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
             OR (g.principal_table = 'kb_teams'    AND g.principal_id IN (SELECT team_id FROM reachable_teams)) )
    );
$$;

-- Q-A: cogmap authorship = explicit write grant only (no membership-implies-write).
-- (cogmaps have no owner column — id,name,telos_resource_id,shape_materialized_event_id,
-- created — so there is no ownership floor; authority is wholly explicit, §3.5 item E.)
CREATE OR REPLACE FUNCTION cogmap_authorable_by_profile(p_profile uuid, p_cogmap uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT profile_explicit_grant(p_profile, 'write', 'kb_cogmaps', p_cogmap);
$$;
```

**The `can()` seam** is axis-dispatched on the principal sum type (CS-1), mirroring the existing
`resources_readable_by` dispatch (`…02:244`). The **`Profile`** arm is union-up (explicit grant ∪ the axis-correct
derived floor); the **`Cogmap`** arm is the intersection/least-privilege producer reach and **takes no explicit
grants** — the operational form of Q-B.

```sql
-- Unified capability seam. Subject-polymorphic {kb_resources,kb_contexts,kb_cogmaps};
-- action {read,write,delete,grant}. Axis-dispatched (CS-1, mirrors …02:244).
CREATE FUNCTION can(
    p_principal_table text, p_principal_id uuid, p_action text,
    p_subject_table text, p_subject_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE p_principal_table
        -- Profile (consumer): union-up. explicit grant OR the derived floor (the
        -- existing per-subject reads, migrated in §4 step 4): resource→resources_visible_to/
        -- can_modify_resource; cogmap→cogmap_readable_by_profile(read)/cogmap_authorable_by_profile(write);
        -- context→context_visible_to(read). Q-B: profile-anchored grants never enter a vis(T).
        WHEN 'kb_profiles' THEN
            profile_explicit_grant(p_principal_id, p_action, p_subject_table, p_subject_id)
            OR derived_access_profile(p_principal_id, p_action, p_subject_table, p_subject_id)
        -- Cogmap (producer): intersection/least-privilege, RESOURCE subjects only, READ only.
        -- NO explicit-grant arm (Q-B). Non-resource subjects / non-read actions ⇒ false:
        -- an agent neither administers grants nor authors maps/contexts qua principal.
        WHEN 'kb_cogmaps' THEN
            p_subject_table = 'kb_resources' AND p_action = 'read'
            AND p_subject_id IN (SELECT resource_id FROM resources_accessible_to_cogmap(p_principal_id))
        ELSE false
    END;
$$;
```

`derived_access_profile` is the per-subject derived floor — for deliverable 2 it can be a thin shim over the
*existing* read predicates (so `can()` lands alongside with no behavior change); §4 step 4 inlines them. The seam is
the single call site surfaces should migrate to (`can('kb_profiles', profile, 'write', 'kb_cogmaps', map)` replaces
the bespoke `cogmap_authorable_by_profile` call, etc.), but **the migration is mechanical and deferred to step 4** —
deliverable 2 only needs the table + `explicit_grant`/`profile_explicit_grant` + the `can()` skeleton present.

### §3.6 — The five open items, resolved

- **A — exact DDL + lockstep flip.** Resolved in §3.3–§3.4 (concrete `CREATE TABLE` + three `CREATE OR REPLACE`).
- **B — `cogmap_authorable_by_profile` rewrite.** Resolved in §3.5: `→ profile_explicit_grant(profile,'write',
  'kb_cogmaps',cogmap)` (Q-A). No ownership floor exists for cogmaps (no owner column), so authority is wholly explicit.
- **C — team-owned-context FLAT → UP?** **Decision: stays FLAT — it is *correct*, not a mismatch.** Rationale:
  the DAG inherits **grants and shares** down (`team_ancestors`, the grant-vantage of §2), *not* home-**ownership**.
  A `kb_contexts` row is **owned by exactly one team** (`owner_table`/`owner_id`, single ownership); a `kb_cogmap` is
  **joined to many teams** (`kb_team_cogmaps`, many-to-many — the producer iterates `teams(M)`). So a cogmap-join is
  *share-like* → inherits up (hence the §3.4 flip); a context-ownership is *home-like* → flat. A descendant team
  reads a parent-owned context's resources **iff the parent explicitly *shares* it** (`kb_team_contexts`, which is in
  `resources_visible_to`'s up `reachable_teams` branch `…06:51-55`) — not merely because the parent owns it. Keeping
  it flat also keeps resource-visibility paired with **addressability** (`context_visible_to` clause 2, `20260627000001:29`,
  is itself flat); flipping it would silently desync "a context you can address" from "the resources in it." ✓
- **D — profile-owned-edge-home gap fold-in.** **Decision: nothing to fold — already closed.** Migration
  `20260627000003` added the profile-owned clause to `anchor_readable_by_profile`'s `kb_contexts` arm (`:30-37`),
  so `edges_visible_to(owner)` now admits edges homed in the owner's own context. The §1 memory
  `project_ws2_access_scoping_and_edge_home_gap` describes the gap **as it existed in the pre-flip `temper_next`
  artifact**, which `…003` post-dates; it is historical, not an open action. The §2 truth-table row is corrected. ✓
- **E — `grant`-verb semantics on cogmap/context subjects.** **Decision:**
  1. **`can_grant` = delegated administration.** Holding `can_grant=true` on subject *S* authorizes minting/editing
     `kb_access_grants` rows whose `subject` is *S*, bounded by the coherence CHECK (`write|delete|grant ⇒ read`),
     so you can never grant what you cannot read.
  2. **Bootstrap authority (the floor, no explicit row):** resource → home `owner_profile_id`
     (`kb_resource_homes:282`); context → `kb_contexts.owner` (profile-owned: that profile; team-owned: the owning
     team's members — role-gating deferred). **Cogmap → no ownership floor** (no owner column), so cogmap-granting
     requires an **explicit `can_grant`** row, seeded to the **creating profile at `cogmap_create`** time (an admin
     event). This is the principled consequence of cogmaps being join-governed rather than owned.
  3. **Transfer-of-ownership is OUT OF SCOPE for this arc** — it mutates the ownership column (`owner_id` /
     `owner_profile_id`), a distinct admin event, not a grant. Flagged for a future ownership-transfer unit.

### §3.7 — Grants are admin events (firewalled from cognition)

Grant/revoke are **admin events** per the admin-event-sourcing firewall (compliance records, bounded at Postgres,
**not** participating in cogmaps/relationships; memory `project_admin_eventsourcing_and_operating_shape`). The
`granted_by_profile_id`/`granted_at` columns are the per-row provenance; the event log is the temporal record. No
grant ever becomes substrate an agent reasons over — which is exactly what lets `kb_profiles` be a safe grantee
(Q-B): a person-grant is an accountability fact, never a cognition input.

---

## §4 — Additive sequencing (concrete; never big-bang)

Per the additive-only-on-`main` invariant, each step lands as a **forward migration**, green under `test-artifacts`
**and e2e** — access-semantics changes need the e2e tier (#219 showed e2e catches hazards the isolated-DB tests
miss). Each step preserves the leak-safety invariants (Q-B) and the deny-split (reads→404, writes→403).

1. **Seam + table alongside (no behavior change).** `CREATE TABLE kb_access_grants` (§3.3) + `explicit_grant` /
   `profile_explicit_grant` + the `can()` skeleton with `derived_access_profile` a thin shim over today's read
   predicates. Nothing reads the new table yet. *Verify:* table CHECKs + `can()` returns identical results to the
   existing predicates on the prod-shape fixture.
2. **Land the NEW capability first.** Explicit **context/cogmap** grants (the inexpressible-today case) + the
   `cogmap_authorable_by_profile` → `profile_explicit_grant(…,'write','kb_cogmaps',…)` rewrite (§3.5 B). *Verify:*
   an e2e scenario where a non-member profile gains cogmap-write **only** via an explicit `can_write` grant.
3. **Fix the Case-1 cogmap-read direction (the §3.4 lockstep flip).** Three `CREATE OR REPLACE` to UP+union, with
   the "map-read = resource-read" invariant re-stated in the re-emitted comments. *Verify:* an e2e scenario where a
   **child-team** member reads an **ancestor-team-joined** map's shape + homed resources + wayfind admission — all
   three agreeing — while a non-member still sees nothing. (This is the visibility *expansion*; it needs its own
   scenario proof, per #219's lesson.)
4. **Migrate `kb_resource_access` → `kb_access_grants`.** Rewrite `resources_visible_to` / `can_modify_resource` /
   `vis_team` grant branches to read `kb_access_grants` (filtering `subject_table='kb_resources'`). **Leak-safety
   filter, load-bearing:** `vis_team`'s grant branch reads **only** `(subject_table='kb_resources',
   principal_table='kb_teams', can_read)` — profile-principal grants and the new context/cogmap subjects **never**
   enter the producer intersection (Q-B; the generalization of `vis_team`'s as-built profile-grant exclusion,
   `…02:196-199`). Inline `derived_access_profile` to call the unified store. *Verify:* the PR #129 access-scenarios
   (S1 consumer reach, S2 producer intersection + profile-grant leak-safety) stay green.
5. **Retire the old `kb_resource_access` branches** and, once no reader remains, the table. *Verify:* `cargo sqlx
   prepare` clean; no surface references `kb_resource_access`.

> **Grounding for deliverable 2:** steps 1–2 are buildable directly from §3.3 + §3.5 (the table, `explicit_grant`,
> `profile_explicit_grant`, `cogmap_authorable_by_profile` rewrite) with no further design. Step 3's three bodies are
> in §3.4 verbatim. Steps 4–5 are mechanical store-swaps preserving the §3.7/Q-B filter.

---

## §5 — Grounding appendix (as-built, GD-1/GD-2)

Every "as-built" claim in §2–§4 was confirmed **both** in the migration source **and** executably against the live
dev DB (`pg_get_functiondef`, after applying through `20260629000007`). The load-bearing confirmations:

| Claim | Source (`file:line`) | Executable confirmation |
|---|---|---|
| `cogmap_readable_by_profile` is FLAT (`profile_effective_teams` join, no ancestor walk) | `…02:259-267` | `pg_get_functiondef` → `JOIN profile_effective_teams(p_profile) e ON e.team_id = tc.team_id` |
| `resources_visible_to` cogmap branch is FLAT | `…06:67-76` | function body contains the `profile_effective_teams … kb_team_cogmaps` join, no `team_ancestors` |
| `cogmap_visible_maps` is FLAT | `…07:10-15` | `pg_get_functiondef` → `JOIN profile_effective_teams(p_principal) e ON e.team_id = tc.team_id` |
| `cogmap_authorable_by_profile` is the read stub | `…05:5-8` | `pg_get_functiondef` → `SELECT cogmap_readable_by_profile(p_profile, p_cogmap);` |
| `reachable_teams` = `profile_effective_teams ⋈ team_ancestors` (the up expansion the flip reuses) | `…06:32-36`, `…02:29-39` | `team_ancestors` is the child→parent recursive walk |
| `kb_resource_access` idiom (VARCHAR(64)+CHECK, rwx booleans, `(w∨d∨g)≤r` CHECK, no subject FK) | `20260624000001_canonical_schema.sql:296-310` | live cols + `CHECK ((can_write OR can_delete OR can_grant) <= can_read)` |
| `kb_resource_homes` polymorphic-anchor, no-real-FK idiom | `…schema:274-285` | live `anchor_table` CHECK `IN ('kb_contexts','kb_cogmaps')` |
| cogmaps have **no owner** (join-governed, not owned) | `kb_cogmaps` cols | live: `id,name,telos_resource_id,shape_materialized_event_id,created` — no owner column; `kb_team_cogmaps` = `(cogmap_id, team_id)` m2m |
| profile-owned-edge-home gap **closed** | `20260627000003:30-37` | `anchor_readable_by_profile` `kb_contexts` arm has the `owner_table='kb_profiles' AND owner_id=p_profile` clause |
| team-owned-context is FLAT and paired with addressability | `…06:60-67`, `20260627000001:29-32` | both use direct `kb_team_members`, no `team_ancestors` |
| `vis_team` excludes profile-anchored grants (the leak-safety floor Q-B generalizes) | `…02:196-205` | `vis_team` JOINs only `anchor_table='kb_teams'` |
| memory `project_authorial_rbac_undefined_contexts_cogmaps` cited at `…05:4` is **absent on disk** | comment `…05:4` | `ls memory/ | grep authorial_rbac` → absent (resolved by AC7: memory written, §6) |

**`pg_get_functiondef` is unfakeable** in the way "I read the file" is not (GD-2): the bodies quoted here are what
Postgres has compiled, not what a migration *claims*. The migration source and the live DB agree on every row above.

**The *proposed* artifacts were executably validated too** (GD-2), inside a `BEGIN … ROLLBACK` against the live dev
schema (dev DB untouched): `CREATE TABLE kb_access_grants` + both indexes, all three flipped `CREATE OR REPLACE`
bodies (§3.4), and `explicit_grant`/`profile_explicit_grant`/`cogmap_authorable_by_profile`/`derived_access_profile`/`can()`
(§3.5) — **all compile against the real schema**, and an insert of a `can_write=true, can_read=false` grant is
**rejected by the coherence CHECK** as designed. So §3.3–§3.5 are not just plausible SQL; they parse, resolve every
referenced table/function, and enforce the carried invariant on this exact schema.

### AC7 — the dangling memory reference

`migrations/20260629000005_cogmap_home_authz_and_scope.sql:4` cites `(memory: project_authorial_rbac_undefined_contexts_cogmaps)`,
which does not exist on disk. **Editing the comment is the wrong fix:** sqlx checksums the *entire* migration file,
so altering even a comment in an already-applied migration breaks `sqlx migrate run` on every DB that applied it
(prod/Neon + every dev box) with a checksum mismatch — a violation of the additive-only invariant. The correct,
non-breaking resolution is to **write the missing memory** (done — see §6), which makes the existing comment accurate
without touching the immutable file.

---

## §6 — Closeout

- **Memory written** (AC7): `project_authorial_rbac_undefined_contexts_cogmaps` now exists, capturing the durable
  fact — authorial RBAC on contexts/cogmaps was inexpressible (no explicit-grant rows for those subjects), and this
  arc resolves it via `kb_access_grants` + `cogmap_authorable_by_profile → profile_explicit_grant(…,'write',…)`. The
  comment at `…05:4` is now an accurate reference, no migration edit required.
- **Status → `reviewed`.** The model (§3) is concrete DDL + predicate rewrites; the five open items (§3.6) are
  resolved with rationale; the sequencing (§4) is buildable; grounding (§5) is dual-confirmed (source + live DB).
- **Next (deliverable 2):** introduce `kb_access_grants` + `explicit_grant`/`profile_explicit_grant` + the `can()`
  skeleton **alongside** the existing functions (§4 step 1), then land the new context/cogmap grant capability + the
  `cogmap_authorable_by_profile` rewrite (§4 step 2). No re-design needed to start.
