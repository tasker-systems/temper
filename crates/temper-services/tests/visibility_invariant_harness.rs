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
// Tombstoned below — confers nothing.
const T_GHOST: usize = 5;
// public_within's T: a NEW team under eng, opened to eng's subtree by the one public_within row
// this world holds. Deliberately not `sec`: opening (sec, eng) would break dana's sibling-deny
// control and hollow the r_owned_anywhere / r_ancestor_grant witnesses.
const T_PUB: usize = 6;
const TEAM_SLUGS: [&str; 7] = ["epd", "eng", "payroll", "squad", "sec", "ghost", "pub"];
const TEAM_PARENTS: [(usize, usize); 6] = [
    (T_EPD, T_ENG),
    (T_ENG, T_PAYROLL),
    (T_PAYROLL, T_SQUAD),
    (T_ENG, T_SEC),
    (T_ENG, T_GHOST),
    (T_ENG, T_PUB),
];

// Profiles, by index: (handle, forced (team, role) memberships).
const P_DANA: usize = 0;
const P_HENRY: usize = 1;
const P_FRANK: usize = 2;
const P_GINA: usize = 3;
const P_OWNER: usize = 4;
const P_OUTSIDER: usize = 5;
// The public_within controls: `ella` is a person on epd, ABOVE the anchor and so outside
// its subtree; `robot` is a machine principal on squad, inside the subtree. Neither may read pub's
// context. Both are kept out of the randomized matrix, like `owner` and `outsider`.
const P_ELLA: usize = 6;
const P_ROBOT: usize = 7;
const PROFILE_HANDLES: [&str; 8] = [
    "dana", "henry", "frank", "gina", "owner", "outsider", "ella", "robot",
];
// `owner` and `outsider` stay teamless — the negative controls the precedent file demands.
const FORCED_MEMBERSHIPS: [(usize, usize, &str); 6] = [
    (P_DANA, T_SQUAD, "member"),
    (P_HENRY, T_GHOST, "member"), // only reach is through the tombstoned team
    (P_FRANK, T_SEC, "watcher"),  // read-only role
    (P_GINA, T_ENG, "maintainer"),
    (P_ELLA, T_EPD, "member"),    // above the anchor: outside its subtree
    (P_ROBOT, T_SQUAD, "member"), // a machine inside the subtree
];

