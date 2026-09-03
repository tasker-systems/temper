#![cfg(feature = "test-db")]

//! The visibility-invariant property harness: randomized worlds, cross-function invariants, and
//! an enrollment gate — the second PR of the scope ruling on the
//! visibility-invariant harness task.
//!
//! # The ruling this harness is built on
//!
//! The visibility functions in `public` fall into four classes, and the class decides what the
//! harness demands of each:
//!
//! - **Set authorities** own a visible-set rule (`resources_visible_to`,
//!   `contexts_readable_by_teams`, `cogmap_visible_maps`, `resources_accessible_to_cogmap`,
//!   `profile_reachable_teams`). Every other function that answers a visibility question must
//!   agree with them.
//! - **Gates** are the point-decisions (`can`, `can_modify_resource`, the
//!   `*_readable/authorable_by_profile` family). The harness holds each gate to its set:
//!   read-duality (`can(read) ⇔ set membership`), write⇒read (bit-CHECK-backed, asserted here
//!   through the derived paths), and gate⇔`can()` dispatch agreement.
//! - **Restatement sites** restate some authority's rule inline (`edges_visible_to`'s
//!   `readable_cogmaps`, `resources_in_team_scope`'s five arms, the search boundary in
//!   `query_find_*`, which hands `resources_visible_to`'s set across to an ungated core). These
//!   are the drift surface — the bite test mutates one and watches the harness go red.
//! - **Composed surfaces** delegate or compute analytics over visible sets (the `graph_*` family,
//!   `_visible_artifacts`, `steward_*`, `visible_region_anchors`, `resources_readable_by`). The
//!   harness demands deny-is-absence (output ⊆ visible) and no value computed over invisible
//!   members (degrees counted over `edges_visible_to` only).
//!
//! # What the invariants are
//!
//! Over a generated world (profiles, team enclosure DAG, memberships+roles, personal/team-owned/
//! team-shared contexts, team-joined cogmaps, homed resources, a randomized grant matrix, a
//! machine-principal cogmap, tombstones, edges, artifacts, search index rows):
//!
//! 1. **Read duality** — `can(principal, 'read', kind, id)` agrees with the kind's visible set,
//!    for profile principals and for the cogmap machine principal; point gates agree with their
//!    set wrappers (`context_visible_to` ⇔ `contexts_readable_by`, etc.).
//! 2. **Write⇒read** — a `write`/`delete`/`grant` admission implies `read` on the same subject;
//!    `can('write', resource)` ⇔ `can_modify_resource` (dispatch agreement).
//! 3. **Deny-is-absence** — `graph_atlas_nodes_visible`, the search doors (`search_exact`,
//!    `search_graph_expand`, `query_find_resources_with`, `query_survey`), `_visible_artifacts`,
//!    and both traversals never emit a row the principal cannot read.
//! 4. **Tombstone floors** — a soft-deleted resource/context, and a soft-deleted team, confer
//!    nothing on any axis for anyone.
//! 5. **No value over invisible members** — `graph_visible_degree_ranking` /
//!    `_bounds` / `graph_atlas_nodes_visible` degrees equal the count of that resource's
//!    `edges_visible_to` edges, computed in Rust from the world.
//! 6. **Cogmap intersection** — a multi-team cogmap's shared (non-interior) accessible set is
//!    contained in EVERY joined team's `vis_team`.
//!
//! # Enrollment, reproducibility, and the bite
//!
//! - The enrolled roster is asserted against the schema by a `pg_proc` scan (the
//!   `reachable_teams_one_definition_test.rs` precedent): any function whose body consults the
//!   substrate must appear in [`ENROLLED`], so a new visibility-answer cannot silently skip the
//!   harness. [`the_enrollment_roster_covers_every_visibility_touching_function`] is that gate.
//! - Worlds come from a seeded xorshift (default [`DEFAULT_SEED`], override with
//!   `HARNESS_SEED`); the plan is generated twice and compared, so a seed reproduces. A failure
//!   prints the seed and the whole world.
//! - [`the_harness_bites_a_search_cte_restatement`] mutates `query_find_exact`'s boundary — the
//!   search CTE's visibility filter — and requires the search invariant to go red, then green
//!   after restore. A harness whose worlds only re-exercise the canonical predicate witnesses
//!   nothing; this is the standing proof that it bites a RESTATEMENT.
//!
//! # Declared remainders
//!
//! - `query_find_wide` is enrolled and existence-pinned but driven only vacuously (zero vector →
//!   empty result ⇒ the ⊆-assertion cannot fail): the embedding axis needs fixture content this
//!   harness does not build. Its boundary mutates identically to `query_find_exact`'s — same
//!   `resources_visible_to` hand-off — so the bite's evidence carries by construction.
//! - `kb_connections` grants have no derived arm and no liveness column; that seam is owned by
//!   `can_subject_liveness_test.rs`, not here.

use sqlx::PgPool;
use uuid::Uuid;

// =================================================================================================
// Seeded RNG — xorshift64*; reproducible worlds from one u64.
// =================================================================================================

const DEFAULT_SEED: u64 = 0x5EED_2026_0903;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn chance(&mut self, pct: u64) -> bool {
        self.next_u64() % 100 < pct
    }
}

