//! Declarative YAML model for the access-scenario kind. Reuses `ProfileDef`, `EntityDef`, `TelosDef`,
//! and `EdgeKind` from the charter scenario model; adds the access topology (teams + DAG, multi-cogmap,
//! homes, grants) and the `AccessCheck` set. All enums are **internally tagged** (an `anchor`/`check`
//! discriminator field) because serde_yaml 0.9 rejects the externally-tagged single-key-map form.

use crate::affinity::EdgeKind;
use crate::scenario::model::{EntityDef, ProfileDef, TelosDef};
use serde::Deserialize;

fn one() -> f64 {
    1.0
}

/// The access-scenario document (`schema-artifact/access-scenarios/*.yaml`): a full access world plus
/// the inline checks that assert it.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct AccessScenario {
    pub name: String,
    pub world: AccessWorld,
    pub checks: Vec<AccessCheck>,
}

/// The access topology. `profiles`/`entities` reuse the charter model's defs.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct AccessWorld {
    pub profiles: Vec<ProfileDef>,
    pub entities: Vec<EntityDef>,
    pub teams: Vec<TeamDef>,
    #[serde(default)]
    pub memberships: Vec<MembershipDef>,
    #[serde(default)]
    pub contexts: Vec<ContextDef>,
    #[serde(default)]
    pub context_shares: Vec<ContextShareDef>,
    pub cogmaps: Vec<AccessCogmapDef>,
    pub resources: Vec<AccessResourceDef>,
    #[serde(default)]
    pub edges: Vec<AccessEdgeDef>,
}

/// A team. `parents` are slugs in this same `teams` list (the down-only DAG, `kb_teams_parents`).
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct TeamDef {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub parents: Vec<String>,
}

/// A sub-team membership. Root (`temper-system`) joins are maintained by the `sync_system_membership`
/// trigger from `system_access`, so they are NOT listed here.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct MembershipDef {
    pub team: String,    // slug
    pub profile: String, // handle
    pub role: String,    // team_role: owner | maintainer | member | watcher
}

/// A named context — a real owner-scoped `kb_contexts` row (WS6 §2 amendment), the referent for named
/// homes and shares. Exactly one of `owner_profile` (a `world` profile handle) / `owner_team` (a
/// `world` team slug) must be set — the loader validates this and derives `slug = slugify(name)`.
/// Owner is purely namespace-scoping (which owner the slug is unique within); reachability is still
/// governed by `context_shares` (`kb_team_contexts`), orthogonal to owner.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ContextDef {
    pub name: String,
    #[serde(default)]
    pub owner_profile: Option<String>, // handle in world.profiles
    #[serde(default)]
    pub owner_team: Option<String>, // slug in world.teams (declared or trigger-created)
}

/// A context-share (`kb_team_contexts`): the team's vis-reach includes the context's
/// resources and context-homed edges (WS6 adjudication §2). `team` may name a
/// trigger-created personal team (`personal-<handle>`) — the loader refreshes its
/// team map from the DB after profiles load.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ContextShareDef {
    pub context: String, // name in world.contexts
    pub team: String,    // slug (declared or trigger-created)
}

/// A cogmap. Bare producer maps carry only `name` + `teams`. A `telos` (charter) makes it a genesis
/// map (needs `owner` + `emitter`); the loader runs `cogmap_genesis` for it.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct AccessCogmapDef {
    pub name: String,
    #[serde(default)]
    pub teams: Vec<String>, // slugs joined via kb_team_cogmaps
    #[serde(default)]
    pub owner: Option<String>, // handle — required only when `telos` is present
    #[serde(default)]
    pub emitter: Option<String>, // entity name — required only when `telos` is present
    #[serde(default)]
    pub telos: Option<TelosDef>,
}