// Contexts: (slug, owner kind, owner index, shared-to team or NONE, alive).
const C_ENG: usize = 0;
const C_SEC: usize = 1;
const C_SHARED: usize = 2;
const C_PERSONAL: usize = 3;
const C_TOMB: usize = 4; // tombstoned below
const C_PUB: usize = 5; // owned by T_PUB — readable by people under eng through public_within only
const CONTEXT_SPECS: [(&str, &str, usize, Option<usize>); 6] = [
    ("ctx-eng", "kb_teams", T_ENG, None),
    ("ctx-sec", "kb_teams", T_SEC, None),
    ("ctx-shared", "kb_teams", T_ENG, Some(T_SQUAD)),
    ("ctx-personal", "kb_profiles", P_OWNER, None),
    ("ctx-tomb", "kb_teams", T_ENG, None),
    ("ctx-pub", "kb_teams", T_PUB, None),
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
const R_PUB: usize = 12; // homed in ctx-pub — the public_within arm's witness
const RESOURCE_SPECS: [(&str, Anchor, usize); 13] = [
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
    ("r_pub", Anchor::Context(C_PUB), P_OWNER),
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

// Write grants that keep two modify arms live on every seed: the context-grant and cogmap-grant
// arms of can_modify_resource's container cascade, which the randomized matrix does not reliably
// reach. They are applied AFTER the randomized matrix (see Plan::generate), replacing any drawn
// grant on the same pair, so the generator's stream, and every other grant an older seed draws,
// is unchanged by them.
const MODIFY_ARM_GRANTS: [GrantSpec; 2] = [
    // henry's only team is the tombstoned ghost, which confers nothing, so this grant is his only
    // way to author or to read ctx-shared: both sides of modify⇒visible run through the grant.
    GrantSpec {
        subject: Subject::Context(C_SHARED),
        principal: Principal::Profile(P_HENRY),
        can_read: true,
        can_write: true,
        can_delete: false,
        can_grant: false,
    },
    // A cogmap outside the admin-only regime and joined to no team, so with the resource's own
    // grants stripped (the arm witness) gina reads and authors it through this grant alone. gina,
    // not dana, so dana's read-only grant stays the cogmap READ witness.
    GrantSpec {
        subject: Subject::Cogmap(M_GRANT),
        principal: Principal::Profile(P_GINA),
        can_read: true,
        can_write: true,
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
            .filter(|&p| p != P_OWNER && p != P_OUTSIDER && p != P_ELLA && p != P_ROBOT)
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
            //
            // r_pub and ctx-pub are grant-immune for the same reason: the public_within controls
            // (ella above the anchor, robot the machine) must read nothing of them on every seed.
            // Skipped before any draw, so the generator's stream, and every older seed's world, is
            // unchanged by their arrival.
            if matches!(
                subject,
                Subject::Resource(R_CTX_SEC)
                    | Subject::Context(C_SEC)
                    | Subject::Resource(R_PERSONAL)
                    | Subject::Context(C_PERSONAL)
                    | Subject::Resource(R_PUB)
                    | Subject::Context(C_PUB)
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

        // The modify-arm grants, last, after every draw: each replaces any drawn grant on its pair.
        for fixed in MODIFY_ARM_GRANTS {
            grants.retain(|g| (g.subject, g.principal) != (fixed.subject, fixed.principal));
            grants.push(fixed);
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
    // Every profile but the machine is a PERSON: a human auth link and no machine-client row, which
    // is what public_within_reading asks. The machine carries the machine provider's link and a
    // registration, as prod does.
    for (i, id) in profiles.iter().enumerate() {
        let provider = if i == P_ROBOT {
            "auth0-m2m"
        } else {
            "harness-idp"
        };
        sqlx::query(
            "INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id) \
             VALUES (uuid_generate_v7(), $1, $2, $3)",
        )
        .bind(id)
        .bind(provider)
        .bind(PROFILE_HANDLES[i])
        .execute(pool)
        .await?;
    }
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ('robot', 'robot', $1, $2)",
    )
    .bind(profiles[P_ROBOT])
    .bind(profiles[P_OWNER])
    .execute(pool)
    .await?;

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

    // The one public_within row: pub opened to eng's subtree. Written directly; no door that
    // opens a row exists yet.
    sqlx::query(
        "INSERT INTO kb_team_public_within (team_id, anchor_id, opened_by) VALUES ($1, $2, $3)",
    )
    .bind(teams[T_PUB])
    .bind(teams[T_ENG])
    .bind(profiles[P_GINA])
    .execute(pool)
    .await?;

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

    // Fixture blobs: one per generated CONTEXT anchor, minted once. The blob invariant
    // below claims the blob contributes NO visibility of its own — `blob_readable_by_profile`
    // is exactly its home anchor's answer (the row IS its home, D2 as amended) — so the
    // fixture needs no blob-specific world content, only rows that exist. The event-id FKs
    // are satisfied by the world's own events; the per-home unique key makes re-minting a
    // no-op. The conflict target carries the partial index's WHERE clause
    // (20260906000010): uniqueness binds LIVE rows only, so the arbiter must be inferred
    // on the same predicate.
    //
    // Contexts only, deliberately: the blob-home exclusion (20260911000020) makes a
    // cogmap-homed row unwritable — `kb_blobs_home_context_only` — so the gate's cogmap
    // arm is dead vocabulary, kept as defense-in-depth, with no legal fixture for it. The
    // duality stays fully exercised over the reachable world.
    let fixture_owner = w.profiles[0];
    let fixture_event: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_events ORDER BY occurred_at, id LIMIT 1")
            .fetch_one(pool)
            .await?;
    sqlx::query(
        "INSERT INTO kb_blobs (id, content_hash, blob_pathname, content_type, content_bytes, \
                              home_table, home_id, owner_profile_id, originator_profile_id, \
                              asserted_by_event_id, last_event_id) \
         SELECT gen_random_uuid(), 'harness-' || a.id, 'ha/' || a.id, \
                'application/octet-stream', 0, 'kb_contexts', a.id, $1, $1, $2, $2 \
           FROM unnest($3::uuid[]) AS a(id) \
         ON CONFLICT (home_table, home_id, content_hash) WHERE content_type IS NOT NULL \
         DO NOTHING",
    )
    .bind(fixture_owner)
    .bind(fixture_event)
    .bind(&w.contexts)
    .execute(pool)
    .await?;

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

        // blobs: the blob answer IS the home anchor's answer — `blob_readable_by_profile`
        // routes through `anchor_readable_by_profile` on the row's own home columns (D2 as
        // amended), so any divergence means the blob gate grew a rule the anchor gate does
        // not have, or lost one it must inherit.
        let blob_rows: Vec<(Uuid, String, Uuid)> =
            sqlx::query_as("SELECT id, home_table, home_id FROM kb_blobs ORDER BY id")
                .fetch_all(pool)
                .await?;
        let blob_ids: Vec<Uuid> = blob_rows.iter().map(|(id, _, _)| *id).collect();
        let blob_answers = point_answers(
            pool,
            "blob_readable_by_profile($1::uuid, c.id)",
            profile,
            &blob_ids,
        )
        .await?;
        for ((blob_id, blob_answer), (_, home_table, home_id)) in
            blob_answers.iter().zip(&blob_rows)
        {
            let anchor_answer: bool =
                sqlx::query_scalar("SELECT anchor_readable_by_profile($1, $2, $3)")
                    .bind(profile)
                    .bind(home_table)
                    .bind(home_id)
                    .fetch_one(pool)
                    .await?;
            check_duality(
                &mut violations,
                "I1.blobs",
                &format!("blob_readable_by_profile vs anchor_readable(home)[{who}]"),
                "blob",
                *blob_id,
                anchor_answer,
                *blob_answer,
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

    // The exclusion's premise (see modify⇒visible below): no world profile is a system admin.
    let admins: Vec<Uuid> = sqlx::query_scalar(
        "SELECT p FROM unnest($1::uuid[]) AS p WHERE is_system_admin(p) ORDER BY 1",
    )
    .bind(&w.profiles)
    .fetch_all(pool)
    .await?;
    if !admins.is_empty() {
        violations.push(format!(
            "I2: the world holds system admins {admins:?}; modify⇒visible does not yet hold for \
             an admin on admin-only-regime maps, so exclude that case explicitly before adding one"
        ));
    }

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

        // modify⇒visible: can_modify_resource ⇒ membership in resources_visible_to, the set the
        // resource read door (GET /api/resources/{id}) answers from. Today this follows from I1
        // (can(read) ⇔ resources_visible_to) and the two I2 checks above (can(write) ⇒ can(read),
        // can(write) ⇔ can_modify_resource). It is asserted directly so that a change breaking
        // any link in that chain fails here, by name: a write refusal that echoes current values
        // is only safe if no one can modify what they cannot read.
        //
        // Excluded case, not reachable from this world: a map in the admin-only regime (the L0
        // kernel, or a map bound to temper-system or the gating team) is authored by a system
        // admin alone, and resources_visible_to has no admin arm, so an admin who cannot otherwise
        // read such a map's nodes could still modify them. That gap is known and tracked to be
        // closed separately. This world holds no system admin, binds no map to an instance team
        // and homes nothing in the kernel, so the case cannot arise. No system admin is enough on
        // its own to rule it out, and that is checked above, so a world that grows an admin must
        // decide the exclusion here instead of failing on it by surprise.
        let visible = scalar_set(
            pool,
            "SELECT resource_id FROM resources_visible_to($1) ORDER BY 1",
            profile,
        )
        .await?;
        for (id, modify) in &can_modify {
            if *modify && !visible.contains(id) {
                violations.push(format!(
                    "I2: can_modify_resource without resources_visible_to — profile {who}, \
                     resource {id}"
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

        // the list door: the selection's set, returned — unfiltered and unbounded, the same set.
        let listed: Vec<Uuid> = sqlx::query_scalar(
            "SELECT resource_id FROM query_list_resources_with($1, NULL::text[], NULL::text[], \
             NULL::jsonb, NULL::text, NULL::text, NULL::uuid, NULL::text, NULL::text, \
             NULL::jsonb, NULL::varchar, NULL::uuid, NULL::jsonb, NULL::uuid[]) ORDER BY 1",
        )
        .bind(profile)
        .fetch_all(pool)
        .await?;
        let leaked: Vec<_> = listed.iter().filter(|id| !visible.contains(id)).collect();
        if !leaked.is_empty() {
            violations.push(format!(
                "I3.search: query_list_resources_with[{who}] leaked unreadable resources \
                 {leaked:?}"
            ));
        }
    }

    Ok(violations)
}

/// **A list stage's `total` counts only what the caller can see.** `total` is the one datum the
/// list act adds that discloses something without carrying an id, so it is held here beside the
/// rows: for every profile — each of whom has matching rows in the world it cannot read — the
/// compiled statement's `total` must equal the number of ids an unpaged walk of the gated list
/// returns, and the `produced_ids` a `find-resources-with` stage with the same arguments reports
/// in the same trace. A count lifted above the core's `JOIN unnest(p_visible_ids)` — over the
/// bound, or over an unfiltered set — reads the invisible rows and fails both.
async fn list_total_slice(pool: &PgPool, w: &World) -> Vec<Violation> {
    use temper_core::types::artifact_ask::ShapeContext;
    use temper_core::types::query::{
        validate, ActInvocation, ActName, BoundTerm, Composition, OutcomeDeclaration, ReturnSpec,
        StageName, StageNode,
    };

    use temper_core::types::query::{IdKind, IdSet, StageInput, StageRelation};

    let stage = |name: &str, act: ActName| {
        StageNode::Act(ActInvocation {
            name: StageName::parse(name).unwrap(),
            act,
            intention: None,
            inputs: vec![],
            terms: if name == "rows" {
                [(BoundTerm::Limit, 1)].into_iter().collect()
            } else {
                Default::default()
            },
            order: None,
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
            artifact_filter: vec![],
        })
    };
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: StageName::parse("rows").unwrap(),
                with: vec![],
            }],
        },
        stages: vec![
            stage("rows", ActName::ListResourcesWith),
            stage("sel", ActName::FindResourcesWith),
        ],
    };
    let v = validate(&c, &mut ShapeContext::empty().session()).expect("the plan is valid");

    let mut violations = Vec::new();
    let mut hid_some = false;
    for (p_idx, &profile) in w.profiles.iter().enumerate() {
        let who = PROFILE_HANDLES[p_idx];
        let walked: Vec<Uuid> = match sqlx::query_scalar(
            "SELECT resource_id FROM query_list_resources_with($1, NULL::text[], NULL::text[], \
             NULL::jsonb, NULL::text, NULL::text, NULL::uuid, NULL::text, NULL::text, \
             NULL::jsonb, NULL::varchar, NULL::uuid, NULL::jsonb, NULL::uuid[])",
        )
        .bind(profile)
        .fetch_all(pool)
        .await
        {
            Ok(ids) => ids,
            Err(e) => {
                violations.push(format!("I3.total: the gated list failed for {who}: {e}"));
                continue;
            }
        };
        hid_some |= w.resources.iter().any(|r| !walked.contains(r));
        let r = match temper_services::backend::query_read::run_composition(
            pool,
            temper_core::types::ids::ProfileId::from(profile),
            &v,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                violations.push(format!("I3.total: the list plan failed for {who}: {e:?}"));
                continue;
            }
        };
        let total = r.returned[&StageName::parse("rows").unwrap()].total;
        let selected = r
            .trace
            .stages
            .iter()
            .find(|t| t.stage.as_str() == "sel")
            .map(|t| t.produced_ids);
        if total != Some(walked.len() as i64) || total != selected {
            violations.push(format!(
                "I3.total: list total[{who}] = {total:?}, but an unpaged walk returns {} and the \
                 selection produced {selected:?} — the count reads rows the caller cannot see",
                walked.len()
            ));
        }
    }

    // **Through a caller's BOUND, which is where an existence oracle would live.** A list bounded
    // by every resource in the world — readable or not — must count exactly the readable ones, and
    // report the rest as `input_unusable`. And a bound of ONE unreadable id must answer exactly as a
    // bound of one id that names nothing: `total: 0`, one unusable. A count taken over the bound
    // rather than over the core's verdict would answer 1 for the first and 0 for the second.
    let bounded = |ids: Vec<Uuid>| {
        let rows = StageName::parse("rows").unwrap();
        let c = Composition {
            outcome: OutcomeDeclaration {
                returns: vec![ReturnSpec {
                    stage: rows.clone(),
                    with: vec![],
                }],
            },
            stages: vec![StageNode::Act(ActInvocation {
                name: rows,
                act: ActName::ListResourcesWith,
                intention: None,
                inputs: vec![StageInput::Caller {
                    relation: StageRelation::Bound,
                    ids: IdSet {
                        kind: IdKind::Resource,
                        provenance: None,
                        ids,
                    },
                }],
                terms: [(BoundTerm::Limit, 1)].into_iter().collect(),
                order: None,
                resource_filter: None,
                edge_filter: None,
                properties: vec![],
                artifact_filter: vec![],
            })],
        };
        validate(&c, &mut ShapeContext::empty().session()).expect("the plan is valid")
    };
    let mut probed_unreadable = false;
    for (p_idx, &profile) in w.profiles.iter().enumerate() {
        let who = PROFILE_HANDLES[p_idx];
        let principal = temper_core::types::ids::ProfileId::from(profile);
        let visible: Vec<Uuid> = match sqlx::query_scalar(
            "SELECT resource_id FROM query_list_resources_with($1, NULL::text[], NULL::text[], \
             NULL::jsonb, NULL::text, NULL::text, NULL::uuid, NULL::text, NULL::text, \
             NULL::jsonb, NULL::varchar, NULL::uuid, NULL::jsonb, NULL::uuid[])",
        )
        .bind(profile)
        .fetch_all(pool)
        .await
        {
            Ok(ids) => ids,
            Err(e) => {
                violations.push(format!("I3.total: the gated list failed for {who}: {e}"));
                continue;
            }
        };
        let answer = |ids: Vec<Uuid>| {
            let v = bounded(ids);
            async move {
                let r = temper_services::backend::query_read::run_composition(pool, principal, &v)
                    .await
                    .map_err(|e| format!("{e:?}"))?;
                let s = &r.returned[&StageName::parse("rows").unwrap()];
                Ok::<_, String>((s.total, s.input_unusable))
            }
        };
        let readable = w.resources.iter().filter(|r| visible.contains(r)).count() as i64;
        let unreadable: Vec<Uuid> = w
            .resources
            .iter()
            .filter(|r| !visible.contains(r))
            .copied()
            .collect();
        match answer(w.resources.clone()).await {
            Ok(got) if got == (Some(readable), unreadable.len() as i64) => {}
            Ok(got) => violations.push(format!(
                "I3.total: list bounded by the whole world[{who}] answered (total, unusable) = \
                 {got:?}; the readable count is {readable} and {} are unreadable",
                unreadable.len()
            )),
            Err(e) => violations.push(format!("I3.total: bounded list failed for {who}: {e}")),
        }
        if let Some(&hidden) = unreadable.first() {
            probed_unreadable = true;
            let hidden_answer = answer(vec![hidden]).await;
            let absent_answer = answer(vec![Uuid::now_v7()]).await;
            if hidden_answer != absent_answer || hidden_answer != Ok((Some(0), 1)) {
                violations.push(format!(
                    "I3.total: a bound of one unreadable id[{who}] answered {hidden_answer:?} and \
                     one naming nothing answered {absent_answer:?} — an existence oracle"
                ));
            }
        }
    }
    if !probed_unreadable {
        violations.push(
            "I3.total: no profile had an unreadable resource to bound by, so the oracle probe is \
             vacuous"
                .to_string(),
        );
    }
    if !hid_some {
        violations.push(
            "I3.total: no profile had an invisible resource, so the count witness is vacuous"
                .to_string(),
        );
    }
    violations
}

async fn i3_deny_is_absence(pool: &PgPool, w: &World) -> sqlx::Result<Vec<Violation>> {
    let mut violations = search_slice(pool, w).await?;
    violations.extend(list_total_slice(pool, w).await);

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
// Invariant family 7 — for an erased resource, `resource_husk_held_by` EQUALS the reach of three
// of `resources_visible_to`'s arms (owner-home, direct-profile-grant, team-grant) with the
// `is_active` floor ignored (spec D6): a dropped arm and an added one both go red. Against the
// full `resources_visible_to` it is only a subset, because the context-homed and both cogmap
// arms are excluded BY DESIGN. For a resource that is not erased the answer is false.
//
// The expected three-arm reach is derived in Rust from the world plan (owner index, the grant
// matrix, `profile_reachable_teams` for the team closure), never by calling the function under
// test. VACUITY: the main generated world erases nothing, so this invariant is driven by
// [`the_husk_predicate_equals_the_three_arm_reach_over_erased_resources`], which erases
// resources through the REAL act first.
// =================================================================================================

async fn i7_husk_held_by_equals_the_three_arm_reach(
    pool: &PgPool,
    w: &World,
) -> sqlx::Result<Vec<Violation>> {
    let mut violations = Vec::new();

    for (r_idx, &resource) in w.resources.iter().enumerate() {
        let erased: bool =
            sqlx::query_scalar("SELECT erased_at IS NOT NULL FROM kb_resources WHERE id = $1")
                .bind(resource)
                .fetch_one(pool)
                .await?;
        let name = RESOURCE_SPECS[r_idx].0;

        for (p_idx, &profile) in w.profiles.iter().enumerate() {
            let who = PROFILE_HANDLES[p_idx];
            let held: bool = sqlx::query_scalar("SELECT resource_husk_held_by($1, $2)")
                .bind(profile)
                .bind(resource)
                .fetch_one(pool)
                .await?;

            if !erased {
                if held {
                    violations.push(format!(
                        "I7: resource_husk_held_by is true for {who} on {name}, which is not erased"
                    ));
                }
                continue;
            }

            let reachable: Vec<Uuid> =
                sqlx::query_scalar("SELECT team_id FROM profile_reachable_teams($1)")
                    .bind(profile)
                    .fetch_all(pool)
                    .await?;
            let owner_arm = RESOURCE_SPECS[r_idx].2 == p_idx;
            let grant_arm = w.plan.grants.iter().any(|g| {
                g.can_read
                    && g.subject == Subject::Resource(r_idx)
                    && match g.principal {
                        Principal::Profile(p) => p == p_idx,
                        Principal::Team(t) => reachable.contains(&w.teams[t]),
                    }
            });
            let expected = owner_arm || grant_arm;
            if held != expected {
                violations.push(format!(
                    "I7: resource_husk_held_by({who}, {name}) = {held}, but the owner / \
                     profile-grant / team-grant arms say {expected} (owner {owner_arm}, grant \
                     {grant_arm})"
                ));
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
    // resource_husk_held_by: equal to 3 of resources_visible_to's arms (minus the is_active
    // floor, erased resources only), a strict subset of its full reach — driven by I7.
    "resource_husk_held_by",
    "can",
    "can_modify_resource",
    "cogmap_readable_by_profile",
    "cogmap_authorable_by_profile",
    "context_readable_by_profile",
    "context_visible_to",
    "context_authorable_by_profile",
    "anchor_readable_by_profile",
    "endpoint_readable_by_profile",
    "blob_readable_by_profile",
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
    // query_list_resources_with — the selection core's set returned, narrowed by a bound; driven by
    // `search_slice` (deny-is-absence) and `list_total_slice` (its `total` counts only visible rows).
    "query_list_resources_with",
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
    // resources_changed_by / contexts_changed_by: exactly the profile's visible set, over a world
    // in which every profile changed everything — driven by
    // `the_change_readers_answer_exactly_their_visible_sets`.
    "resources_changed_by",
    "contexts_changed_by",
    // context_changes_after (the change feed, 20261116000010): an answer exactly for the contexts
    // in contexts_readable_by, each serving its own change — driven by
    // `the_change_feed_answers_exactly_the_readable_contexts`. That gate is the whole gate for
    // resources, blocks and blobs: an entry is a pointer, and its subject's own read answers under
    // the reader's standing. An edge subject is served only while the reader can read both its
    // endpoints (20261117000020), witnessed in temper-substrate's context_change_feed.rs.
    "context_changes_after",
    // context_cursor (the context cursor, 20261117000040): non-NULL exactly for the contexts in
    // contexts_readable_by that hold a change the feed serves, NULL for every other — driven by the
    // same `the_change_feed_answers_exactly_the_readable_contexts`. Its other predicates are the
    // feed's; temper-substrate's context_change_feed.rs pins the two to the same answer.
    "context_cursor",
    // search doors (wrappers over the gated query_* machines)
    "search_exact",
    "search_wide",
    "search_graph_expand",
    // public_within. The read-set BODIES, which the two set authorities
    // above are now body-only wrappers over, and the membership entry points beside them: driven
    // by `public_within_reads_for_people_under_the_anchor_and_nobody_else`, which holds reading =
    // membership + the public_within arm for every profile in the world, membership as reading
    // minus that arm, and the wrappers equal to their bodies.
    "_contexts_readable_by_teams",
    "_resources_visible",
    "resources_visible_by_membership",
    "contexts_readable_by_membership",
    "can_read_by_membership",
    // the public_within delta the audit queue and the element trail derive reading from: held
    // equal to reading minus membership by the same test.
    "resources_public_within",
    // the public_within arm and the predicates it is built from — driven by the same test, each
    // at both answers: over the world's one row, under a pool and under a forbid.
    "contexts_public_within",
    "public_within_row_effective",
    "public_within_allowed_over",
    "public_within_reading",
    "anchor_subtree_holds_pool",
    // the single-object reach the context and resource gets answer: driven by the same test, which
    // asks each of every object every profile reads, with and without the forbid, and holds the
    // answer to the two sets (public_within exactly outside membership).
    "context_reached_via",
    "resource_reached_via",
];

/// The substrate the substrate-existence half pins; excluded from the scan because these are the
/// PRIMITIVES every answer routes through, not answers themselves.
const PRIMITIVES: &[&str] = &[
    "team_ancestors",
    "profile_effective_teams",
    "vis_team",
    // The DAG walks public_within's predicates are built from: the live down-walk and the
    // live-or-dead up-walk the forbid uses. Structure, never an answer about who reads what.
    "team_live_descendants",
    "team_ancestors_any",
];

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
    // artifact_ask_counts / artifacts_matching_ask — the batch read of an artifact ask
    // (20261106000000). Each builds resources_visible_to(p_principal) once, COALESCEd to an empty
    // array, and hands it with the principal to the ungated counted-set body, which applies every
    // other conjunct (the anchor_readable_by_profile home re-check among them); neither restates a
    // rule. Their deny direction, conjunct by conjunct, is temper-substrate's
    // artifact_ask_batch_read.rs.
    "artifact_ask_counts",
    "artifact_by_id",
    "artifacts_matching_ask",
    "audit_drift_sweep",
    "cogmap_analytics",
    // cogmap_regulation — cogmap_analytics' regulation rows (20261119000030). Its profile arm
    // gates each telos edge on edges_visible_to and its target on resources_readable_by; the map
    // principal's arm reads resources_accessible_to_cogmap. It restates no rule. The edge
    // conjunction, both arms, is pinned by edge_conjunction_harness.rs
    // (cogmap_regulation_admits_only_visible_edges).
    "cogmap_regulation",
    // _resource_contradiction_balance — the evidence read's per-reader balance (20261119000040).
    // Its profile arm counts only edges in edges_visible_to; the map principal's arm reads
    // resources_accessible_to_cogmap; the context principal's arm reads
    // resources_readable_by('context', ..). It restates no rule; the conjunction is pinned by
    // edge_conjunction_harness.rs (standing_scores_only_edges_the_reader_sees).
    "_resource_contradiction_balance",
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
    // principal_erasure_execute — the scan hit is a body COMMENT, not a call: the strike arm
    // spells its derivation as "the read model's own owner arm (contexts_readable_by arm 1,
    // 20260712000010)" and then restates the governed-home predicate inline. It consults no
    // substrate authority — erasure's scope is the two indexed halves filtered to governed
    // personal contexts BY RULING (20260909000025) — so there is no authority here to drive.
    // Acknowledged explicitly rather than silently excluded. (Landed with 20260909000025;
    // the enrollment gate caught it at Beat 4's full-suite run.)
    "principal_erasure_execute",
    // _erasure_apply_redaction — the same ruling as the strike above, in the execution arm: the
    // scan hit is the SAME body COMMENT ("contexts_readable_by arm 1, 20260712000010") in the
    // recompute-marking arm, not a call. The one redaction definition consults no substrate
    // authority — its arms are hash-keyed content empties, the profile tombstone, the external
    // identifier deletes, and the recompute marks whose context arm restates the governed
    // personal-contexts predicate inline BY RULING (20260909000025). Acknowledged explicitly
    // rather than silently excluded, on the principal_erasure_execute precedent.
    "_erasure_apply_redaction",
    // principal_erasure_survey_plan — the erasure act's own scope/outcomes computation (20260913000010
    // moved it out of principal_erasure_execute so the read-only survey door and the act run ONE
    // computation), so this is the SAME hit as the execute entry above: the scan matches the body
    // COMMENT ("contexts_readable_by arm 1, 20260712000010") carried over with the governed-home
    // arms, not a call. It consults no substrate authority — the scope is the two indexed halves
    // filtered to governed personal contexts BY RULING (the 2026-09-11 scope-of-engagement ruling,
    // home-pure), byte-identical to the act's own restatement. Acknowledged explicitly rather than
    // silently excluded, on the principal_erasure_execute precedent.
    "principal_erasure_survey_plan",
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
        sees(P_DANA, R_PUB).await?,
        "arm broken: dana cannot see r_pub via public_within: a context owned by a team opened \
         to the subtree of an ancestor dana is under"
    );
    assert!(
        !sees(P_DANA, R_CTX_SEC).await?,
        "negative control broken: dana sees a sibling team's context resource"
    );
    assert!(
        !sees(P_ELLA, R_PUB).await?,
        "negative control broken: ella, above the anchor and so outside its subtree, reads r_pub"
    );
    assert!(
        !sees(P_ROBOT, R_PUB).await?,
        "negative control broken: a machine principal under the anchor reads r_pub through \
         public_within"
    );
    assert!(
        !sees(P_DANA, R_PERSONAL).await?,
        "negative control broken: dana sees into the owner's personal context"
    );

    // Modify-arm witnesses, seed-independent: the owner home, a team-owned context authored by
    // direct membership, a context write grant and a cogmap write grant, the last three also
    // through can_modify_resource's container cascade. Each runs in a rolled-back transaction that
    // first deletes every grant on the resource and its container except the witness's own fixed
    // grant (`keep`), so no randomized grant answers for the arm; it then asserts the gate admits
    // AND that the subject stays readable, so modify⇒visible is witnessed through the arm alone.
    // These pin admission, not refusal: the resource-level profile and team write-grant arms, and
    // any widening of a gate past its fixtures, are reached only through the randomized matrix.
    let none = Uuid::nil();
    let (r_ctx_eng, r_shared, r_map_grant) = (
        w.resources[R_CTX_ENG],
        w.resources[R_SHARED],
        w.resources[R_MAP_GRANT],
    );
    let (c_eng, c_shared, m_grant) = (w.contexts[C_ENG], w.contexts[C_SHARED], w.cogmaps[M_GRANT]);
    const READ_RESOURCE: &str =
        "EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)";
    const READ_CONTEXT: &str = "can('kb_profiles', $1, 'read', 'kb_contexts', $2)";
    const READ_COGMAP: &str = "can('kb_profiles', $1, 'read', 'kb_cogmaps', $2)";
    for (gate, read, who, subject, strip, keep, arm) in [
        (
            "can_modify_resource",
            READ_RESOURCE,
            P_OWNER,
            r_ctx_eng,
            [r_ctx_eng, c_eng],
            none,
            "owner home",
        ),
        (
            "context_authorable_by_profile",
            READ_CONTEXT,
            P_GINA,
            c_eng,
            [c_eng, c_eng],
            none,
            "team-owned context, maintainer",
        ),
        (
            "can_modify_resource",
            READ_RESOURCE,
            P_GINA,
            r_ctx_eng,
            [r_ctx_eng, c_eng],
            none,
            "team-owned context, cascaded",
        ),
        (
            "context_authorable_by_profile",
            READ_CONTEXT,
            P_HENRY,
            c_shared,
            [c_shared, c_shared],
            c_shared,
            "context write grant",
        ),
        (
            "can_modify_resource",
            READ_RESOURCE,
            P_HENRY,
            r_shared,
            [r_shared, c_shared],
            c_shared,
            "context write grant, cascaded",
        ),
        (
            "cogmap_authorable_by_profile",
            READ_COGMAP,
            P_GINA,
            m_grant,
            [m_grant, m_grant],
            m_grant,
            "cogmap write grant",
        ),
        (
            "can_modify_resource",
            READ_RESOURCE,
            P_GINA,
            r_map_grant,
            [r_map_grant, m_grant],
            m_grant,
            "cogmap write grant, cascaded",
        ),
    ] {
        let mut tx = pool.begin().await?;
        sqlx::query(
            "DELETE FROM kb_access_grants WHERE subject_id = ANY($1) \
               AND NOT (subject_id = $2 AND principal_table = 'kb_profiles' AND principal_id = $3)",
        )
        .bind(&strip[..])
        .bind(keep)
        .bind(w.profiles[who])
        .execute(&mut *tx)
        .await?;
        let (admitted, reads): (bool, bool) =
            sqlx::query_as(&format!("SELECT {gate}($1, $2), {read}"))
                .bind(w.profiles[who])
                .bind(subject)
                .fetch_one(&mut *tx)
                .await?;
        tx.rollback().await?;
        assert!(
            admitted,
            "modify arm broken: {gate} refuses {} via {arm}",
            PROFILE_HANDLES[who]
        );
        assert!(
            reads,
            "modify⇒visible broken: {} may modify via {arm} but cannot read the subject",
            PROFILE_HANDLES[who]
        );
    }

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

/// public_within's arm and its fences, over every profile in the world.
///
/// - **Layering.** Each wrapper equals its body with the arm on: `resources_visible_to(p)` is
///   `_resources_visible(p, true)`, `resources_visible_by_membership(p)` is
///   `_resources_visible(p, false)`, and the context pair likewise.
/// - **Reading = membership + the public_within arm.** Membership ⊆ reading, and every resource
///   in reading but not membership is homed in a context `contexts_public_within` opens to that
///   profile's closure.
///   Only a person (`public_within_reading`) has any difference at all.
/// - **The witnesses.** dana (a person under eng) reads r_pub and ctx-pub, but not by membership,
///   so `can_read_by_membership` refuses them where `can(read)` admits. ella (above the anchor)
///   and robot (a machine under it) read neither. The row's predicates answer as the world says.
/// - **The forbid.** With a forbid at the top of the world's tree, the row confers nothing and
///   reading equals membership for everyone.
/// - **The delta.** Reading is exactly membership UNION `resources_public_within`, which is what
///   lets the audit queue and the element trail derive one from the other without a second scan.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn public_within_reads_for_people_under_the_anchor_and_nobody_else(
    pool: PgPool,
) -> sqlx::Result<()> {
    let plan = Plan::generate(harness_seed());
    let w = insert_world(&pool, &plan).await?;
    let mut violations: Vec<Violation> = Vec::new();

    let set = |sql: &'static str, p: Uuid| {
        let pool = pool.clone();
        async move {
            let mut v: Vec<Uuid> = sqlx::query_scalar(sql).bind(p).fetch_all(&pool).await?;
            v.sort();
            v.dedup();
            Ok::<_, sqlx::Error>(v)
        }
    };

    for forbidden in [false, true] {
        sqlx::query("UPDATE kb_teams SET public_within_forbidden = $1 WHERE id = $2")
            .bind(forbidden)
            .bind(w.teams[T_EPD])
            .execute(&pool)
            .await?;
        for (i, &p) in w.profiles.iter().enumerate() {
            let who = format!("{} (forbid at top {forbidden})", PROFILE_HANDLES[i]);
            let reading = set("SELECT resource_id FROM resources_visible_to($1)", p).await?;
            let reading_body =
                set("SELECT resource_id FROM _resources_visible($1, true)", p).await?;
            let member = set(
                "SELECT resource_id FROM resources_visible_by_membership($1)",
                p,
            )
            .await?;
            let member_body =
                set("SELECT resource_id FROM _resources_visible($1, false)", p).await?;
            let creading = set("SELECT context_id FROM contexts_readable_by($1)", p).await?;
            let creading_body = set(
                "SELECT context_id FROM _contexts_readable_by_teams($1, \
                   coalesce((SELECT array_agg(team_id) FROM profile_reachable_teams($1)), '{}'), \
                   true)",
                p,
            )
            .await?;
            let cmember = set(
                "SELECT context_id FROM contexts_readable_by_membership($1)",
                p,
            )
            .await?;
            let delta = set("SELECT resource_id FROM resources_public_within($1)", p).await?;
            let mut union: Vec<Uuid> = member.iter().chain(delta.iter()).copied().collect();
            union.sort();
            union.dedup();
            if union != reading {
                violations.push(format!(
                    "{who}: reading is not membership UNION resources_public_within"
                ));
            }
            let person: bool = sqlx::query_scalar("SELECT public_within_reading($1)")
                .bind(p)
                .fetch_one(&pool)
                .await?;

            if reading != reading_body {
                violations.push(format!("{who}: resources_visible_to differs from its body"));
            }
            if member != member_body {
                violations.push(format!(
                    "{who}: resources_visible_by_membership differs from _resources_visible(p, false)"
                ));
            }
            if creading != creading_body {
                violations.push(format!("{who}: contexts_readable_by differs from its body"));
            }
            if !member.iter().all(|r| reading.contains(r)) {
                violations.push(format!("{who}: membership is not a subset of reading"));
            }
            if !cmember.iter().all(|c| creading.contains(c)) {
                violations.push(format!(
                    "{who}: context membership is not a subset of reading"
                ));
            }
            let extra: Vec<&Uuid> = reading.iter().filter(|r| !member.contains(r)).collect();
            if !person && !extra.is_empty() {
                violations.push(format!(
                    "{who}: not a person, yet reads {extra:?} beyond membership"
                ));
            }
            if forbidden && (!extra.is_empty() || !delta.is_empty()) {
                violations.push(format!(
                    "{who}: under a forbid above the team, yet reads {extra:?} beyond membership"
                ));
            }

            // The single-object reach: for everything the profile reads, `public_within` exactly
            // when it is outside the profile's membership set, `membership` otherwise. Asked of
            // every read object, so a reach function that disagreed with the two sets for any
            // profile, resource or context, with or without the forbid, would show here.
            for r in &reading {
                let via: Option<String> = sqlx::query_scalar("SELECT resource_reached_via($1, $2)")
                    .bind(p)
                    .bind(r)
                    .fetch_one(&pool)
                    .await?;
                let want = if member.contains(r) {
                    "membership"
                } else {
                    "public_within"
                };
                if via.as_deref() != Some(want) {
                    violations.push(format!(
                        "{who}: resource_reached_via({r}) is {via:?}, the sets say {want}"
                    ));
                }
            }
            for c in &creading {
                let via: Option<String> = sqlx::query_scalar("SELECT context_reached_via($1, $2)")
                    .bind(p)
                    .bind(c)
                    .fetch_one(&pool)
                    .await?;
                let want = if cmember.contains(c) {
                    "membership"
                } else {
                    "public_within"
                };
                if via.as_deref() != Some(want) {
                    violations.push(format!(
                        "{who}: context_reached_via({c}) is {via:?}, the sets say {want}"
                    ));
                }
            }
        }
    }

    // The witnesses, with the forbid cleared.
    sqlx::query("UPDATE kb_teams SET public_within_forbidden = false WHERE id = $1")
        .bind(w.teams[T_EPD])
        .execute(&pool)
        .await?;
    let ask = |sql: &'static str, a: Uuid, b: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(sql)
                .bind(a)
                .bind(b)
                .fetch_one(&pool)
                .await
        }
    };
    let (dana, ella, robot) = (w.profiles[P_DANA], w.profiles[P_ELLA], w.profiles[P_ROBOT]);
    let (r_pub, c_pub) = (w.resources[R_PUB], w.contexts[C_PUB]);
    let (t_pub, t_eng) = (w.teams[T_PUB], w.teams[T_ENG]);
    // The reach invariant above is not vacuous: dana reads r_pub and ctx-pub only through the
    // arm, so both reach functions answer public_within for her at least once.
    for (sql, object) in [
        ("SELECT resource_reached_via($1, $2)", r_pub),
        ("SELECT context_reached_via($1, $2)", c_pub),
    ] {
        let via: Option<String> = sqlx::query_scalar(sql)
            .bind(dana)
            .bind(object)
            .fetch_one(&pool)
            .await?;
        if via.as_deref() != Some("public_within") {
            violations.push(format!(
                "witness: {sql} for dana is {via:?}, not public_within"
            ));
        }
    }
    for (got, want, what) in [
        (
            ask(
                "SELECT can('kb_profiles', $1, 'read', 'kb_resources', $2)",
                dana,
                r_pub,
            )
            .await?,
            true,
            "dana can(read) r_pub",
        ),
        (
            ask(
                "SELECT can_read_by_membership($1, 'kb_resources', $2)",
                dana,
                r_pub,
            )
            .await?,
            false,
            "dana can_read_by_membership r_pub",
        ),
        (
            ask("SELECT context_visible_to($1, $2)", dana, c_pub).await?,
            true,
            "dana context_visible_to ctx-pub",
        ),
        (
            ask(
                "SELECT can_read_by_membership($1, 'kb_contexts', $2)",
                dana,
                c_pub,
            )
            .await?,
            false,
            "dana can_read_by_membership ctx-pub",
        ),
        (
            ask("SELECT context_visible_to($1, $2)", ella, c_pub).await?,
            false,
            "ella context_visible_to ctx-pub",
        ),
        (
            ask("SELECT context_visible_to($1, $2)", robot, c_pub).await?,
            false,
            "robot context_visible_to ctx-pub",
        ),
        (
            ask(
                "SELECT public_within_reading($1) AND $2 IS NOT NULL",
                dana,
                dana,
            )
            .await?,
            true,
            "public_within_reading(dana)",
        ),
        (
            ask(
                "SELECT public_within_reading($1) AND $2 IS NOT NULL",
                robot,
                robot,
            )
            .await?,
            false,
            "public_within_reading(robot)",
        ),
        (
            ask(
                "SELECT public_within_row_effective($1, $2, false)",
                t_pub,
                t_eng,
            )
            .await?,
            true,
            "public_within_row_effective(pub, eng)",
        ),
        (
            ask(
                "SELECT public_within_allowed_over($1) AND $2 IS NOT NULL",
                t_pub,
                t_pub,
            )
            .await?,
            true,
            "public_within_allowed_over(pub)",
        ),
        (
            ask(
                "SELECT anchor_subtree_holds_pool($1) AND $2 IS NOT NULL",
                t_eng,
                t_eng,
            )
            .await?,
            false,
            "anchor_subtree_holds_pool(eng)",
        ),
        (
            // `eng` and pub are not each other's ancestors in the other direction.
            ask(
                "SELECT public_within_row_effective($1, $2, false)",
                t_eng,
                t_pub,
            )
            .await?,
            false,
            "public_within_row_effective(eng, pub): the anchor is not an ancestor",
        ),
    ] {
        if got != want {
            violations.push(format!("{what}: expected {want}, got {got}"));
        }
    }

    // The other answer of each predicate. A pool under the anchor: make eng the gating team, in a
    // transaction rolled back after the probe, since a set gating slug is never cleared again
    // (20261124000050) and the world may have started with none.
    let mut probe = pool.begin().await?;
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'eng' WHERE id = 1")
        .execute(&mut *probe)
        .await?;
    let pool_true: bool = sqlx::query_scalar("SELECT anchor_subtree_holds_pool($1)")
        .bind(t_eng)
        .fetch_one(&mut *probe)
        .await?;
    let pool_row: bool = sqlx::query_scalar("SELECT public_within_row_effective($1, $2, false)")
        .bind(t_pub)
        .bind(t_eng)
        .fetch_one(&mut *probe)
        .await?;
    probe.rollback().await?;
    // A forbid above T.
    sqlx::query("UPDATE kb_teams SET public_within_forbidden = true WHERE id = $1")
        .bind(t_eng)
        .execute(&pool)
        .await?;
    let forbid_allowed = ask(
        "SELECT public_within_allowed_over($1) AND $2 IS NOT NULL",
        t_pub,
        t_pub,
    )
    .await?;
    sqlx::query("UPDATE kb_teams SET public_within_forbidden = false WHERE id = $1")
        .bind(t_eng)
        .execute(&pool)
        .await?;
    // A forbid at the top of the world's tree, above the anchor.
    sqlx::query("UPDATE kb_teams SET public_within_forbidden = true WHERE id = $1")
        .bind(w.teams[T_EPD])
        .execute(&pool)
        .await?;
    let top_allowed = ask(
        "SELECT public_within_allowed_over($1) AND $2 IS NOT NULL",
        t_pub,
        t_pub,
    )
    .await?;
    let top_row = ask(
        "SELECT public_within_row_effective($1, $2, false)",
        t_pub,
        t_eng,
    )
    .await?;
    sqlx::query("UPDATE kb_teams SET public_within_forbidden = false WHERE id = $1")
        .bind(w.teams[T_EPD])
        .execute(&pool)
        .await?;
    for (got, want, what) in [
        (
            pool_true,
            true,
            "anchor_subtree_holds_pool(eng) with eng the gating team",
        ),
        (
            pool_row,
            false,
            "public_within_row_effective(pub, eng) under a pool",
        ),
        (
            forbid_allowed,
            false,
            "public_within_allowed_over(pub) under a forbid on eng",
        ),
        (
            top_allowed,
            false,
            "public_within_allowed_over(pub) under a forbid at the top",
        ),
        (
            top_row,
            false,
            "public_within_row_effective(pub, eng) under a forbid at the top",
        ),
    ] {
        if got != want {
            violations.push(format!("{what}: expected {want}, got {got}"));
        }
    }

    assert!(
        violations.is_empty(),
        "public_within invariants violated:\n{}\n\nworld:\n{}",
        violations.join("\n"),
        w.render()
    );
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

/// Invariant 7's driver. The generated world contains no erased resource, so I7 over it alone
/// would pass vacuously (every answer would be the `false` of the not-erased branch). Five
/// edge-free resources are therefore erased through the REAL act (`resource_erasure_execute`,
/// never an UPDATE): one per distinguishing arm, including a cogmap-grant admission and a
/// context-homed resource whose context readers must NOT hold the husk. The anti-vacuity asserts
/// pin that the act took and that at least one profile holds a husk.
///
/// Bite: dropping an arm from `resource_husk_held_by`, adding the context arm, or dropping the
/// `erased_at` test (every live resource would answer true for its owner) makes I7 report.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_husk_predicate_equals_the_three_arm_reach_over_erased_resources(
    pool: PgPool,
) -> sqlx::Result<()> {
    let seed = harness_seed();
    let w = insert_world(&pool, &Plan::generate(seed)).await?;

    let operator: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
          WHERE p.handle = 'system' AND e.name = 'system'",
    )
    .fetch_one(&pool)
    .await?;
    for r in [
        R_SHARED,
        R_PERSONAL,
        R_OWNED_ANYWHERE,
        R_ANCESTOR_GRANT,
        R_MAP_GRANT,
    ] {
        sqlx::query("SELECT resource_erasure_execute($1, $2, $3, $4)")
            .bind(w.resources[r])
            .bind(operator)
            .bind(operator)
            .bind(Uuid::now_v7())
            .execute(&pool)
            .await?;
    }

    let erased: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resources WHERE erased_at IS NOT NULL")
            .fetch_one(&pool)
            .await?;
    assert_eq!(erased, 5, "the act erased the five chosen resources");

    let mut held_pairs = 0;
    for &resource in &w.resources {
        for &profile in &w.profiles {
            let held: bool = sqlx::query_scalar("SELECT resource_husk_held_by($1, $2)")
                .bind(profile)
                .bind(resource)
                .fetch_one(&pool)
                .await?;
            held_pairs += i64::from(held);
        }
    }
    assert!(
        held_pairs > 0,
        "no profile holds any husk: I7 would only be exercising the `false` branch"
    );

    let violations = i7_husk_held_by_equals_the_three_arm_reach(&pool, &w).await?;
    assert!(
        violations.is_empty(),
        "{} husk-predicate violation(s) over seed {}:\n  - {}\n\n{}",
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

/// The blob gate's bite. `blob_readable_by_profile` is enrolled on a DUALITY (it must equal the
/// anchor gate's answer on the blob's home — I1.blobs), and a duality whose two sides are driven
/// over fixture rows the world cannot make unreadable would be vacuous. This bite restates the
/// gate as constant-TRUE and requires the blob invariant to go red on exactly that label — the
/// same inside-the-test mutation discipline as [`the_harness_bites_a_search_cte_restatement`],
/// aimed at the one predicate the blob surfaces added to the substrate.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_harness_bites_a_blob_gate_restatement(pool: PgPool) -> sqlx::Result<()> {
    let plan = Plan::generate(harness_seed());
    let w = insert_world(&pool, &plan).await?;

    let pre = i1_read_duality(&pool, &w).await?;
    assert!(
        pre.is_empty(),
        "precondition failed: the read duality must hold before the mutation, or the bite \
         proves nothing: {pre:?}"
    );

    let original: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef(p.oid) FROM pg_proc p \
           JOIN pg_namespace n ON n.oid = p.pronamespace \
          WHERE n.nspname = 'public' AND p.prokind = 'f' AND p.proname = 'blob_readable_by_profile'",
    )
    .fetch_one(&pool)
    .await?;

    // THE RESTATEMENT: the gate answers true unconditionally — the drift that would make every
    // blob readable to everyone while the anchor gate still says otherwise.
    const TRUTHY: &str = "SELECT true";
    let mutated = format!(
        "CREATE OR REPLACE FUNCTION blob_readable_by_profile(p_profile uuid, p_blob uuid) \
         RETURNS boolean LANGUAGE sql STABLE AS $body$ {TRUTHY} $body$;"
    );

    sqlx::query(&mutated).execute(&pool).await?;
    let bitten = i1_read_duality(&pool, &w).await?;
    sqlx::query(&original).execute(&pool).await?; // restore before asserting

    assert!(
        !bitten.is_empty(),
        "restating blob_readable_by_profile as constant-true did NOT turn the read duality red \
         — the enrollment is vacuous"
    );
    assert!(
        bitten.iter().any(|v| v.contains("I1.blobs")),
        "the mutation was caught, but not by the blob invariant: {bitten:?}"
    );

    let restored = i1_read_duality(&pool, &w).await?;
    assert!(
        restored.is_empty(),
        "after restore the blob invariant did not come back green: {restored:?}"
    );

    Ok(())
}

/// `resources_changed_by` and `contexts_changed_by` answer "what did this profile change" and must
/// name nothing the profile cannot read. The generated world holds no event any of its profiles
/// emitted, so a subset check over it alone would pass vacuously. Every world profile is therefore
/// given an emitter, and one change naming every world resource and one change in every world
/// context. Then each reader must answer **exactly** that profile's visible set: a gate too wide
/// leaks a tombstoned or unreachable row, and a gate too narrow drops one the profile can read.
///
/// Bite: dropping the `resources_visible_to` join from `resources_changed_by`, or the
/// `contexts_readable_by` join from `contexts_changed_by`, makes this report.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_change_readers_answer_exactly_their_visible_sets(pool: PgPool) -> sqlx::Result<()> {
    let seed = harness_seed();
    let w = insert_world(&pool, &Plan::generate(seed)).await?;

    let changed: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_event_types WHERE name = 'resource_updated'")
            .fetch_one(&pool)
            .await?;
    for &profile in &w.profiles {
        let entity: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'harness@web') RETURNING id",
        )
        .bind(profile)
        .fetch_one(&pool)
        .await?;
        for &resource in &w.resources {
            sqlx::query(
                "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, \
                                        producing_anchor_id, payload) \
                 VALUES ($1, $2, 'kb_contexts', $3, jsonb_build_object('resource_id', $4::uuid))",
            )
            .bind(changed)
            .bind(entity)
            .bind(w.contexts[0])
            .bind(resource)
            .execute(&pool)
            .await?;
        }
        for &context in &w.contexts {
            sqlx::query(
                "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, \
                                        producing_anchor_id) \
                 VALUES ($1, $2, 'kb_contexts', $3)",
            )
            .bind(changed)
            .bind(entity)
            .bind(context)
            .execute(&pool)
            .await?;
        }
    }

    let mut violations = Vec::new();
    let mut nonempty = 0;
    let mut narrowed = 0;
    for (p_idx, &profile) in w.profiles.iter().enumerate() {
        let who = PROFILE_HANDLES[p_idx];
        let visible: Vec<Uuid> = scalar_set(
            &pool,
            "SELECT resource_id FROM resources_visible_to($1) ORDER BY 1",
            profile,
        )
        .await?
        .into_iter()
        .filter(|r| w.resources.contains(r))
        .collect();
        let changed_resources = scalar_set(
            &pool,
            "SELECT resource_id FROM resources_changed_by($1) ORDER BY 1",
            profile,
        )
        .await?;
        if changed_resources != visible {
            violations.push(format!(
                "I8.resources: resources_changed_by[{who}] = {changed_resources:?}, but the \
                 visible world resources are {visible:?}"
            ));
        }
        nonempty += usize::from(!changed_resources.is_empty());
        narrowed += usize::from(visible.len() < w.resources.len());

        let readable: Vec<Uuid> = scalar_set(
            &pool,
            "SELECT context_id FROM contexts_readable_by($1) ORDER BY 1",
            profile,
        )
        .await?
        .into_iter()
        .filter(|c| w.contexts.contains(c))
        .collect();
        let changed_contexts = scalar_set(
            &pool,
            "SELECT context_id FROM contexts_changed_by($1) ORDER BY 1",
            profile,
        )
        .await?;
        if changed_contexts != readable {
            violations.push(format!(
                "I8.contexts: contexts_changed_by[{who}] = {changed_contexts:?}, but the \
                 readable world contexts are {readable:?}"
            ));
        }
    }
    // Anti-vacuity: some profile reads some changed resource, and some profile's visible set is
    // narrower than the whole world, so a missing gate would have something to leak.
    assert!(
        nonempty > 0,
        "anti-vacuity: no profile reads any changed resource"
    );
    assert!(
        narrowed > 0,
        "anti-vacuity: every profile reads every world resource"
    );
    assert!(
        violations.is_empty(),
        "{} change-reader violation(s) over seed {}:\n  - {}\n\n{}",
        violations.len(),
        seed,
        violations.join("\n  - "),
        w.render()
    );
    Ok(())
}