fn harness_seed() -> u64 {
    std::env::var("HARNESS_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SEED)
}

// =================================================================================================
// The world plan — fixed topology, randomized grants and extra memberships.
//
// The fixed topology carries arm coverage on every seed (ancestor reach, sibling deny, container
// cascade, owner-anywhere, machine intersection, tombstones); the randomized grant matrix varies
// WHICH principals hold WHAT, so invariants are exercised over facts the fixtures did not
// hand-pick. Invariants are universal — degeneracy weakens evidence, never correctness.
// =================================================================================================

// Teams, by index: (slug, parent index or NONE, alive).
const T_EPD: usize = 0;
const T_ENG: usize = 1;
const T_PAYROLL: usize = 2;
const T_SQUAD: usize = 3;
const T_SEC: usize = 4;
const T_GHOST: usize = 5; // tombstoned below — confers nothing
const TEAM_SLUGS: [&str; 6] = ["epd", "eng", "payroll", "squad", "sec", "ghost"];
const TEAM_PARENTS: [(usize, usize); 5] = [
    (T_EPD, T_ENG),
    (T_ENG, T_PAYROLL),
    (T_PAYROLL, T_SQUAD),
    (T_ENG, T_SEC),
    (T_ENG, T_GHOST),
];

// Profiles, by index: (handle, forced (team, role) memberships).
const P_DANA: usize = 0;
const P_HENRY: usize = 1;
const P_FRANK: usize = 2;
const P_GINA: usize = 3;
const P_OWNER: usize = 4;
const P_OUTSIDER: usize = 5;
const PROFILE_HANDLES: [&str; 6] = ["dana", "henry", "frank", "gina", "owner", "outsider"];
// `owner` and `outsider` stay teamless — the negative controls the precedent file demands.
const FORCED_MEMBERSHIPS: [(usize, usize, &str); 4] = [
    (P_DANA, T_SQUAD, "member"),
    (P_HENRY, T_GHOST, "member"), // only reach is through the tombstoned team
    (P_FRANK, T_SEC, "watcher"),  // read-only role
    (P_GINA, T_ENG, "maintainer"),
];

// Contexts: (slug, owner kind, owner index, shared-to team or NONE, alive).
const C_ENG: usize = 0;
const C_SEC: usize = 1;
const C_SHARED: usize = 2;
const C_PERSONAL: usize = 3;
const C_TOMB: usize = 4; // tombstoned below
const CONTEXT_SPECS: [(&str, &str, usize, Option<usize>); 5] = [
    ("ctx-eng", "kb_teams", T_ENG, None),
    ("ctx-sec", "kb_teams", T_SEC, None),
    ("ctx-shared", "kb_teams", T_ENG, Some(T_SQUAD)),
    ("ctx-personal", "kb_profiles", P_OWNER, None),
    ("ctx-tomb", "kb_teams", T_ENG, None),
];

// Cogmaps: (name, joined teams).
const M_ENG: usize = 0;
const M_SEC: usize = 1;
const M_BRIDGE: usize = 2; // joined to TWO teams — the intersection subject
const M_GRANT: usize = 3; // no teams; grant-admitted only
const COGMAP_SPECS: [(&str, &[usize]); 4] = [
    ("map-eng", &[T_ENG]),
    ("map-sec", &[T_SEC]),
    ("map-bridge", &[T_ENG, T_SEC]),
    ("map-grant", &[]),
];

// Resources: (name token, home anchor, owner profile).
const R_CTX_ENG: usize = 0;
const R_CTX_SEC: usize = 1;
const R_SHARED: usize = 2;
const R_PERSONAL: usize = 3;
const R_MAP_ENG: usize = 4;
const R_MAP_ENG2: usize = 5;
const R_MAP_SEC: usize = 6;
const R_OWNED_ANYWHERE: usize = 7; // homed where dana cannot read; owned by dana — owner arm
const R_ANCESTOR_GRANT: usize = 8; // grant to ENG below — ancestor-grant arm, seed-independent
const R_MAP_GRANT: usize = 9; // cogmap-grant arm, seed-independent
const R_MAP_BRIDGE: usize = 10; // interior of the BRIDGE map — machine-principal interior arm
const R_TOMB: usize = 11; // tombstoned below
const RESOURCE_SPECS: [(&str, Anchor, usize); 12] = [
    ("r_ctx_eng", Anchor::Context(C_ENG), P_OWNER),
    ("r_ctx_sec", Anchor::Context(C_SEC), P_OWNER),
    ("r_shared", Anchor::Context(C_SHARED), P_OWNER),
    ("r_personal", Anchor::Context(C_PERSONAL), P_OWNER),
    ("r_map_eng", Anchor::Cogmap(M_ENG), P_OWNER),
    ("r_map_eng2", Anchor::Cogmap(M_ENG), P_OWNER),
    ("r_map_sec", Anchor::Cogmap(M_SEC), P_OWNER),
    ("r_owned_anywhere", Anchor::Context(C_SEC), P_DANA),
    ("r_ancestor_grant", Anchor::Context(C_SEC), P_OWNER),
    ("r_map_grant", Anchor::Cogmap(M_GRANT), P_OWNER),
    ("r_map_bridge", Anchor::Cogmap(M_BRIDGE), P_OWNER),
    ("r_tomb", Anchor::Context(C_ENG), P_OWNER),
];

// Edges: (source resource, target resource, home anchor, folded).
const EDGE_SPECS: [(usize, usize, Anchor, bool); 4] = [
    (R_MAP_ENG, R_MAP_ENG2, Anchor::Cogmap(M_ENG), false), // inside the cogmap traversal scope
    (R_CTX_SEC, R_MAP_SEC, Anchor::Context(C_SEC), false), // invisible side for dana
    (R_CTX_ENG, R_CTX_SEC, Anchor::Context(C_ENG), true),  // folded — never returned
    (R_MAP_ENG, R_MAP_SEC, Anchor::Context(C_ENG), false), // spans the sibling boundary
];

// Seed-independent grants, so the interesting arms exist on EVERY seed. The randomized matrix
// (below) adds on top of these.
const FIXED_GRANTS: [GrantSpec; 3] = [
    // r_ancestor_grant to ENG: dana reaches it only through the ancestor walk.
    GrantSpec {
        subject: Subject::Resource(R_ANCESTOR_GRANT),
        principal: Principal::Team(T_ENG),
        can_read: true,
        can_write: false,
        can_delete: false,
        can_grant: false,
    },
    // map-grant has no teams; the cogmap READ grant is the whole admission.
    GrantSpec {
        subject: Subject::Cogmap(M_GRANT),
        principal: Principal::Profile(P_DANA),
        can_read: true,
        can_write: false,
        can_delete: false,
        can_grant: false,
    },
    // Anchored to the tombstoned team: must confer nothing (I4), duality-covered via I1.
    GrantSpec {
        subject: Subject::Resource(R_CTX_SEC),
        principal: Principal::Team(T_GHOST),
        can_read: true,
        can_write: false,
        can_delete: false,
        can_grant: false,
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Context(usize),
    Cogmap(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Subject {
    Resource(usize),
    Context(usize),
    Cogmap(usize),
}

impl Subject {
    fn table(&self) -> &'static str {
        match self {
            Subject::Resource(_) => "kb_resources",
            Subject::Context(_) => "kb_contexts",
            Subject::Cogmap(_) => "kb_cogmaps",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Principal {
    Profile(usize),
    Team(usize),
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct GrantSpec {
    subject: Subject,
    principal: Principal,
    can_read: bool,
    can_write: bool,
    can_delete: bool,
    can_grant: bool,
}

/// The generated plan. Two `generate` calls with one seed MUST be equal — asserted in the
/// harness test before anything touches the database.
#[derive(Clone, PartialEq, Debug)]
struct Plan {
    seed: u64,
    grants: Vec<GrantSpec>,
    extra_memberships: Vec<(usize, usize, &'static str)>, // (profile, team, role)
}

impl Plan {
    fn generate(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let mut grants: Vec<GrantSpec> = FIXED_GRANTS.to_vec();
        let mut seen: std::collections::HashSet<(Subject, Principal)> = FIXED_GRANTS
            .iter()
            .map(|g| (g.subject, g.principal))
            .collect();

        // The randomized grant matrix: every subject × every live principal, chance-gated.
        // Bits respect the table's coherence CHECK: nothing without read. The two teamless
        // controls (`owner`, `outsider`) are NOT in the principal pool — their negative face is
        // load-bearing (the arm-characterization test), so they hold nothing but what the
        // fixtures force.
        let subjects = (0..RESOURCE_SPECS.len())
            .map(Subject::Resource)
            .chain((0..CONTEXT_SPECS.len()).map(Subject::Context))
            .chain((0..COGMAP_SPECS.len()).map(Subject::Cogmap));
        let principals: Vec<Principal> = (0..PROFILE_HANDLES.len())
            .filter(|&p| p != P_OWNER && p != P_OUTSIDER)
            .map(Principal::Profile)
            .chain([
                Principal::Team(T_ENG),
                Principal::Team(T_SQUAD),
                Principal::Team(T_SEC),
            ])
            .collect();

        for subject in subjects {
            // Deny sentinels are grant-immune: no randomized grant may land on them, or the
            // negative assertions below (and I4's ghost check) would flip per seed. For
            // r_ctx_sec that means the whole CONTEXT, not just the resource — a grant on the
            // context admits its interior at once. C_PERSONAL is the owner's personal context,
            // the personal-arm negative.
            if matches!(
                subject,
                Subject::Resource(R_CTX_SEC)
                    | Subject::Context(C_SEC)
                    | Subject::Resource(R_PERSONAL)
                    | Subject::Context(C_PERSONAL)
            ) {
                continue;
            }
            for principal in &principals {
                if !rng.chance(22) || seen.contains(&(subject, *principal)) {
                    continue;
                }
                seen.insert((subject, *principal));
                let can_read = rng.chance(70);
                let (w, d, g) = if can_read {
                    (rng.chance(30), rng.chance(20), rng.chance(15))
                } else {
                    (false, false, false)
                };
                grants.push(GrantSpec {
                    subject,
                    principal: *principal,
                    can_read,
                    can_write: w,
                    can_delete: d,
                    can_grant: g,
                });
            }
        }

        // Extra memberships for the worker profiles — never for the teamless controls, and
        // never into T_SEC for dana: her sibling-deny negative (the arm test) needs her OUT of
        // the sec tree on every seed.
        let mut extra_memberships = Vec::new();
        for profile in [P_DANA, P_FRANK, P_GINA] {
            if rng.chance(30) {
                extra_memberships.push((profile, T_PAYROLL, "member"));
            }
        }

        Plan {
            seed,
            grants,
            extra_memberships,
        }
    }
}

struct World {
    plan: Plan,
    teams: Vec<Uuid>,
    profiles: Vec<Uuid>,
    contexts: Vec<Uuid>,
    cogmaps: Vec<Uuid>,
    resources: Vec<Uuid>,
}

impl World {
    /// Renders the whole world for a failure message — seed first, so it can be replayed.
    fn render(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "seed: {} (replay with HARNESS_SEED={})\n",
            self.plan.seed, self.plan.seed
        ));
        s.push_str("teams:\n");
        for (i, id) in self.teams.iter().enumerate() {
            let parent = TEAM_PARENTS
                .iter()
                .find(|(_, child)| *child == i)
                .map(|(parent, _)| TEAM_SLUGS[*parent])
                .unwrap_or("-");
            let alive = if i == T_GHOST { " TOMBSTONED" } else { "" };
            s.push_str(&format!(
                "  [{}] {} (parent {parent}){alive} {id}\n",
                i, TEAM_SLUGS[i]
            ));
        }
        s.push_str("profiles:\n");
        for (i, id) in self.profiles.iter().enumerate() {
            let mut memberships: Vec<String> = FORCED_MEMBERSHIPS
                .iter()
                .filter(|(p, _, _)| *p == i)
                .map(|(_, t, r)| format!("{}:{r}", TEAM_SLUGS[*t]))
                .collect();
            memberships.extend(
                self.plan
                    .extra_memberships
                    .iter()
                    .filter(|(p, _, _)| *p == i)
                    .map(|(_, t, r)| format!("{}:{r} (extra)", TEAM_SLUGS[*t])),
            );
            s.push_str(&format!(
                "  [{}] {} {id} memberships [{}]\n",
                i,
                PROFILE_HANDLES[i],
                memberships.join(", ")
            ));
        }
        s.push_str("contexts:\n");
        for (i, (id, spec)) in self.contexts.iter().zip(CONTEXT_SPECS).enumerate() {
            let alive = if i == C_TOMB { " TOMBSTONED" } else { "" };
            let owner = if spec.1 == "kb_teams" {
                format!("team:{}", TEAM_SLUGS[spec.2])
            } else {
                format!("profile:{}", PROFILE_HANDLES[spec.2])
            };
            s.push_str(&format!(
                "  [{}] {} owner {owner} shared_to {:?}{alive} {id}\n",
                i, spec.0, spec.3
            ));
        }
        s.push_str("cogmaps:\n");
        for (i, (id, spec)) in self.cogmaps.iter().zip(COGMAP_SPECS).enumerate() {
            s.push_str(&format!("  [{}] {} joined {:?} {id}\n", i, spec.0, spec.1));
        }
        s.push_str("resources:\n");
        for (i, (id, spec)) in self.resources.iter().zip(RESOURCE_SPECS).enumerate() {
            let home = match spec.1 {
                Anchor::Context(c) => format!("ctx[{}]", CONTEXT_SPECS[c].0),
                Anchor::Cogmap(m) => format!("map[{}]", COGMAP_SPECS[m].0),
            };
            let alive = if i == R_TOMB { " TOMBSTONED" } else { "" };
            s.push_str(&format!(
                "  [{}] {} home {home} owner {}{alive} {id}\n",
                i, spec.0, PROFILE_HANDLES[spec.2]
            ));
        }
        s.push_str("grants:\n");
        for g in &self.plan.grants {
            let principal = match g.principal {
                Principal::Profile(p) => format!("profile:{}", PROFILE_HANDLES[p]),
                Principal::Team(t) => format!("team:{}", TEAM_SLUGS[t]),
            };
            s.push_str(&format!(
                "  {principal} -> {:?} {:?} r{}w{}d{}g{}\n",
                g.subject.table(),
                match g.subject {
                    Subject::Resource(i) => RESOURCE_SPECS[i].0.to_string(),
                    Subject::Context(i) => CONTEXT_SPECS[i].0.to_string(),
                    Subject::Cogmap(i) => COGMAP_SPECS[i].0.to_string(),
                },
                g.can_read as u8,
                g.can_write as u8,
                g.can_delete as u8,
                g.can_grant as u8,
            ));
        }
        s
    }
}

// =================================================================================================
// Fixture builders (precedent: reachable_teams_one_definition_test.rs).
// =================================================================================================

async fn insert_world(pool: &PgPool, plan: &Plan) -> sqlx::Result<World> {
    let mut teams = Vec::new();
    for slug in TEAM_SLUGS {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
        )
        .bind(slug)
        .fetch_one(pool)
        .await?;
        teams.push(id);
    }
    for (parent, child) in TEAM_PARENTS {
        sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
            .bind(teams[parent])
            .bind(teams[child])
            .execute(pool)
            .await?;
    }

    let mut profiles = Vec::new();
    for handle in PROFILE_HANDLES {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (id, handle, display_name) \
             VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await?;
        profiles.push(id);
    }
    for (profile, team, role) in FORCED_MEMBERSHIPS {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role)",
        )
        .bind(teams[team])
        .bind(profiles[profile])
        .bind(role)
        .execute(pool)
        .await?;
    }
    for (profile, team, role) in &plan.extra_memberships {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role)",
        )
        .bind(teams[*team])
        .bind(profiles[*profile])
        .bind(*role)
        .execute(pool)
        .await?;
    }

    let mut contexts = Vec::new();
    for (slug, owner_table, owner_idx, shared_to) in CONTEXT_SPECS {
        let owner_id = if owner_table == "kb_teams" {
            teams[owner_idx]
        } else {
            profiles[owner_idx]
        };
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
             VALUES (uuid_generate_v7(), $1, $2, $3, $3) RETURNING id",
        )
        .bind(owner_table)
        .bind(owner_id)
        .bind(slug)
        .fetch_one(pool)
        .await?;
        contexts.push(id);
        if let Some(team) = shared_to {
            sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
                .bind(id)
                .bind(teams[team])
                .execute(pool)
                .await?;
        }
    }

    let mut cogmaps = Vec::new();
    for (name, joined) in COGMAP_SPECS {
        let telos: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (id, title, origin_uri) \
             VALUES (uuid_generate_v7(), $1, '') RETURNING id",
        )
        .bind(format!("{name}-telos"))
        .fetch_one(pool)
        .await?;
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_cogmaps (id, name, telos_resource_id) \
             VALUES (uuid_generate_v7(), $1, $2) RETURNING id",
        )
        .bind(name)
        .bind(telos)
        .fetch_one(pool)
        .await?;
        cogmaps.push(id);
        for team in joined {
            sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
                .bind(id)
                .bind(teams[*team])
                .execute(pool)
                .await?;
        }
    }

    let mut resources = Vec::new();
    for (name, anchor, owner) in RESOURCE_SPECS {
        let (anchor_table, anchor_id) = match anchor {
            Anchor::Context(c) => ("kb_contexts", contexts[c]),
            Anchor::Cogmap(m) => ("kb_cogmaps", cogmaps[m]),
        };
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (id, title, origin_uri) \
             VALUES (uuid_generate_v7(), $1, '') RETURNING id",
        )
        .bind(format!("zenith_{name}"))
        .fetch_one(pool)
        .await?;
        sqlx::query(
            "INSERT INTO kb_resource_homes \
               (id, resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES (uuid_generate_v7(), $1, $2, $3, $4, $4)",
        )
        .bind(id)
        .bind(anchor_table)
        .bind(anchor_id)
        .bind(profiles[owner])
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO kb_resource_search_index (resource_id, search_vector) \
             VALUES ($1, to_tsvector('english', $2))",
        )
        .bind(id)
        .bind(format!("zenith_{name}"))
        .execute(pool)
        .await?;
        resources.push(id);
    }

    // Tombstones — flipped, not omitted, so every door must actively exclude them.
    for (table, column, id) in [
        ("kb_resources", "id", resources[R_TOMB]),
        ("kb_contexts", "id", contexts[C_TOMB]),
        ("kb_teams", "id", teams[T_GHOST]),
    ] {
        sqlx::query(&format!(
            "UPDATE {table} SET is_active = false WHERE {column} = $1"
        ))
        .bind(id)
        .execute(pool)
        .await?;
    }

    let subject_id = |subject: Subject| -> Uuid {
        match subject {
            Subject::Resource(i) => resources[i],
            Subject::Context(i) => contexts[i],
            Subject::Cogmap(i) => cogmaps[i],
        }
    };

    for g in &plan.grants {
        let (principal_table, principal_id) = match g.principal {
            Principal::Profile(p) => ("kb_profiles", profiles[p]),
            Principal::Team(t) => ("kb_teams", teams[t]),
        };
        sqlx::query(
            "INSERT INTO kb_access_grants \
               (id, subject_table, subject_id, principal_table, principal_id, \
                can_read, can_write, can_delete, can_grant, granted_by_profile_id) \
             VALUES (uuid_generate_v7(), $1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(g.subject.table())
        .bind(subject_id(g.subject))
        .bind(principal_table)
        .bind(principal_id)
        .bind(g.can_read)
        .bind(g.can_write)
        .bind(g.can_delete)
        .bind(g.can_grant)
        .bind(profiles[P_OWNER])
        .execute(pool)
        .await?;
    }

    for (source, target, home, folded) in EDGE_SPECS {
        let (home_table, home_id) = match home {
            Anchor::Context(c) => ("kb_contexts", contexts[c]),
            Anchor::Cogmap(m) => ("kb_cogmaps", cogmaps[m]),
        };
        sqlx::query(
            "INSERT INTO kb_edges \
               (id, source_table, source_id, target_table, target_id, edge_kind, label, \
                home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id, is_folded) \
             SELECT uuid_generate_v7(), 'kb_resources', $1, 'kb_resources', $2, 'near', 'relates_to', \
                    $3, $4, e.id, e.id, $5 \
               FROM kb_events e ORDER BY e.id LIMIT 1",
        )
        .bind(resources[source])
        .bind(resources[target])
        .bind(home_table)
        .bind(home_id)
        .bind(folded)
        .execute(pool)
        .await?;
    }

    for (resource, folded) in [(R_CTX_ENG, false), (R_CTX_ENG, true), (R_CTX_SEC, false)] {
        sqlx::query(
            "INSERT INTO kb_data_artifacts \
               (id, resource_id, kind_owner_table, kind_owner_id, artifact_kind, intent, \
                content_hash, content_bytes, asserted_by_event_id, last_event_id, is_folded) \
             SELECT uuid_generate_v7(), $1, 'kb_profiles', $3, 'row', 'current', 'h', 0, e.id, e.id, $2 \
               FROM kb_events e ORDER BY e.id LIMIT 1",
        )
        .bind(resources[resource])
        .bind(folded)
        .bind(profiles[P_OWNER])
        .execute(pool)
        .await?;
    }

    Ok(World {
        plan: plan.clone(),
        teams,
        profiles,
        contexts,
        cogmaps,
        resources,
    })
}