/// A resource: identity + a single home (context or cogmap) + explicit capability grants.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct AccessResourceDef {
    pub key: String,
    pub title: String,
    pub origin_uri: String,
    pub home: HomeDef,
    pub owner: String, // handle — originator + owner on the home row, granter on the grants
    #[serde(default)]
    pub grants: Vec<GrantDef>,
}

/// The resource home anchor. `{ anchor: cogmap, name: <cogmap> }`, `{ anchor: context, name: <context> }`
/// (a real `kb_contexts` row from `world.contexts` — shareable via `context_shares`), or the anonymous
/// `{ anchor: context }` (a synthetic generated-uuid anchor with no row — an unshared workspace).
#[derive(Debug, Deserialize)]
#[serde(tag = "anchor", rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum HomeDef {
    Cogmap {
        name: String,
    },
    Context {
        #[serde(default)]
        name: Option<String>,
    },
}

/// A capability grant (`kb_resource_access`). `to` is a team or profile anchor. Caps default false; the
/// DB CHECK enforces `write|delete|grant ⇒ read`.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct GrantDef {
    pub to: GrantAnchor,
    #[serde(default)]
    pub can_read: bool,
    #[serde(default)]
    pub can_write: bool,
    #[serde(default)]
    pub can_delete: bool,
    #[serde(default)]
    pub can_grant: bool,
}

/// A grant anchor — `{ anchor: team, slug: <slug> }` or `{ anchor: profile, handle: <handle> }`.
#[derive(Debug, Deserialize)]
#[serde(tag = "anchor", rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum GrantAnchor {
    Team { slug: String },
    Profile { handle: String },
}

/// An authored edge homed in a named cogmap, fired through `relationship_assert` (the event-backed path
/// — `kb_edges` carries NOT-NULL event FKs). `from`/`to` are resource keys.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct AccessEdgeDef {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "one")]
    pub weight: f64,
    pub home: EdgeHomeDef, // cogmap or context home anchor
    pub emitter: String,   // entity name
}

/// An edge home anchor — `{ anchor: cogmap, name: .. }` or `{ anchor: context, name: .. }` (a named
/// context from `world.contexts`; context-homed edges gate by context-share, WS6 adjudication §2).
#[derive(Debug, Deserialize)]
#[serde(tag = "anchor", rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum EdgeHomeDef {
    Cogmap { name: String },
    Context { name: String },
}