/// `context_changes_after` (the change feed) answers a profile for a context exactly when that
/// context is in `contexts_readable_by`, and NULL otherwise: refusal, never an empty answer that
/// would confirm the context exists. Every world context is given one change, so a readable context's
/// answer must serve it. An answer that is non-NULL but empty would be a gate that admits and then
/// hides, which this also reports.
///
/// The feed serves only settled rows (R5: the horizon is cluster-wide), so the read waits until the
/// changes inserted here are below it.
///
/// Bite: replacing `context_readable_by_profile(p_profile, p_context)` in the function with `true`
/// makes this report I9 for every profile that cannot read some world context.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_change_feed_answers_exactly_the_readable_contexts(pool: PgPool) -> sqlx::Result<()> {
    let seed = harness_seed();
    let w = insert_world(&pool, &Plan::generate(seed)).await?;

    let changed: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_event_types WHERE name = 'resource_updated'")
            .fetch_one(&pool)
            .await?;
    let entity: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'harness@web') RETURNING id",
    )
    .bind(w.profiles[0])
    .fetch_one(&pool)
    .await?;
    for &context in &w.contexts {
        sqlx::query(
            "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, \
                                    producing_anchor_id) \
             VALUES ($1, $2, 'kb_contexts', $3)",
        )
        .bind(changed)
        .bind(entity)
        .bind(context)
        .execute(&pool)
        .await?;
    }
    let target: String = sqlx::query_scalar("SELECT pg_snapshot_xmax(pg_current_snapshot())::text")
        .fetch_one(&pool)
        .await?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !sqlx::query_scalar::<_, bool>(
        "SELECT pg_snapshot_xmin(pg_current_snapshot()) >= $1::text::xid8",
    )
    .bind(&target)
    .fetch_one(&pool)
    .await?
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the horizon did not settle"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let mut violations = Vec::new();
    // Anti-vacuity counts the world, not the function's answers, so a gate widened to `true` reports
    // as I9 rather than as a vacuous world.
    let mut unreadable_pairs = 0;
    let mut readable_pairs = 0;
    for (p_idx, &profile) in w.profiles.iter().enumerate() {
        let who = PROFILE_HANDLES[p_idx];
        let readable = scalar_set(
            &pool,
            "SELECT context_id FROM contexts_readable_by($1) ORDER BY 1",
            profile,
        )
        .await?;
        for &context in &w.contexts {
            let answer: Option<serde_json::Value> =
                sqlx::query_scalar("SELECT context_changes_after($1, $2, '0'::xid8, 0, 1000)")
                    .bind(profile)
                    .bind(context)
                    .fetch_one(&pool)
                    .await?;
            let may = readable.contains(&context);
            if may {
                readable_pairs += 1;
            } else {
                unreadable_pairs += 1;
            }
            let cursor: Option<Uuid> = sqlx::query_scalar("SELECT context_cursor($1, $2)")
                .bind(profile)
                .bind(context)
                .fetch_one(&pool)
                .await?;
            match (cursor, may) {
                (None, false) | (Some(_), true) => {}
                (Some(id), false) => violations.push(format!(
                    "I9: context_cursor[{who}, {context}] answered {id} for a context outside \
                     contexts_readable_by"
                )),
                (None, true) => violations.push(format!(
                    "I9: context_cursor[{who}, {context}] answered nothing for a readable \
                     context holding a served change"
                )),
            }
            match (&answer, may) {
                (None, false) => {}
                (Some(a), true) => {
                    if a["entries"].as_array().is_none_or(|e| e.is_empty()) {
                        violations.push(format!(
                            "I9: context_changes_after[{who}, {context}] admitted a readable \
                             context but served none of its change: {a}"
                        ));
                    }
                }
                (Some(a), false) => violations.push(format!(
                    "I9: context_changes_after[{who}, {context}] answered a context outside \
                     contexts_readable_by: {a}"
                )),
                (None, true) => violations.push(format!(
                    "I9: context_changes_after[{who}, {context}] refused a context in \
                     contexts_readable_by"
                )),
            }
        }
    }
    assert!(
        unreadable_pairs > 0,
        "anti-vacuity: every profile can read every world context"
    );
    assert!(
        readable_pairs > 0,
        "anti-vacuity: no profile can read any world context"
    );
    assert!(
        violations.is_empty(),
        "{} change-feed violation(s) over seed {}:\n  - {}\n\n{}",
        violations.len(),
        seed,
        violations.join("\n  - "),
        w.render()
    );
    Ok(())
}