// =================================================================================================
// Violations and small query helpers.
// =================================================================================================

type Violation = String;

async fn scalar_set(pool: &PgPool, sql: &str, principal: Uuid) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar(sql)
        .bind(principal)
        .fetch_all(pool)
        .await
}

/// Batched point answers for one principal over a candidate list: `(id, answer)` pairs.
///
/// `expr` is a SINGLE boolean expression over `c.id` in which the principal is `$1::uuid`; the
/// candidate ids are bound from `$2` up.
async fn point_answers(
    pool: &PgPool,
    expr: &str,
    principal: Uuid,
    candidates: &[Uuid],
) -> sqlx::Result<Vec<(Uuid, bool)>> {
    let values: Vec<String> = candidates
        .iter()
        .enumerate()
        // ids start at $2 — the principal inside `expr` is $1.
        .map(|(i, _)| format!("(${}::uuid)", i + 2))
        .collect();
    let sql = format!(
        "SELECT c.id, ({expr}) AS answer FROM (VALUES {}) AS c(id)",
        values.join(", ")
    );
    let mut q = sqlx::query_as::<_, (Uuid, bool)>(&sql).bind(principal);
    for id in candidates {
        q = q.bind(id);
    }
    q.fetch_all(pool).await
}

fn check_duality(
    violations: &mut Vec<Violation>,
    label: &str,
    holder: &str,
    subject: &str,
    id: Uuid,
    set_answer: bool,
    point_answer: bool,
) {
    if set_answer != point_answer {
        violations.push(format!(
            "{label}: {holder} and the set disagree on {subject} {id} — set says {set_answer}, \
             point says {point_answer}"
        ));
    }
}