/// One access assertion. Internally tagged by `check:` (same serde_yaml constraint as the charter
/// `Expectation`). Each variant resolves its named referents and calls one gate function.
#[derive(Debug, Deserialize)]
#[serde(tag = "check", rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum AccessCheck {
    /// S1 — consumer reach: `resources_visible_to(profile)` ∋ resource.
    VisibleTo {
        profile: String,
        resource: String,
        expect: bool,
    },
    /// S2 — producer intersection / leak-safety: `resources_accessible_to_cogmap(cogmap)` ∋ resource.
    ProducerReach {
        cogmap: String,
        resource: String,
        expect: bool,
    },
    /// S3 — edge-home protection: `edges_visible_to(profile)` ∋ edge (resolved by label).
    EdgeVisibleTo {
        profile: String,
        edge: String,
        expect: bool,
    },
    /// S5 — delegation priming: `cogmaps_share_a_team(a, b)`.
    CogmapsShareTeam { a: String, b: String, expect: bool },
    /// S4 — charter-block gating: `count(resource_blocks(cogmap_telos(cogmap), 'profile', profile, NULL))`.
    CharterBlocksVisible {
        cogmap: String,
        profile: String,
        expect_count: i64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
name: t
world:
  profiles:
    - { handle: alice, display_name: Alice, system_access: approved }
    - { handle: nomad, display_name: Nomad, system_access: none }
  entities:
    - { name: carol-agent, profile: alice }
  teams:
    - { slug: temper-system, name: Temper System }
    - { slug: epd-team-a, name: Team A, parents: [temper-system] }
  memberships:
    - { team: epd-team-a, profile: alice, role: member }
  contexts:
    - { name: research, owner_team: epd-team-a }
  context_shares:
    - { context: research, team: epd-team-a }
  cogmaps:
    - { name: side-map, teams: [epd-team-a] }
    - name: onb
      teams: [epd-team-a]
      owner: alice
      emitter: carol-agent
      telos: { title: T, statement: S, questions: [{ question: q }] }
  resources:
    - { key: c, title: "concept: c", origin_uri: "temper://c",
        home: { anchor: cogmap, name: side-map }, owner: alice,
        grants: [{ to: { anchor: team, slug: temper-system }, can_read: true }] }
    - { key: d, title: "doc: d", origin_uri: "temper://d",
        home: { anchor: context, name: research }, owner: alice,
        grants: [{ to: { anchor: profile, handle: alice }, can_read: true }] }
  edges:
    - { from: c, to: d, kind: leads_to, label: "c->d", home: { anchor: cogmap, name: side-map }, emitter: carol-agent }
checks:
  - { check: visible_to, profile: alice, resource: c, expect: true }
  - { check: producer_reach, cogmap: side-map, resource: c, expect: true }
  - { check: edge_visible_to, profile: alice, edge: "c->d", expect: true }
  - { check: cogmaps_share_team, a: side-map, b: onb, expect: true }
  - { check: charter_blocks_visible, cogmap: onb, profile: nomad, expect_count: 0 }
"#;

    #[test]
    fn parses_access_scenario() {
        let s: AccessScenario = serde_yaml::from_str(YAML).unwrap();
        assert_eq!(s.world.teams.len(), 2);
        assert_eq!(s.world.teams[1].parents, vec!["temper-system".to_string()]);
        assert_eq!(s.world.cogmaps.len(), 2);
        assert!(s.world.cogmaps[0].telos.is_none());
        assert!(s.world.cogmaps[1].telos.is_some());
        assert_eq!(s.world.contexts.len(), 1);
        assert_eq!(
            s.world.contexts[0].owner_team.as_deref(),
            Some("epd-team-a")
        );
        assert!(s.world.contexts[0].owner_profile.is_none());
        assert_eq!(s.world.context_shares.len(), 1);
        assert_eq!(s.world.resources.len(), 2);
        assert!(matches!(s.world.resources[0].home, HomeDef::Cogmap { .. }));
        assert!(matches!(
            &s.world.resources[1].home,
            HomeDef::Context { name: Some(n) } if n == "research"
        ));
        assert!(matches!(
            s.world.resources[1].grants[0].to,
            GrantAnchor::Profile { .. }
        ));
        assert_eq!(s.world.edges.len(), 1);
        assert!(matches!(
            &s.world.edges[0].home,
            EdgeHomeDef::Cogmap { name } if name == "side-map"
        ));
        assert_eq!(s.checks.len(), 5);
        assert!(matches!(
            s.checks[0],
            AccessCheck::VisibleTo { expect: true, .. }
        ));
        assert!(matches!(
            s.checks[4],
            AccessCheck::CharterBlocksVisible {
                expect_count: 0,
                ..
            }
        ));
    }

    #[test]
    fn epd_bridge_fixture_deserializes() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../schema-artifact/access-scenarios/epd-bridge-access.yaml"
        );
        let s: AccessScenario =
            serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(s.name, "epd-bridge-access");
        assert_eq!(s.world.profiles.len(), 6);
        assert_eq!(s.world.teams.len(), 6);
        assert_eq!(s.world.cogmaps.len(), 5);
        assert_eq!(s.world.resources.len(), 5);
        assert_eq!(s.world.edges.len(), 1);
        assert_eq!(s.checks.len(), 19);
        // exactly one genesis cogmap (the onboarding charter)
        assert_eq!(
            s.world.cogmaps.iter().filter(|c| c.telos.is_some()).count(),
            1
        );
    }
}