// =================================================================================================
// list_visible: the set-based query equals the per-context predicate
// =================================================================================================

/// `context_service::list_visible`'s per-context form: the rows `context_visible_to` admits, each
/// counting the live resources homed in it that `resources_visible_to` admits, evaluated per
/// context. The door now computes both sets once per call; this is what it must still equal.
const LIST_VISIBLE_PER_CONTEXT: &str = "\
    SELECT c.id, \
           (SELECT count(*) FROM kb_resource_homes rh \
              JOIN resources_visible_to($1) v ON v.resource_id = rh.resource_id \
              JOIN kb_resources r ON r.id = rh.resource_id AND r.is_active \
             WHERE rh.anchor_table = 'kb_contexts' AND rh.anchor_id = c.id) \
      FROM kb_contexts c WHERE context_visible_to($1, c.id)";

/// The door's set-based query with its row gate drifted to the membership twin: what a rewrite
/// that reached for the wrong set would ship. Used only by the bite below.
const LIST_VISIBLE_SET_ON_MEMBERSHIP: &str = "\
    WITH readable AS MATERIALIZED (SELECT r.context_id FROM contexts_readable_by_membership($1) r), \
         visible AS MATERIALIZED (SELECT v.resource_id FROM resources_visible_to($1) v) \
    SELECT c.id, \
           (SELECT count(*) FROM kb_resource_homes rh \
              JOIN visible v ON v.resource_id = rh.resource_id \
              JOIN kb_resources r ON r.id = rh.resource_id AND r.is_active \
             WHERE rh.anchor_table = 'kb_contexts' AND rh.anchor_id = c.id) \
      FROM kb_contexts c WHERE c.id IN (SELECT rc.context_id FROM readable rc)";