// =================================================================================================
// Invariant family 1 — read duality: can(read) ⇔ set membership, per principal kind,
// plus the point-gate ⇔ set-wrapper agreements.
// =================================================================================================

async fn i1_read_duality(pool: &PgPool, w: &World) -> sqlx::Result<Vec<Violation>> {
    let mut violations = Vec::new();

    for (p_idx, &profile) in w.profiles.iter().enumerate() {
        let who = PROFILE_HANDLES[p_idx];

        // resources: can(read) ⇔ resources_visible_to
        let visible = scalar_set(
            pool,
            "SELECT resource_id FROM resources_visible_to($1) ORDER BY 1",
            profile,
        )
        .await?;
        let answers = point_answers(
            pool,
            "can('kb_profiles', $1::uuid, 'read', 'kb_resources', c.id)",
            profile,
            &w.resources,
        )
        .await?;
        for (id, can_answer) in answers {
            check_duality(
                &mut violations,
                "I1.resources",
                &format!("can(read) vs resources_visible_to[{who}]"),
                "resource",
                id,
                visible.contains(&id),
                can_answer,
            );
        }

        // contexts: can(read) ⇔ context_readable_by_profile ⇔ membership in contexts_readable_by
        // ⇔ context_visible_to (all four faces of one answer).
        let contexts = scalar_set(
            pool,
            "SELECT context_id FROM contexts_readable_by($1) ORDER BY 1",
            profile,
        )
        .await?;
        let readable_ctx = point_answers(
            pool,
            "context_readable_by_profile($1::uuid, c.id)",
            profile,
            &w.contexts,
        )
        .await?;
        let visible_to = point_answers(
            pool,
            "context_visible_to($1::uuid, c.id)",
            profile,
            &w.contexts,
        )
        .await?;
        let can_ctx = point_answers(
            pool,
            "can('kb_profiles', $1::uuid, 'read', 'kb_contexts', c.id)",
            profile,
            &w.contexts,
        )
        .await?;
        for (((id, by_profile), (_, vis_to)), (_, can_answer)) in
            readable_ctx.iter().zip(&visible_to).zip(&can_ctx)
        {
            check_duality(
                &mut violations,
                "I1.contexts",
                &format!("contexts_readable_by[{who}] vs context_readable_by_profile"),
                "context",
                *id,
                contexts.contains(id),
                *by_profile,
            );
            check_duality(
                &mut violations,
                "I1.contexts",
                &format!("context_visible_to vs context_readable_by_profile[{who}]"),
                "context",
                *id,
                *by_profile,
                *vis_to,
            );
            check_duality(
                &mut violations,
                "I1.contexts",
                &format!("can(read) vs context_readable_by_profile[{who}]"),
                "context",
                *id,
                *by_profile,
                *can_answer,
            );
        }

        // cogmaps: can(read) ⇔ cogmap_readable_by_profile ⇔ membership in cogmap_visible_maps.
        // SETOF uuid — the set IS the scalar column, not a named table column.
        let maps = scalar_set(pool, "SELECT cogmap_visible_maps($1) ORDER BY 1", profile).await?;
        let readable_map = point_answers(
            pool,
            "cogmap_readable_by_profile($1::uuid, c.id)",
            profile,
            &w.cogmaps,
        )
        .await?;
        let can_map = point_answers(
            pool,
            "can('kb_profiles', $1::uuid, 'read', 'kb_cogmaps', c.id)",
            profile,
            &w.cogmaps,
        )
        .await?;
        for ((id, by_profile), (_, can_answer)) in readable_map.iter().zip(&can_map) {
            check_duality(
                &mut violations,
                "I1.cogmaps",
                &format!("cogmap_visible_maps[{who}] vs cogmap_readable_by_profile"),
                "cogmap",
                *id,
                maps.contains(id),
                *by_profile,
            );
            check_duality(
                &mut violations,
                "I1.cogmaps",
                &format!("can(read) vs cogmap_readable_by_profile[{who}]"),
                "cogmap",
                *id,
                *by_profile,
                *can_answer,
            );
        }

        // anchor dispatch on a context anchor agrees with the context gate's own answer.
        let anchor_ctx = point_answers(
            pool,
            "anchor_readable_by_profile($1::uuid, 'kb_contexts', c.id)",
            profile,
            &w.contexts,
        )
        .await?;
        for ((id, anchor_answer), (_, by_profile)) in anchor_ctx.iter().zip(&readable_ctx) {
            check_duality(
                &mut violations,
                "I1.anchors",
                &format!("anchor_readable vs context_readable_by_profile[{who}]"),
                "context",
                *id,
                *by_profile,
                *anchor_answer,
            );
        }

        // endpoint dispatch agrees with the per-kind gates, on both kinds it dispatches.
        let endpoint_res = point_answers(
            pool,
            "endpoint_readable_by_profile($1::uuid, 'kb_resources', c.id)",
            profile,
            &w.resources,
        )
        .await?;
        for (id, endpoint_answer) in &endpoint_res {
            check_duality(
                &mut violations,
                "I1.endpoints",
                &format!("endpoint_readable vs resources_visible_to[{who}]"),
                "resource",
                *id,
                visible.contains(id),
                *endpoint_answer,
            );
        }
        let endpoint_map = point_answers(
            pool,
            "endpoint_readable_by_profile($1::uuid, 'kb_cogmaps', c.id)",
            profile,
            &w.cogmaps,
        )
        .await?;
        for ((id, endpoint_answer), (_, by_profile)) in endpoint_map.iter().zip(&readable_map) {
            check_duality(
                &mut violations,
                "I1.endpoints",
                &format!("endpoint_readable vs cogmap_readable_by_profile[{who}]"),
                "cogmap",
                *id,
                *by_profile,
                *endpoint_answer,
            );
        }

        // composed surfaces: region anchors and steward candidates are unions of the sets.
        let region = scalar_set(
            pool,
            "SELECT anchor_id FROM visible_region_anchors($1) WHERE anchor_table = 'kb_contexts' \
             ORDER BY 1",
            profile,
        )
        .await?;
        if region != contexts {
            let missing: Vec<_> = contexts.iter().filter(|c| !region.contains(c)).collect();
            let extra: Vec<_> = region.iter().filter(|r| !contexts.contains(r)).collect();
            violations.push(format!(
                "I1.composed: visible_region_anchors context face differs from \
                 contexts_readable_by[{who}] — missing {missing:?}, extra {extra:?}"
            ));
        }

        let candidates = scalar_set(
            pool,
            "SELECT cogmap_id FROM steward_candidate_cogmaps($1) ORDER BY 1",
            profile,
        )
        .await?;
        // steward candidates iterate kb_team_cogmaps rows — a teamless, grant-admitted map
        // (map-grant) is deliberately NOT a candidate, so the comparator is the team-joined
        // maps filtered by readability, NOT cogmap_visible_maps.
        for (id, by_profile) in &readable_map {
            let is_team_joined = w
                .cogmaps
                .iter()
                .position(|c| c == id)
                .map(|idx| !COGMAP_SPECS[idx].1.is_empty())
                .unwrap_or(false);
            if candidates.contains(id) != (is_team_joined && *by_profile) {
                violations.push(format!(
                    "I1.composed: steward_candidate_cogmaps[{who}] disagrees with \
                     anchor_readable over team-joined cogmap {id}"
                ));
            }
        }

        // resources_readable_by's profile face IS the visible set.
        let readable_resources = scalar_set(
            pool,
            "SELECT resource_id FROM resources_readable_by('profile', $1) ORDER BY 1",
            profile,
        )
        .await?;
        if readable_resources != visible {
            violations.push(format!(
                "I1.composed: resources_readable_by('profile') differs from \
                 resources_visible_to[{who}]"
            ));
        }
    }

    // the machine principal: can('kb_cogmaps', m, 'read', r) ⇔ resources_accessible_to_cogmap(m)
    for (m_idx, &map) in w.cogmaps.iter().enumerate() {
        let accessible = scalar_set(
            pool,
            "SELECT resource_id FROM resources_accessible_to_cogmap($1) ORDER BY 1",
            map,
        )
        .await?;
        let answers = point_answers(
            pool,
            "can('kb_cogmaps', $1::uuid, 'read', 'kb_resources', c.id)",
            map,
            &w.resources,
        )
        .await?;
        for (id, can_answer) in answers {
            check_duality(
                &mut violations,
                "I1.machine",
                &format!(
                    "can vs resources_accessible_to_cogmap[{}]",
                    COGMAP_SPECS[m_idx].0
                ),
                "resource",
                id,
                accessible.contains(&id),
                can_answer,
            );
        }
    }

    Ok(violations)
}

// =================================================================================================
// Invariant family 2 — write⇒read, and can('write') ⇔ the concrete write gates.
// =================================================================================================

async fn i2_write_implies_read(pool: &PgPool, w: &World) -> sqlx::Result<Vec<Violation>> {
    let mut violations = Vec::new();

    for (p_idx, &profile) in w.profiles.iter().enumerate() {
        let who = PROFILE_HANDLES[p_idx];

        for (table, ids) in [
            ("kb_resources", &w.resources),
            ("kb_contexts", &w.contexts),
            ("kb_cogmaps", &w.cogmaps),
        ] {
            let writes = point_answers(
                pool,
                &format!("can('kb_profiles', $1::uuid, 'write', '{table}', c.id)"),
                profile,
                ids,
            )
            .await?;
            let reads = point_answers(
                pool,
                &format!("can('kb_profiles', $1::uuid, 'read', '{table}', c.id)"),
                profile,
                ids,
            )
            .await?;
            for ((id, write), (_, read)) in writes.iter().zip(&reads) {
                if *write && !read {
                    violations.push(format!(
                        "I2: can(write) without can(read) — profile {who}, {table} {id}"
                    ));
                }
            }
        }

        // resource write dispatch: can('write', resource) ⇔ can_modify_resource
        let can_write = point_answers(
            pool,
            "can('kb_profiles', $1::uuid, 'write', 'kb_resources', c.id)",
            profile,
            &w.resources,
        )
        .await?;
        let can_modify = point_answers(
            pool,
            "can_modify_resource($1::uuid, c.id)",
            profile,
            &w.resources,
        )
        .await?;
        for ((id, write), (_, modify)) in can_write.iter().zip(&can_modify) {
            if write != modify {
                violations.push(format!(
                    "I2: can(write) vs can_modify_resource disagree — profile {who}, \
                     resource {id}: can says {write}, can_modify says {modify}"
                ));
            }
        }

        // context write dispatch: can('write', context) ⇔ context_authorable_by_profile
        let can_write_ctx = point_answers(
            pool,
            "can('kb_profiles', $1::uuid, 'write', 'kb_contexts', c.id)",
            profile,
            &w.contexts,
        )
        .await?;
        let authorable = point_answers(
            pool,
            "context_authorable_by_profile($1::uuid, c.id)",
            profile,
            &w.contexts,
        )
        .await?;
        for ((id, write), (_, authorable)) in can_write_ctx.iter().zip(&authorable) {
            if write != authorable {
                violations.push(format!(
                    "I2: can(write) vs context_authorable_by_profile disagree — profile {who}, \
                     context {id}: can says {write}, authorable says {authorable}"
                ));
            }
        }

        // cogmap write dispatch: can('write', cogmap) ⇔ cogmap_authorable_by_profile
        let can_write_map = point_answers(
            pool,
            "can('kb_profiles', $1::uuid, 'write', 'kb_cogmaps', c.id)",
            profile,
            &w.cogmaps,
        )
        .await?;
        let authorable = point_answers(
            pool,
            "cogmap_authorable_by_profile($1::uuid, c.id)",
            profile,
            &w.cogmaps,
        )
        .await?;
        for ((id, write), (_, authorable)) in can_write_map.iter().zip(&authorable) {
            if write != authorable {
                violations.push(format!(
                    "I2: can(write) vs cogmap_authorable_by_profile disagree — profile {who}, \
                     cogmap {id}: can says {write}, authorable says {authorable}"
                ));
            }
        }
    }

    Ok(violations)
}

// =================================================================================================
// Invariant family 3 — deny-is-absence: every read path returns only readable rows.
// Also the one the BITE aims at: `search_slice` is the search-door invariant.
// =================================================================================================

/// The search-door invariant, split out so the bite can aim at exactly it.
async fn search_slice(pool: &PgPool, w: &World) -> sqlx::Result<Vec<Violation>> {
    let mut violations = Vec::new();

    for (p_idx, &profile) in w.profiles.iter().enumerate() {
        let who = PROFILE_HANDLES[p_idx];
        let visible = scalar_set(
            pool,
            "SELECT resource_id FROM resources_visible_to($1) ORDER BY 1",
            profile,
        )
        .await?;

        // search_exact per title token — the production search door.
        for (r_idx, _) in w.resources.iter().enumerate() {
            let title = format!("zenith_{}", RESOURCE_SPECS[r_idx].0);
            let found: Vec<Uuid> =
                sqlx::query_scalar("SELECT resource_id FROM search_exact($1, $2) ORDER BY 1")
                    .bind(profile)
                    .bind(&title)
                    .fetch_all(pool)
                    .await?;
            let leaked: Vec<_> = found.iter().filter(|id| !visible.contains(id)).collect();
            if !leaked.is_empty() {
                violations.push(format!(
                    "I3.search: search_exact[{who}, '{title}'] returned unreadable resources \
                     {leaked:?}"
                ));
            }
        }

        // graph expansion from a visible seed — every expansion stays inside the visible set.
        let seed = visible.first().copied();
        if let Some(seed) = seed {
            let expanded: Vec<Uuid> = sqlx::query_scalar(
                "SELECT resource_id FROM search_graph_expand($1, $2, 2, NULL, \
                 0.5::double precision) ORDER BY 1",
            )
            .bind(profile)
            .bind(vec![seed])
            .fetch_all(pool)
            .await?;
            let leaked: Vec<_> = expanded.iter().filter(|id| !visible.contains(id)).collect();
            if !leaked.is_empty() {
                violations.push(format!(
                    "I3.search: search_graph_expand[{who}] leaked unreadable resources {leaked:?}"
                ));
            }
        }

        // the survey door with no embedding: same boundary set, same demand.
        let surveyed: Vec<Uuid> =
            sqlx::query_scalar("SELECT resource_id FROM query_survey($1) ORDER BY 1")
                .bind(profile)
                .fetch_all(pool)
                .await?;
        let leaked: Vec<_> = surveyed.iter().filter(|id| !visible.contains(id)).collect();
        if !leaked.is_empty() {
            violations.push(format!(
                "I3.search: query_survey[{who}] leaked unreadable resources {leaked:?}"
            ));
        }

        // the unfiltered filter door: doc-type search with no filters is bounded by the set.
        let filtered: Vec<Uuid> =
            sqlx::query_scalar("SELECT resource_id FROM query_find_resources_with($1) ORDER BY 1")
                .bind(profile)
                .fetch_all(pool)
                .await?;
        let leaked: Vec<_> = filtered.iter().filter(|id| !visible.contains(id)).collect();
        if !leaked.is_empty() {
            violations.push(format!(
                "I3.search: query_find_resources_with[{who}] leaked unreadable resources \
                 {leaked:?}"
            ));
        }
    }

    Ok(violations)
}