/// Which answer the slice holds against the per-context form: the real door, or (for the bite) a
/// drifted restatement of its query.
#[derive(Clone, Copy)]
enum ListDoor {
    Real,
    Drifted,
}

async fn list_visible_slice(
    pool: &PgPool,
    w: &World,
    door: ListDoor,
) -> sqlx::Result<Vec<Violation>> {
    let mut violations = Vec::new();
    for forbidden in [false, true] {
        sqlx::query("UPDATE kb_teams SET public_within_forbidden = $1 WHERE id = $2")
            .bind(forbidden)
            .bind(w.teams[T_EPD])
            .execute(pool)
            .await?;
        for (i, &p) in w.profiles.iter().enumerate() {
            let who = format!("{} (forbid at top {forbidden})", PROFILE_HANDLES[i]);
            let mut expected: Vec<(Uuid, i64)> = sqlx::query_as(LIST_VISIBLE_PER_CONTEXT)
                .bind(p)
                .fetch_all(pool)
                .await?;
            expected.sort();
            let mut got: Vec<(Uuid, i64)> = match door {
                ListDoor::Real => temper_services::services::context_service::list_visible(
                    pool,
                    temper_core::types::ids::ProfileId::from(p),
                )
                .await
                .map_err(|e| sqlx::Error::Protocol(format!("list_visible: {e}")))?
                .into_iter()
                .map(|r| (Uuid::from(r.id), r.resource_count))
                .collect(),
                ListDoor::Drifted => {
                    sqlx::query_as(LIST_VISIBLE_SET_ON_MEMBERSHIP)
                        .bind(p)
                        .fetch_all(pool)
                        .await?
                }
            };
            got.sort();
            if got != expected {
                violations.push(format!(
                    "I8.list_visible: {who}: list_visible answered {got:?}, the per-context form \
                     {expected:?}"
                ));
            }
        }
    }
    sqlx::query("UPDATE kb_teams SET public_within_forbidden = false WHERE id = $1")
        .bind(w.teams[T_EPD])
        .execute(pool)
        .await?;
    Ok(violations)
}