async fn i3_deny_is_absence(pool: &PgPool, w: &World) -> sqlx::Result<Vec<Violation>> {
    let mut violations = search_slice(pool, w).await?;

    for (p_idx, &profile) in w.profiles.iter().enumerate() {
        let who = PROFILE_HANDLES[p_idx];
        let visible = scalar_set(
            pool,
            "SELECT resource_id FROM resources_visible_to($1) ORDER BY 1",
            profile,
        )
        .await?;

        // atlas: unseen ids drop out (deny-as-absence join).
        let atlas: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM graph_atlas_nodes_visible($1, $2) ORDER BY 1")
                .bind(profile)
                .bind(&w.resources)
                .fetch_all(pool)
                .await?;
        let leaked: Vec<_> = atlas.iter().filter(|id| !visible.contains(id)).collect();
        if !leaked.is_empty() {
            violations.push(format!(
                "I3.atlas: graph_atlas_nodes_visible[{who}] emitted unreadable ids {leaked:?}"
            ));
        }

        // artifacts: a resource's artifacts exist only behind its resource's visibility.
        for (r_idx, resource) in w.resources.iter().enumerate() {
            let artifacts: Vec<Uuid> =
                sqlx::query_scalar("SELECT id FROM _visible_artifacts($1, $2) ORDER BY 1")
                    .bind(profile)
                    .bind(resource)
                    .fetch_all(pool)
                    .await?;
            if !artifacts.is_empty() && !visible.contains(resource) {
                violations.push(format!(
                    "I3.artifacts: _visible_artifacts[{who}] returned rows for unreadable \
                     resource {}",
                    RESOURCE_SPECS[r_idx].0,
                ));
            }
        }

        // traversals: endpoints stay inside their scope, and every returned edge is one the
        // edges_visible_to admits.
        let visible_edges = scalar_set(
            pool,
            "SELECT edge_id FROM edges_visible_to($1) ORDER BY 1",
            profile,
        )
        .await?;

        let team_edges: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM graph_traverse_scoped($1, $2, $3, 3, NULL) ORDER BY 1",
        )
        .bind(profile)
        .bind(w.teams[T_ENG])
        .bind(&w.resources)
        .fetch_all(pool)
        .await?;
        let leaked: Vec<_> = team_edges
            .iter()
            .filter(|e| !visible_edges.contains(e))
            .collect();
        if !leaked.is_empty() {
            violations.push(format!(
                "I3.traverse: graph_traverse_scoped[{who}] returned edges edges_visible_to does \
                 not admit: {leaked:?}"
            ));
        }

        let cogmap_edges: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM graph_traverse_cogmap_scoped($1, $2, $3, 3, NULL) ORDER BY 1",
        )
        .bind(profile)
        .bind(w.cogmaps[M_ENG])
        .bind(&w.resources)
        .fetch_all(pool)
        .await?;
        let leaked: Vec<_> = cogmap_edges
            .iter()
            .filter(|e| !visible_edges.contains(e))
            .collect();
        if !leaked.is_empty() {
            violations.push(format!(
                "I3.traverse: graph_traverse_cogmap_scoped[{who}] returned edges \
                 edges_visible_to does not admit: {leaked:?}"
            ));
        }
    }

    Ok(violations)
}

// =================================================================================================
// Invariant family 4 — tombstone floors.
// =================================================================================================

async fn i4_tombstone_floors(pool: &PgPool, w: &World) -> sqlx::Result<Vec<Violation>> {
    let mut violations = Vec::new();
    let dead_resource = w.resources[R_TOMB];
    let dead_context = w.contexts[C_TOMB];

    for (p_idx, &profile) in w.profiles.iter().enumerate() {
        let who = PROFILE_HANDLES[p_idx];

        let visible: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
        )
        .bind(profile)
        .bind(dead_resource)
        .fetch_one(pool)
        .await?;
        let can_read: bool =
            sqlx::query_scalar("SELECT can('kb_profiles', $1, 'read', 'kb_resources', $2)")
                .bind(profile)
                .bind(dead_resource)
                .fetch_one(pool)
                .await?;
        let can_write: bool =
            sqlx::query_scalar("SELECT can('kb_profiles', $1, 'write', 'kb_resources', $2)")
                .bind(profile)
                .bind(dead_resource)
                .fetch_one(pool)
                .await?;
        let modify: bool = sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
            .bind(profile)
            .bind(dead_resource)
            .fetch_one(pool)
            .await?;
        if visible || can_read || can_write || modify {
            violations.push(format!(
                "I4: tombstoned resource r_tomb answers true for {who} — visible {visible}, \
                 can_read {can_read}, can_write {can_write}, can_modify {modify}"
            ));
        }

        // the tombstoned CONTEXT: unreadable, unauthorable, absent from the set — even for a
        // principal holding an explicit grant on it (the liveness floor's job).
        let readable: bool = sqlx::query_scalar("SELECT context_readable_by_profile($1, $2)")
            .bind(profile)
            .bind(dead_context)
            .fetch_one(pool)
            .await?;
        let authorable: bool = sqlx::query_scalar("SELECT context_authorable_by_profile($1, $2)")
            .bind(profile)
            .bind(dead_context)
            .fetch_one(pool)
            .await?;
        let in_set: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM contexts_readable_by($1) c WHERE c.context_id = $2)",
        )
        .bind(profile)
        .bind(dead_context)
        .fetch_one(pool)
        .await?;
        if readable || authorable || in_set {
            violations.push(format!(
                "I4: tombstoned context ctx-tomb answers true for {who} — readable {readable}, \
                 authorable {authorable}, in_set {in_set}"
            ));
        }

        // the tombstoned TEAM confers no reach for anyone.
        let reaches: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM profile_reachable_teams($1) r \
              WHERE r.team_id = $2)",
        )
        .bind(profile)
        .bind(w.teams[T_GHOST])
        .fetch_one(pool)
        .await?;
        if reaches {
            violations.push(format!(
                "I4: tombstoned team ghost appears in profile_reachable_teams[{who}]"
            ));
        }
    }

    // henry's ONLY membership is the ghost. Personal teams nest under the migration's root
    // team, so {personal, root} reach is by design — but no FIXTURE team may leak into his
    // reach: not the ghost, nor any live one.
    let henry_reach: Vec<Uuid> =
        sqlx::query_scalar("SELECT team_id FROM profile_reachable_teams($1) ORDER BY 1")
            .bind(w.profiles[P_HENRY])
            .fetch_all(pool)
            .await?;
    let leaked_teams: Vec<_> = henry_reach.iter().filter(|t| w.teams.contains(t)).collect();
    if !leaked_teams.is_empty() {
        violations.push(format!(
            "I4: henry (member of the tombstoned ghost only) reaches fixture team(s) \
             {leaked_teams:?}"
        ));
    }

    // The ghost's own grant (fixed grant 3) must confer nothing: r_ctx_sec is otherwise
    // unreachable for henry, so it must be invisible to him.
    let henry_sees: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) v \
          WHERE v.resource_id = $2)",
    )
    .bind(w.profiles[P_HENRY])
    .bind(w.resources[R_CTX_SEC])
    .fetch_one(pool)
    .await?;
    if henry_sees {
        violations.push(
            "I4: the ghost team's grant leaked — r_ctx_sec visible to henry through a \
             tombstoned team"
                .to_string(),
        );
    }

    Ok(violations)
}

// =================================================================================================
// Invariant family 5 — no value computed over invisible members: degrees count only
// edges_visible_to edges.
// =================================================================================================

async fn i5_degrees_over_visible_edges_only(
    pool: &PgPool,
    w: &World,
) -> sqlx::Result<Vec<Violation>> {
    let mut violations = Vec::new();

    for (p_idx, &profile) in w.profiles.iter().enumerate() {
        let who = PROFILE_HANDLES[p_idx];
        let visible_edges = scalar_set(
            pool,
            "SELECT edge_id FROM edges_visible_to($1) ORDER BY 1",
            profile,
        )
        .await?;

        // expected degree per resource, from the world's own edge list filtered to visible edges.
        let mut expected: std::collections::HashMap<Uuid, i32> = std::collections::HashMap::new();
        for (source, target, home, folded) in EDGE_SPECS {
            let (home_table, home_id) = match home {
                Anchor::Context(c) => ("kb_contexts", w.contexts[c]),
                Anchor::Cogmap(m) => ("kb_cogmaps", w.cogmaps[m]),
            };
            let edge_id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM kb_edges \
                  WHERE source_id = $1 AND target_id = $2 \
                    AND home_anchor_table = $3 AND home_anchor_id = $4 AND is_folded = $5",
            )
            .bind(w.resources[source])
            .bind(w.resources[target])
            .bind(home_table)
            .bind(home_id)
            .bind(folded)
            .fetch_optional(pool)
            .await?;
            if let Some(edge_id) = edge_id {
                if visible_edges.contains(&edge_id) {
                    *expected.entry(w.resources[source]).or_insert(0) += 1;
                    if w.resources[source] != w.resources[target] {
                        *expected.entry(w.resources[target]).or_insert(0) += 1;
                    }
                }
            }
        }
        for id in &w.resources {
            expected.entry(*id).or_insert(0);
        }

        // ranking: rows ⊆ visible, and each degree equals the expected count.
        let ranking: Vec<(Uuid, i32)> = sqlx::query_as(
            "SELECT resource_id, degree FROM graph_visible_degree_ranking($1, NULL, 0, 1000) \
             ORDER BY 1",
        )
        .bind(profile)
        .fetch_all(pool)
        .await?;
        let visible = scalar_set(
            pool,
            "SELECT resource_id FROM resources_visible_to($1) ORDER BY 1",
            profile,
        )
        .await?;
        for (id, degree) in &ranking {
            if !visible.contains(id) {
                violations.push(format!(
                    "I5: graph_visible_degree_ranking[{who}] ranked an unreadable resource {id}"
                ));
            }
            let want = expected.get(id).copied().unwrap_or(0);
            if *degree != want {
                violations.push(format!(
                    "I5: degree over invisible edges — ranking[{who}] says resource {id} has \
                     degree {degree}, edges_visible_to admits {want}"
                ));
            }
        }

        // bounds: in_scope equals the visible count; eligible equals the expected filter at
        // min_degree 1 — a real cut, not the degenerate ≥0 one.
        let (in_scope, eligible): (i32, i32) = sqlx::query_as(
            "SELECT in_scope, eligible FROM graph_visible_degree_bounds($1, NULL, 1)",
        )
        .bind(profile)
        .fetch_one(pool)
        .await?;
        let want_eligible = expected
            .iter()
            .filter(|(id, degree)| visible.contains(id) && **degree >= 1)
            .count() as i32;
        if in_scope != visible.len() as i32 || eligible != want_eligible {
            violations.push(format!(
                "I5: graph_visible_degree_bounds[{who}] says in_scope {in_scope}, eligible \
                 {eligible}; visible set is {}, eligible should be {want_eligible}",
                visible.len()
            ));
        }

        // atlas degrees agree too.
        let atlas: Vec<(Uuid, i32)> =
            sqlx::query_as("SELECT id, degree FROM graph_atlas_nodes_visible($1, $2) ORDER BY 1")
                .bind(profile)
                .bind(&w.resources)
                .fetch_all(pool)
                .await?;
        for (id, degree) in &atlas {
            let want = expected.get(id).copied().unwrap_or(0);
            if *degree != want {
                violations.push(format!(
                    "I5: atlas degree over invisible edges — atlas[{who}] says resource {id} has \
                     degree {degree}, edges_visible_to admits {want}"
                ));
            }
        }
    }

    Ok(violations)
}

// =================================================================================================
// Invariant family 6 — a multi-team cogmap's shared accessible set ⊆ every joined team's vis.
// =================================================================================================

async fn i6_cogmap_intersection(pool: &PgPool, w: &World) -> sqlx::Result<Vec<Violation>> {
    let mut violations = Vec::new();

    for (m_idx, &map) in w.cogmaps.iter().enumerate() {
        let (name, joined) = COGMAP_SPECS[m_idx];
        if joined.len() < 2 {
            continue;
        }

        let accessible = scalar_set(
            pool,
            "SELECT resource_id FROM resources_accessible_to_cogmap($1) ORDER BY 1",
            map,
        )
        .await?;

        // interior = homes anchored to this map; shared = accessible − interior.
        let interior: Vec<Uuid> = sqlx::query_scalar(
            "SELECT h.resource_id FROM kb_resource_homes h \
              WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = $1 ORDER BY 1",
        )
        .bind(map)
        .fetch_all(pool)
        .await?;
        let shared: Vec<Uuid> = accessible
            .iter()
            .filter(|r| !interior.contains(r))
            .copied()
            .collect();

        let mut team_views = Vec::new();
        for &team_idx in joined {
            let view = scalar_set(
                pool,
                "SELECT resource_id FROM vis_team($1) ORDER BY 1",
                w.teams[team_idx],
            )
            .await?;
            team_views.push((team_idx, view));
        }

        for resource in &shared {
            for (team_idx, view) in &team_views {
                if !view.contains(resource) {
                    violations.push(format!(
                        "I6: cogmap {name}'s shared accessible set leaks — resource {resource} \
                         is accessible to the map but NOT in vis_team({})",
                        TEAM_SLUGS[*team_idx]
                    ));
                }
            }
        }
    }

    Ok(violations)
}

// =================================================================================================
// The enrollment gate — a pg_proc scan, not a fixed list, so a new visibility-answer cannot
// silently skip the harness (precedent: reachable_teams_one_definition_test.rs).
// =================================================================================================

/// Every function the harness drives or pins. A scan hit outside this list FAILS — add the
/// function here AND give it an invariant above, or explain in this list's doc why existence
/// alone is the right demand.
const ENROLLED: &[&str] = &[
    // set authorities
    "resources_visible_to",
    "contexts_readable_by",
    "contexts_readable_by_teams",
    "cogmap_visible_maps",
    "resources_accessible_to_cogmap",
    "profile_reachable_teams",
    // gates
    "can",
    "can_modify_resource",
    "cogmap_readable_by_profile",
    "cogmap_authorable_by_profile",
    "context_readable_by_profile",
    "context_visible_to",
    "context_authorable_by_profile",
    "anchor_readable_by_profile",
    "endpoint_readable_by_profile",
    "profile_explicit_grant",
    "derived_access_profile",
    // restatement sites
    "edges_visible_to",
    "resources_in_team_scope",
    "resources_in_cogmap_scope",
    "query_find_exact",
    "query_find_wide",
    "query_follow_from",
    "query_survey",
    "query_find_resources_with",
    // composed surfaces
    "_visible_artifacts",
    "resources_readable_by",
    "steward_candidate_cogmaps",
    "steward_authorable_cogmaps",
    "visible_region_anchors",
    "graph_atlas_nodes_visible",
    "graph_visible_degree_bounds",
    "graph_visible_degree_ranking",
    "graph_traverse_scoped",
    "graph_traverse_cogmap_scoped",
    // search doors (wrappers over the gated query_* machines)
    "search_exact",
    "search_wide",
    "search_graph_expand",
];

/// The substrate the substrate-existence half pins; excluded from the scan because these are the
/// PRIMITIVES every answer routes through, not answers themselves.
const PRIMITIVES: &[&str] = &["team_ancestors", "profile_effective_teams", "vis_team"];

/// Acknowledged CONSUMERS of the substrate: functions that CALL the driven authorities (which is
/// how the scan caught them) but have no bespoke invariant here.
///
/// The demand on this tier is weaker than on [`ENROLLED`], and that is a stated scope decision,
/// not an oversight: the universal invariants bind every authority these functions call, and each
/// name below was read — they route through the authorities rather than restating their rules
/// inline. A function that restates WITHOUT calling any authority is invisible to the vocabulary
/// scan in both designs; that is the scan's declared blind spot (see its doc). Bespoke
/// per-consumer invariants (the `reachable_teams_one_definition_test.rs` fold-site standard) are
/// the follow-up scope for the highest-risk names here — `cogmap_list_rows` and
/// `graph_home_cogmaps` already carry such characterization there.
///
/// The gate's property is unaffected: a NEW function consulting the substrate matches neither
/// list and fails, forcing the drive-or-acknowledge decision explicitly.
const CONSUMES: &[&str] = &[
    "anchor_region_metrics",
    "anchor_shape",
    "anchor_staleness",
    "artifact_by_id",
    "audit_drift_sweep",
    "cogmap_analytics",
    "cogmap_foundations",
    "cogmap_list_rows",
    "element_trail_edge",
    "element_trail_node",
    "graph_atlas_nodes",
    "graph_atlas_nodes_cogmap",
    "graph_cogmap_orphan_nodes",
    "graph_cogmap_territories",
    "graph_context_containers",
    "graph_context_residual_counts",
    "graph_context_residual_members",
    "graph_context_territories",
    "graph_home_cogmaps",
    "graph_home_contexts",
    "graph_home_teams",
    "graph_induced_edges",
    "graph_orphan_salient_nodes",
    "graph_region_composition_edges",
    "graph_region_members",
    "graph_region_territories",
    "graph_territory_bridges",
    "graph_traverse",
    "resource_auditable_citations",
    "resource_lineage",
    "resource_stale_citations_multi",
    "shape_by_id",
    "shapes_for_home",
    "steward_team_contexts",
];