/// `list_visible` computes the reader's context set and resource set once per call. Over every
/// profile of a generated world, with the public_within row effective and again under a forbid at
/// the top, its rows are exactly `{c | context_visible_to(p, c)}` and each `resource_count` equals
/// the per-context count. Either side rewritten later cannot drift from the other silently.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn list_visible_equals_the_per_context_read_predicate(pool: PgPool) -> sqlx::Result<()> {
    let plan = Plan::generate(harness_seed());
    let w = insert_world(&pool, &plan).await?;
    let violations = list_visible_slice(&pool, &w, ListDoor::Real).await?;
    assert!(
        violations.is_empty(),
        "list_visible drifted from the per-context predicate:\n{}\n{}",
        violations.join("\n"),
        w.render()
    );
    Ok(())
}

/// The bite: the same slice over the door's query with its row gate on the membership twin must go
/// red. The world's public_within row opens ctx-pub to the people under eng through reading only,
/// so a membership gate drops it for them: proof the slice tells the two sets apart.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_harness_bites_a_list_visible_gate_on_the_membership_twin(
    pool: PgPool,
) -> sqlx::Result<()> {
    let plan = Plan::generate(harness_seed());
    let w = insert_world(&pool, &plan).await?;
    let bitten = list_visible_slice(&pool, &w, ListDoor::Drifted).await?;
    assert!(
        bitten.iter().any(|v| v.contains("I8.list_visible")),
        "a list_visible gated on contexts_readable_by_membership did NOT turn the slice red"
    );
    assert!(
        bitten.iter().all(|v| v.contains("forbid at top false")),
        "under a forbid at the top the twin and reading agree, so only the effective case may \
         differ: {bitten:?}"
    );
    Ok(())
}