/// Any function whose body consults the visibility substrate must be enrolled. The vocabulary is
/// the substrate's NAMES: a function that consults visibility without touching any of them is not
/// answerable from this scan — that residue is the scan's stated blind spot, not a covered one.
const VISIBILITY_VOCABULARY: &str =
    "(resources_visible_to|contexts_readable_by|cogmap_visible_maps|\
     cogmap_readable_by_profile|context_readable_by_profile|context_visible_to|\
     resources_accessible_to_cogmap|anchor_readable_by_profile|endpoint_readable_by_profile|\
     can_modify_resource|profile_reachable_teams|resources_in_team_scope|resources_in_cogmap_scope|\
     edges_visible_to|derived_access_profile|profile_explicit_grant|\\mcan\\()";

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_enrollment_roster_covers_every_visibility_touching_function(
    pool: PgPool,
) -> sqlx::Result<()> {
    // Anti-vacuity: the authorities and the primitives must all exist, and the scan must hit.
    for name in ENROLLED.iter().chain(PRIMITIVES) {
        let n: Option<i64> = sqlx::query_scalar(
            "SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
              WHERE n.nspname = 'public' AND p.prokind = 'f' AND p.proname = $1",
        )
        .bind(name)
        .fetch_one(&pool)
        .await?;
        assert!(
            n.unwrap_or(0) > 0,
            "enrolled function {name} does not exist — the roster is stale; remove it"
        );
    }

    let scan_hits: Vec<String> = sqlx::query_scalar(
        "SELECT p.proname \
           FROM pg_proc p \
           JOIN pg_namespace n ON n.oid = p.pronamespace \
           CROSS JOIN LATERAL (SELECT pg_get_functiondef(p.oid) AS def) d \
          WHERE n.nspname = 'public' AND p.prokind = 'f' \
            AND d.def ~ $1 \
            AND p.proname <> ALL($2) \
          ORDER BY 1",
    )
    .bind(VISIBILITY_VOCABULARY)
    .bind(
        ENROLLED
            .iter()
            .chain(PRIMITIVES.iter())
            .chain(CONSUMES.iter())
            .copied()
            .collect::<Vec<&str>>(),
    )
    .fetch_all(&pool)
    .await?;

    assert!(
        !scan_hits.is_empty(),
        "the enrollment scan returned nothing — the vocabulary regex no longer matches anything, \
         so this gate is vacuous"
    );

    // The ungated cores take the visible set handed IN; each one behind a gated wrapper is
    // already enrolled through that wrapper.
    let unmanaged: Vec<_> = scan_hits
        .into_iter()
        .filter(|n| !n.starts_with("__temper_ungated"))
        .collect();

    assert!(
        unmanaged.is_empty(),
        "{} visibility-touching function(s) are NOT enrolled in the invariant harness: {:?}\n\
         \n\
         A function that consults the visibility substrate must be driven by \
         visibility_invariant_harness.rs — add it to ENROLLED and give it an invariant, or it \
         can drift from the authorities with nothing watching.\n\
         \n\
         If the function consults the substrate for an unrelated reason, say so explicitly and \
         exclude it here — do not silently enroll it.",
        unmanaged.len(),
        unmanaged
    );

    Ok(())
}

/// The fixed topology's meaning: every admission arm the schema has is exercised by SOME
/// seed-independent fixture, and the negative controls hold. These are characterizations — they
/// pin that the world stays non-degenerate as the schema and the generator evolve; the invariants
/// above are what hold the doors to each other. A green here says the harness's worlds still
/// exercise the arms; it is not itself the drift witness.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_world_exercises_its_arms(pool: PgPool) -> sqlx::Result<()> {
    let plan = Plan::generate(harness_seed());
    let w = insert_world(&pool, &plan).await?;

    let sees = |who: usize, resource: usize| {
        let pool = pool.clone();
        let profile = w.profiles[who];
        let resource = w.resources[resource];
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) v \
                  WHERE v.resource_id = $2)",
            )
            .bind(profile)
            .bind(resource)
            .fetch_one(&pool)
            .await
        }
    };

    // dana's seed-independent admissions — one per arm.
    for (resource, arm) in [
        (R_CTX_ENG, "context read: ancestor team's OWN context"),
        (R_SHARED, "context read: context SHARED to dana's team"),
        (R_MAP_ENG, "cogmap join: map joined to an ancestor team"),
        (R_ANCESTOR_GRANT, "team-anchored grant on an ANCESTOR team"),
        (R_MAP_GRANT, "explicit read-grant on a teamless cogmap home"),
        (
            R_OWNED_ANYWHERE,
            "owner arm: owned resource in an UNREADABLE context",
        ),
    ] {
        assert!(
            sees(P_DANA, resource).await?,
            "arm broken: dana cannot see fixture {resource:?} via {arm}"
        );
    }
    assert!(
        !sees(P_DANA, R_CTX_SEC).await?,
        "negative control broken: dana sees a sibling team's context resource"
    );
    assert!(
        !sees(P_DANA, R_PERSONAL).await?,
        "negative control broken: dana sees into the owner's personal context"
    );

    // the machine interior arm: a resource homed IN the bridge map is accessible to the map's
    // machine principal regardless of teams or grants.
    let interior: bool =
        sqlx::query_scalar("SELECT can('kb_cogmaps', $1, 'read', 'kb_resources', $2)")
            .bind(w.cogmaps[M_BRIDGE])
            .bind(w.resources[R_MAP_BRIDGE])
            .fetch_one(&pool)
            .await?;
    assert!(
        interior,
        "arm broken: the bridge map cannot read its own interior"
    );

    // the negative controls, clean by construction (the generator excludes them from the
    // randomized matrix — see Plan::generate).
    for (resource, spec) in RESOURCE_SPECS.iter().enumerate() {
        assert!(
            !sees(P_OUTSIDER, resource).await?,
            "negative control broken: outsider sees fixture {:?}",
            spec.0
        );
    }
    for (resource, spec) in RESOURCE_SPECS.iter().enumerate() {
        // Owner face: the owner arm admits every LIVE fixture whose owner is P_OWNER.
        // r_owned_anywhere is deliberately dana's (the owner-anywhere arm), so `owner` does not
        // see it; r_tomb is dead, so even its owner does not.
        let expected = resource != R_TOMB && spec.2 == P_OWNER;
        assert_eq!(
            sees(P_OWNER, resource).await?,
            expected,
            "owner arm broken on fixture {:?}",
            spec.0
        );
    }

    Ok(())
}

// =================================================================================================
// The harness proper — one world, all invariant families, seed printed on failure.
// =================================================================================================

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn invariants_hold_over_a_generated_world(pool: PgPool) -> sqlx::Result<()> {
    let seed = harness_seed();
    let plan_a = Plan::generate(seed);
    let plan_b = Plan::generate(seed);
    assert_eq!(
        plan_a, plan_b,
        "the plan generator is not reproducible for seed {seed} — same seed, different world"
    );

    let w = insert_world(&pool, &plan_a).await?;

    let mut violations = Vec::new();
    violations.extend(i1_read_duality(&pool, &w).await?);
    violations.extend(i2_write_implies_read(&pool, &w).await?);
    violations.extend(i3_deny_is_absence(&pool, &w).await?);
    violations.extend(i4_tombstone_floors(&pool, &w).await?);
    violations.extend(i5_degrees_over_visible_edges_only(&pool, &w).await?);
    violations.extend(i6_cogmap_intersection(&pool, &w).await?);

    assert!(
        violations.is_empty(),
        "{} visibility invariant violation(s) over seed {}:\n  - {}\n\n{}",
        violations.len(),
        seed,
        violations.join("\n  - "),
        w.render()
    );

    Ok(())
}

/// The bite: a harness whose worlds only re-exercise the canonical predicate witnesses nothing.
/// So — mutate ONE search CTE's visibility filter (the `resources_visible_to` hand-off at
/// `query_find_exact`'s boundary), and require the search invariant to go RED, then green again
/// after restore. This is the standing proof that the harness bites a RESTATEMENT, not just the
/// canonical gates.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_harness_bites_a_search_cte_restatement(pool: PgPool) -> sqlx::Result<()> {
    let plan = Plan::generate(harness_seed());
    let w = insert_world(&pool, &plan).await?;

    let pre = search_slice(&pool, &w).await?;
    assert!(
        pre.is_empty(),
        "precondition failed: the search invariant must hold before the mutation, or the bite \
         proves nothing: {pre:?}"
    );

    let original: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef(p.oid) FROM pg_proc p \
           JOIN pg_namespace n ON n.oid = p.pronamespace \
          WHERE n.nspname = 'public' AND p.prokind = 'f' AND p.proname = 'query_find_exact'",
    )
    .fetch_one(&pool)
    .await?;

    // THE RESTATEMENT: widen the boundary set from "visible to the principal" to "every live
    // resource" — exactly the drift a perf-motivated rewrite would introduce.
    const BOUNDARY: &str = "ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v)";
    const WIDENED: &str = "ARRAY(SELECT r.id FROM kb_resources r)";
    let mutated = original.replace(BOUNDARY, WIDENED);
    assert_ne!(
        mutated, original,
        "the boundary expression was not found in query_find_exact's body — the function changed \
         shape and this bite must be re-aimed"
    );

    sqlx::query(&mutated).execute(&pool).await?;
    let bitten = search_slice(&pool, &w).await?;
    sqlx::query(&original).execute(&pool).await?; // restore before asserting

    assert!(
        !bitten.is_empty(),
        "widening query_find_exact's boundary set did NOT turn the search invariant red — the \
         harness cannot catch the drift class it exists for"
    );
    assert!(
        bitten.iter().any(|v| v.contains("I3.search")),
        "the mutation was caught, but not by the search invariant: {bitten:?}"
    );

    let restored = search_slice(&pool, &w).await?;
    assert!(
        restored.is_empty(),
        "after restore the search invariant did not come back green: {restored:?}"
    );

    Ok(())
}