// =================================================================================================
// public_within effects follow every structural act
// =================================================================================================

/// Over the generated world, with a public_within row opened for every eligible (team, live
/// ancestor) pair, a seeded run of structural and admin acts through their SQL chokepoints (delete,
/// restore, link, unlink, auto-join, forbid, rename, gating-slug repoint). After EVERY act, each row's recorded effect (its effect
/// when the run began, folded through every `public_within_effect_changed` since) equals
/// `public_within_row_effective`, and a row is gone exactly when a `lapsed` record says so: no act
/// changes a row's effect, or deletes it, without a record.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn public_within_records_every_effect_change_of_every_structural_act(
    pool: PgPool,
) -> sqlx::Result<()> {
    use std::collections::HashMap;

    let plan = Plan::generate(harness_seed());
    let w = insert_world(&pool, &plan).await?;
    let system: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
          WHERE p.handle = 'system' AND e.name = 'system'",
    )
    .fetch_one(&pool)
    .await?;

    // Every eligible pair the shape guard admits, opened directly.
    sqlx::query(
        "INSERT INTO kb_team_public_within (team_id, anchor_id, opened_by) \
         SELECT t.id, a.team_id, $2 \
           FROM kb_teams t CROSS JOIN LATERAL team_ancestors(t.id) a \
           JOIN kb_teams at ON at.id = a.team_id \
          WHERE t.id = ANY($1) AND a.team_id <> t.id AND t.personal_of IS NULL \
            AND at.personal_of IS NULL AND NOT team_is_root_or_gating(t.id) \
         ON CONFLICT DO NOTHING",
    )
    .bind(&w.teams)
    .bind(w.profiles[0])
    .execute(&pool)
    .await?;
    let mut recorded: HashMap<(Uuid, Uuid), bool> = sqlx::query_as::<_, (Uuid, Uuid, bool)>(
        "SELECT team_id, anchor_id, public_within_row_effective(team_id, anchor_id, instance_wide) \
           FROM kb_team_public_within",
    )
    .fetch_all(&pool)
    .await?
    .into_iter()
    .map(|(t, a, e)| ((t, a), e))
    .collect();
    assert!(recorded.len() >= 3, "the world opens rows: {}", w.render());
    let mut seen: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name IN ('public_within_effect_changed', 'team_public_within_changed')",
    )
    .fetch_one(&pool)
    .await?;
    let seen_at_start = seen;

    let mut rng = Rng::new(plan.seed ^ 0x9E37_79B9);
    let n = w.teams.len() as u64;
    let mut acts = 0;
    for step in 0..60 {
        let a = w.teams[(rng.next_u64() % n) as usize];
        let b = w.teams[(rng.next_u64() % n) as usize];
        let (label, sql): (&str, String) = match rng.next_u64() % 8 {
            6 => ("rename", format!("SELECT _admin_team_lifecycle_act('{system}', NULL, '{a}', 'renamed', 'h-{step}')")),
            7 => ("gating", format!("SELECT _admin_gating_team_slug_write(t.slug) FROM kb_teams t WHERE t.id = '{b}'")),
            0 => (
                "delete",
                format!("SELECT _admin_team_lifecycle_act('{system}', NULL, '{a}', 'deleted')"),
            ),
            1 => (
                "restore",
                format!("SELECT _admin_team_lifecycle_act('{system}', NULL, '{a}', 'restored')"),
            ),
            2 => (
                "link",
                format!(
                    "SELECT _admin_team_parent_link_act('{system}', NULL, 'link', '{a}', '{b}')"
                ),
            ),
            3 => {
                let edge: Option<(Uuid, Uuid)> = sqlx::query_as(
                    "SELECT parent_id, child_id FROM kb_teams_parents \
                      WHERE child_id = ANY($1) ORDER BY parent_id, child_id \
                      OFFSET $2 LIMIT 1",
                )
                .bind(&w.teams)
                .bind((rng.next_u64() % n) as i64)
                .fetch_optional(&pool)
                .await?;
                let Some((p, c)) = edge else { continue };
                ("unlink", format!("SELECT _admin_team_parent_link_act('{system}', NULL, 'unlink', '{p}', '{c}')"))
            }
            4 => {
                let role = if rng.chance(50) { "'member'" } else { "NULL" };
                ("auto-join", format!("SELECT _admin_team_lifecycle_act('{system}', NULL, '{a}', 'auto_join_changed', {role})"))
            }
            _ => {
                let f = rng.chance(50);
                (
                    "forbid",
                    format!(
                        "SELECT _admin_public_within_policy_write('{system}', NULL, '{a}', {f})"
                    ),
                )
            }
        };
        // An act a trigger refuses (a cycle, a personal or root link) writes nothing; skip it.
        let mut tx = pool.begin().await?;
        if sqlx::query(&sql).execute(&mut *tx).await.is_err() {
            continue;
        }
        tx.commit().await?;
        acts += 1;

        let new: Vec<serde_json::Value> = sqlx::query_scalar(
            "SELECT e.payload || jsonb_build_object('type', et.name) FROM kb_events e \
               JOIN kb_event_types et ON et.id = e.event_type_id \
              WHERE et.name IN ('public_within_effect_changed', 'team_public_within_changed') \
              ORDER BY e.id OFFSET $1",
        )
        .bind(seen)
        .fetch_all(&pool)
        .await?;
        seen += new.len() as i64;
        for rec in &new {
            let key: (Uuid, Uuid) = (
                rec["team_id"].as_str().unwrap().parse().unwrap(),
                rec["anchor_id"].as_str().unwrap().parse().unwrap(),
            );
            if rec["type"] == "team_public_within_changed" {
                assert_eq!(rec["act"], "lapsed", "step {step} {label}: {rec}");
                assert!(
                    recorded.remove(&key).is_some(),
                    "step {step} {label}: lapsed twice"
                );
            } else {
                assert_eq!(
                    recorded.insert(key, rec["to"] == "effective"),
                    Some(rec["from"] == "effective"),
                    "step {step} {label}: the record's `from` is not the recorded state"
                );
            }
        }
        let now: HashMap<(Uuid, Uuid), bool> = sqlx::query_as::<_, (Uuid, Uuid, bool)>(
            "SELECT team_id, anchor_id, \
                    public_within_row_effective(team_id, anchor_id, instance_wide) \
               FROM kb_team_public_within",
        )
        .fetch_all(&pool)
        .await?
        .into_iter()
        .map(|(t, a, e)| ((t, a), e))
        .collect();
        assert_eq!(
            now,
            recorded,
            "step {step} {label}: the recorded effects drifted from the rows\n{}",
            w.render()
        );
        // A row left in place has its anchor above its team by some path.
        let orphaned: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_team_public_within pw \
              WHERE NOT EXISTS (SELECT 1 FROM team_ancestors_any(pw.team_id) a \
                                 WHERE a.team_id = pw.anchor_id)",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(orphaned, 0, "step {step} {label}: a lapsed row survived");
    }
    assert!(acts >= 20, "the run acted: {acts}");
    assert!(
        seen > seen_at_start,
        "the run recorded no transition at all, so it proved nothing (seed {})",
        plan.seed
    );
    Ok(())
}
