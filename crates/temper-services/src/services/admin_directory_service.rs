//! The operator directory — the admin surface's read-only principal inventory
//! (admin-operator-directory spec §5/§6).
//!
//! Two reads, both system-admin-gated by the sealed [`SystemAdmin`] proof (the F-3 posture:
//! CLI/MCP/API enforce identically because the gate lives in the service signatures):
//!
//! - [`list_profiles`] — human principals LEFT JOIN `kb_principal_standing` (an absent row
//!   renders and filters as `denied` — the same reading `access_service::get_entitlements`
//!   already makes), with the `needs-access` default filter defined as NOT-approved-including-
//!   absence: literally the `has_system_access` predicate negated (`20260720000110`).
//! - [`profile_card`] / [`profile_card_by_email`] — the state card, composed from existing
//!   tables only. Its invitation rows project from `vw_invitee_invitations` **without** the
//!   view's `token` column (field-pinned), so the attribution rule — an address verified-owned
//!   by two or more profiles addresses nobody — is REUSED, never restated.
//!
//! The directory scopes to HUMAN principals (spec §4): a profile is human when it holds at
//! least one `kb_profile_auth_links` row with `auth_provider <> 'auth0-m2m'`. Machine agents
//! carry links too (both mint doors insert them) and connection profiles carry none, so
//! "has a link" would be the falsified discriminator; the positive form here is the pinned one.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::admin::{
    AdminDirectoryEntry, AdminDirectoryListResponse, AdminOpenJoinRequest, AdminOpenReviewRequest,
    AdminProfileAuthLink, AdminProfileCard, AdminProfileInvitation, AdminProfileTeamMembership,
    AdminProfilesListQuery,
};
use temper_core::types::ids::ProfileId;
use temper_principal::Standing;

use crate::auth::SystemAdmin;
use crate::error::{ApiError, ApiResult};

/// Page size when the caller does not ask for one, and the ceiling when they ask for too much —
/// CONFORM the only clamp the admin surface already had (`handlers/admin_ledger.rs`).
const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
/// Offset ceiling: depth protection one notch beyond the ledger's bare `.max(0)` floor.
const MAX_OFFSET: i64 = 10_000;

/// How the standing filter narrows the list. `needs-access` is the default and is defined as
/// every non-approved admission state INCLUDING no standing row — exactly NOT
/// `has_system_access`, which is `state = 'approved'` and nothing else (`20260720000110`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StandingFilter {
    /// The operator's work queue: NOT approved, absence included.
    NeedsAccess,
    /// One named state; an absent standing row counts as `denied`.
    State(&'static str),
    /// Every human principal.
    All,
}

impl StandingFilter {
    fn parse(raw: Option<&str>) -> ApiResult<Self> {
        let lowered = raw.map(|s| s.trim().to_ascii_lowercase());
        match lowered.as_deref() {
            None | Some("needs-access") => Ok(Self::NeedsAccess),
            Some("all") => Ok(Self::All),
            // The named states are &'static literals — the filter borrows nothing from the
            // request string it validated.
            Some(state) => match state {
                "denied" => Ok(Self::State("denied")),
                "requested" => Ok(Self::State("requested")),
                "approved" => Ok(Self::State("approved")),
                "revoked" => Ok(Self::State("revoked")),
                "deactivated" => Ok(Self::State("deactivated")),
                other => Err(ApiError::BadRequest(format!(
                    "unknown standing filter '{other}': expected denied, requested, approved, \
                     revoked, deactivated, needs-access, or all"
                ))),
            },
        }
    }
}

/// The §6 default-link fallback, derived from the profile's links (which the card fetches in
/// full anyway — one source of truth for both the list row and the card):
///
/// 1. the default link, if it exists and is verified; otherwise
/// 2. the earliest-`linked_at` verified link; otherwise
/// 3. `email: None` with `provisioned_via` = the earliest link's provider (provenance stays
///    honest while the unverified address stays out of the primary slot).
///
/// `links` must be ordered `linked_at ASC` (the card queries' fixed order).
fn derive_default_identity(links: &[AdminProfileAuthLink]) -> (Option<String>, Option<String>) {
    let default_verified = links
        .iter()
        .find(|l| l.is_default && l.email_verified && l.email.is_some());
    let earliest_verified = links.iter().find(|l| l.email_verified && l.email.is_some());
    match default_verified.or(earliest_verified) {
        Some(link) => (link.email.clone(), Some(link.auth_provider.clone())),
        // No verified email anywhere: keep the earliest link's provider, withhold the address.
        None => (None, links.first().map(|l| l.auth_provider.clone())),
    }
}

/// Absence denies, and it is REPORTED as `denied` rather than as an absent field — CONFORM
/// `access_service::get_entitlements`' reading of the same table (`None` on the wire is
/// reserved for "this server predates the field").
fn standing_wire(
    state: Option<String>,
    updated: Option<DateTime<Utc>>,
) -> (String, Option<DateTime<Utc>>) {
    match state {
        Some(state) => (state, updated),
        None => (Standing::Denied.as_str().to_owned(), None),
    }
}

/// One fetched list row plus its window `total` — the count over the WHOLE filtered
/// population, riding the same statement so rows and count can never disagree.
#[derive(sqlx::FromRow)]
struct DirectoryRow {
    profile_id: Uuid,
    handle: String,
    display_name: String,
    standing: String,
    standing_updated: Option<DateTime<Utc>>,
    is_system_admin: bool,
    email: Option<String>,
    provisioned_via: Option<String>,
    team_count: i64,
    has_pending_request: bool,
    matched_email: Option<String>,
    total: i64,
}

impl DirectoryRow {
    fn into_entry(self) -> AdminDirectoryEntry {
        AdminDirectoryEntry {
            profile_id: self.profile_id,
            handle: self.handle,
            display_name: self.display_name,
            standing: self.standing,
            standing_updated: self.standing_updated,
            is_system_admin: self.is_system_admin,
            email: self.email,
            provisioned_via: self.provisioned_via,
            team_count: self.team_count,
            has_pending_request: self.has_pending_request,
            matched_email: self.matched_email,
        }
    }
}

/// The directory list (spec §5). Human principals, filtered, paged, `total` included so a
/// paging agent is never silently incomplete. Sort is `created DESC`, fixed in v1.
///
/// The `?email=` parameter is NOT handled here — it changes the response SHAPE (a card, not a
/// page), so the surface dispatches to [`profile_card_by_email`] before calling this.
pub async fn list_profiles(
    pool: &PgPool,
    admin: &SystemAdmin,
    query: &AdminProfilesListQuery,
) -> ApiResult<AdminDirectoryListResponse> {
    let _ = admin.actor(); // the proof IS the check; a read records no actor

    let filter = StandingFilter::parse(query.standing.as_deref())?;
    // Literal matching by construction: `position()` takes a plain substring, so `%`, `_` and
    // `\` carry no wildcard meaning and there is no escape step that could drift.
    let needle = query
        .email_contains
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
    // A team filter arrives as slug OR uuid; resolve which in Rust and bind exactly one.
    let team_id = query
        .team
        .as_deref()
        .and_then(|t| Uuid::parse_str(t.trim()).ok());
    let team_slug = match (&query.team, team_id) {
        (Some(t), None) => Some(t.trim().to_string()),
        _ => None,
    };

    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = query.offset.unwrap_or(0).clamp(0, MAX_OFFSET);

    let rows = fetch_directory_page(
        pool,
        needle.as_deref(),
        &filter,
        team_slug.as_deref(),
        team_id,
        limit,
        offset,
    )
    .await?;

    // The window count rides every returned row. An empty page (offset beyond the end) returns
    // no rows and therefore no count — reread the SAME statement at its first row rather than
    // restating the predicate in a second query, which is exactly the drift the two copies
    // would eventually make. The recount's ROWS are discarded: the page stays empty.
    let total = match rows.first() {
        Some(row) => row.total,
        None => {
            let recount = fetch_directory_page(
                pool,
                needle.as_deref(),
                &filter,
                team_slug.as_deref(),
                team_id,
                1,
                0,
            )
            .await?;
            recount.first().map(|r| r.total).unwrap_or(0)
        }
    };

    Ok(AdminDirectoryListResponse {
        entries: rows.into_iter().map(DirectoryRow::into_entry).collect(),
        total,
    })
}

/// The list statement, parameterized so the page read and the empty-page recount run
/// byte-identical predicates.
#[allow(clippy::too_many_arguments)]
async fn fetch_directory_page(
    pool: &PgPool,
    needle: Option<&str>,
    filter: &StandingFilter,
    team_slug: Option<&str>,
    team_id: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> ApiResult<Vec<DirectoryRow>> {
    let rows = sqlx::query_as!(
        DirectoryRow,
        r#"
        SELECT p.id                    AS "profile_id!",
               p.handle                AS "handle!",
               p.display_name          AS "display_name!",
               COALESCE(s.state, 'denied') AS "standing!",
               s.updated               AS "standing_updated?",
               is_system_admin(p.id)   AS "is_system_admin!",
               d.email                 AS "email",
               COALESCE(d.auth_provider, f.auth_provider) AS "provisioned_via?",
               (SELECT count(*) FROM kb_team_members tm WHERE tm.profile_id = p.id)
                                       AS "team_count!",
               EXISTS (SELECT 1 FROM kb_join_requests jr
                       WHERE jr.requesting_profile_id = p.id AND jr.status = 'pending')
                                       AS "has_pending_request!",
               m.matched_email         AS "matched_email",
               COUNT(*) OVER ()        AS "total!"
        FROM kb_profiles p
        LEFT JOIN kb_principal_standing s ON s.profile_id = p.id
        LEFT JOIN LATERAL (
            -- §6 fallback, steps 1+2: best verified link (default first, then earliest).
            SELECT al.email, al.auth_provider
            FROM kb_profile_auth_links al
            WHERE al.profile_id = p.id AND al.email_verified AND al.email IS NOT NULL
            ORDER BY al.is_default DESC, al.linked_at ASC
            LIMIT 1
        ) d ON true
        LEFT JOIN LATERAL (
            -- §6 fallback, step 3: the earliest link overall, provider only.
            SELECT al.auth_provider
            FROM kb_profile_auth_links al
            WHERE al.profile_id = p.id
            ORDER BY al.linked_at ASC
            LIMIT 1
        ) f ON d.email IS NULL
        LEFT JOIN LATERAL (
            SELECT al.email AS matched_email
            FROM kb_profile_auth_links al
            WHERE al.profile_id = p.id AND al.email_verified AND al.email IS NOT NULL
              AND position($1 IN lower(al.email)) > 0
            ORDER BY al.linked_at ASC
            LIMIT 1
        ) m ON $1::text IS NOT NULL
        WHERE EXISTS (
            -- §4, the pinned human discriminator: >=1 non-machine auth link.
            SELECT 1 FROM kb_profile_auth_links al
            WHERE al.profile_id = p.id AND al.auth_provider <> 'auth0-m2m'
        )
        AND (
            ($2 AND s.state IS DISTINCT FROM 'approved')
            OR ($3::text IS NOT NULL AND COALESCE(s.state, 'denied') = $3)
            OR (NOT $2 AND $3::text IS NULL)
        )
        AND (
            ($4::text IS NULL AND $5::uuid IS NULL)
            OR EXISTS (
                SELECT 1 FROM kb_team_members tm JOIN kb_teams t ON t.id = tm.team_id
                WHERE tm.profile_id = p.id
                  AND (($5::uuid IS NOT NULL AND t.id = $5)
                       OR ($4::text IS NOT NULL AND t.slug = $4))
            )
        )
        AND ($1::text IS NULL OR m.matched_email IS NOT NULL)
        ORDER BY p.created DESC
        LIMIT $6 OFFSET $7
        "#,
        needle,
        matches!(filter, StandingFilter::NeedsAccess),
        match filter {
            StandingFilter::State(state) => Some(*state),
            _ => None,
        },
        team_slug,
        team_id,
        limit,
        offset,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// The principal state card (spec §6) by profile UUID. Reads existing tables only; composes
/// the §6 default-link fallback in Rust from the same link rows the card renders.
pub async fn profile_card(
    pool: &PgPool,
    admin: &SystemAdmin,
    profile_id: ProfileId,
) -> ApiResult<AdminProfileCard> {
    let _ = admin.actor();

    let identity = sqlx::query!(
        r#"
        SELECT p.handle          AS "handle!",
               p.display_name    AS "display_name!",
               COALESCE(s.state, 'denied') AS "standing!",
               s.updated    AS "standing_updated?"
        FROM kb_profiles p
        LEFT JOIN kb_principal_standing s ON s.profile_id = p.id
        WHERE p.id = $1
        "#,
        *profile_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("profile {profile_id} not found")))?;

    let links = sqlx::query!(
        r#"
        SELECT auth_provider     AS "auth_provider!",
               email             AS "email",
               email_verified    AS "email_verified!",
               is_default        AS "is_default!",
               linked_at         AS "linked_at!"
        FROM kb_profile_auth_links
        WHERE profile_id = $1
        ORDER BY linked_at ASC
        "#,
        *profile_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| AdminProfileAuthLink {
        auth_provider: row.auth_provider,
        email: row.email,
        email_verified: row.email_verified,
        is_default: row.is_default,
        linked_at: row.linked_at,
    })
    .collect::<Vec<_>>();

    let is_system_admin = sqlx::query_scalar!(
        "SELECT is_system_admin($1) AS \"is_system_admin!\"",
        *profile_id,
    )
    .fetch_one(pool)
    .await?;

    let teams = sqlx::query!(
        r#"
        SELECT t.slug        AS "team_slug!",
               tm.role::text AS "role!"
        FROM kb_team_members tm
        JOIN kb_teams t ON t.id = tm.team_id
        WHERE tm.profile_id = $1
        ORDER BY t.slug
        "#,
        *profile_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| AdminProfileTeamMembership {
        team_slug: row.team_slug,
        role: row.role,
    })
    .collect();

    // Token-free by projection: the view's `token` column is never named, and the view itself
    // carries the uniqueness-attribution rule (an address verified-owned by two or more
    // profiles yields NO invitee row) — reused, not restated.
    let pending_invitations = sqlx::query!(
        r#"
        SELECT i.id                   AS "id!",
               i.team_slug            AS "team_slug!",
               i.role::text           AS "role!",
               i.invited_by_profile_id AS "invited_by_profile_id!",
               i.created              AS "created!",
               i.expires_at           AS "expires_at!"
        FROM vw_invitee_invitations i
        WHERE i.invitee_profile_id = $1
        ORDER BY i.created DESC
        "#,
        *profile_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| AdminProfileInvitation {
        id: row.id,
        team_slug: row.team_slug,
        role: row.role,
        invited_by_profile_id: row.invited_by_profile_id,
        created: row.created,
        expires_at: row.expires_at,
    })
    .collect();

    let open_join_request = sqlx::query!(
        r#"
        SELECT id    AS "id!",
               created AS "created!",
               message AS "message"
        FROM kb_join_requests
        WHERE requesting_profile_id = $1 AND status = 'pending'
        ORDER BY created DESC
        LIMIT 1
        "#,
        *profile_id,
    )
    .fetch_optional(pool)
    .await?
    .map(|row| AdminOpenJoinRequest {
        id: row.id,
        created: row.created,
        message: row.message,
    });

    let open_reconsideration = sqlx::query!(
        r#"
        SELECT id      AS "id!",
               created AS "created!"
        FROM kb_principal_review_requests
        WHERE profile_id = $1 AND decided_at IS NULL
        ORDER BY created DESC
        LIMIT 1
        "#,
        *profile_id,
    )
    .fetch_optional(pool)
    .await?
    .map(|row| AdminOpenReviewRequest {
        id: row.id,
        created: row.created,
    });

    let (standing, standing_updated) =
        standing_wire(Some(identity.standing), identity.standing_updated);
    let (email, provisioned_via) = derive_default_identity(&links);

    // Hints, not doors (spec §6): existing commands, printed as text. They add no mutation
    // path — each names an act that already exists.
    let mut hints = Vec::new();
    if standing != Standing::Approved.as_str() {
        hints.push(format!("temper admin access approve {profile_id}"));
    }
    if let Some(request) = &open_join_request {
        hints.push(format!(
            "temper admin requests review {} --approve",
            request.id
        ));
    }

    Ok(AdminProfileCard {
        profile_id: *profile_id,
        handle: identity.handle,
        display_name: identity.display_name,
        standing,
        standing_updated,
        is_system_admin,
        auth_links: links,
        email,
        provisioned_via,
        teams,
        pending_invitations,
        open_join_request,
        open_reconsideration,
        hints,
    })
}

/// The state card resolved through an EXACT, case-insensitive verified-email match (spec §6's
/// route contract — the single point where a human-controlled address becomes a target UUID).
///
/// Zero matches → 404. More than one profile verified-owns the address → 404 whose BODY NAMES
/// the collision: the system refuses to pick, and the operator disambiguates by UUID. A
/// substring/lookalike address can never win here — there is no partial match to win with.
pub async fn profile_card_by_email(
    pool: &PgPool,
    admin: &SystemAdmin,
    email: &str,
) -> ApiResult<AdminProfileCard> {
    let _ = admin.actor();

    let trimmed = email.trim();
    if trimmed.is_empty() {
        return Err(ApiError::BadRequest("email must not be empty".to_owned()));
    }

    let matches = sqlx::query!(
        r#"
        SELECT DISTINCT al.profile_id AS "profile_id!"
        FROM kb_profile_auth_links al
        WHERE al.email_verified AND al.email IS NOT NULL AND lower(al.email) = lower($1)
        "#,
        trimmed,
    )
    .fetch_all(pool)
    .await?;

    match matches.as_slice() {
        [] => Err(ApiError::NotFound(format!(
            "no profile holds '{trimmed}' as a verified email"
        ))),
        [one] => profile_card(pool, admin, ProfileId::from(one.profile_id)).await,
        many => {
            let ids = many
                .iter()
                .map(|m| m.profile_id.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Err(ApiError::NotFound(format!(
                "'{trimmed}' is verified-owned by {} profiles ({ids}); refusing to pick — \
                 show one by UUID instead",
                many.len()
            )))
        }
    }
}

#[cfg(test)]
mod gate_tripwire {
    /// Spec §7 (review finding L1): the F-3 pattern only holds for functions that DEMAND the
    /// proof. This tripwire reads the module source and fails if any `pub fn` here can be
    /// called without a sealed `&SystemAdmin` — a new function added without the proof fails
    /// this test, not a review.
    #[test]
    fn every_public_fn_demands_the_sealed_admin_proof() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/services/admin_directory_service.rs"
        );
        let source = std::fs::read_to_string(path).expect("directory module source");
        // Signatures span lines; accumulate from the `pub fn` marker to the body/closing brace
        // and judge the WHOLE signature, not its first line.
        let mut offenders: Vec<String> = Vec::new();
        let mut current: Option<String> = None;
        for line in source.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("pub fn ") || trimmed.starts_with("pub async fn ") {
                current = Some(trimmed.to_owned());
            } else if let Some(sig) = current.as_mut() {
                sig.push(' ');
                sig.push_str(trimmed);
            }
            if let Some(sig) = &current {
                let complete = trimmed.starts_with('{')
                    || trimmed.ends_with('{')
                    || trimmed.ends_with(';')
                    || trimmed.contains(") ->");
                if complete {
                    if !sig.contains("&SystemAdmin") {
                        offenders.push(sig.clone());
                    }
                    current = None;
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "directory service fn(s) taking no sealed &SystemAdmin proof: {offenders:#?}"
        );
    }
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use temper_core::types::machine::{ProvisionMachineRequest, RebindMachineRequest};

    use crate::services::machine_registration_service;

    // ── fixtures ─────────────────────────────────────────────────────────────

    /// Seed a HUMAN profile: an auth link under a non-machine provider, verified unless
    /// `verified: false`. No standing row unless one is added.
    async fn human(pool: &PgPool, handle: &str, email: &str, verified: bool) -> (ProfileId, Uuid) {
        let profile_id = Uuid::now_v7();
        sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)")
            .bind(profile_id)
            .bind(handle)
            .execute(pool)
            .await
            .expect("seed human profile");
        let link_id = auth_link(pool, profile_id, "saml:okta", email, verified, true).await;
        (ProfileId::from(profile_id), link_id)
    }

    async fn auth_link(
        pool: &PgPool,
        profile_id: Uuid,
        provider: &str,
        email: &str,
        verified: bool,
        is_default: bool,
    ) -> Uuid {
        let link_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO kb_profile_auth_links \
             (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, \
              is_default) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(link_id)
        .bind(profile_id)
        .bind(provider)
        .bind(format!("{provider}-{}", link_id.simple()))
        .bind(email)
        .bind(verified)
        .bind(is_default)
        .execute(pool)
        .await
        .expect("seed auth link");
        link_id
    }

    /// Seed a standing row the way the transition functions leave it (the fixture wants the
    /// end state, not the append-only history — CONFORM `test_support`'s own rationale).
    async fn standing(pool: &PgPool, profile_id: Uuid, state: &str) {
        sqlx::query!(
            "INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, $2) \
             ON CONFLICT (profile_id) DO UPDATE SET state = $2",
            profile_id,
            state,
        )
        .execute(pool)
        .await
        .expect("seed standing");
    }

    async fn governance(pool: &PgPool, profile_id: Uuid) {
        sqlx::query!(
            "INSERT INTO kb_principal_governance (profile_id) VALUES ($1) \
             ON CONFLICT (profile_id) DO NOTHING",
            profile_id,
        )
        .execute(pool)
        .await
        .expect("seed governance");
    }

    async fn team(pool: &PgPool, slug: &str) -> Uuid {
        sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
            .bind(slug)
            .fetch_one(pool)
            .await
            .expect("seed team")
    }

    async fn membership(pool: &PgPool, team_id: Uuid, profile_id: Uuid, role: &str) {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
                     VALUES ($1, $2, $3::team_role)",
        )
        .bind(team_id)
        .bind(profile_id)
        .bind(role)
        .execute(pool)
        .await
        .expect("seed membership");
    }

    async fn admin(pool: &PgPool) -> SystemAdmin {
        crate::test_support::system_admin_proof(pool).await
    }

    fn query(standing: Option<&str>) -> AdminProfilesListQuery {
        AdminProfilesListQuery {
            standing: standing.map(str::to_owned),
            ..Default::default()
        }
    }

    // ── §4 the discriminator pinned against both machine mint doors ──────────

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provisioned_and_rebound_machines_never_appear_but_a_saml_human_does(pool: PgPool) {
        let admin = admin(&pool).await;

        // Mint door 1: a provisioned machine, through the REAL registration service.
        let provisioned = machine_registration_service::provision(
            &pool,
            admin.actor(),
            &ProvisionMachineRequest {
                client_id: format!("m2m-provisioned-{}", Uuid::now_v7()),
                label: "door one".to_owned(),
                owner_team_id: None,
                teams: vec![],
                grants: vec![],
            },
        )
        .await
        .expect("provision machine");

        // Mint door 2: a rebound machine — a SECOND auth0-m2m link on a fresh profile.
        let rebound = machine_registration_service::provision(
            &pool,
            admin.actor(),
            &ProvisionMachineRequest {
                client_id: format!("m2m-rebind-src-{}", Uuid::now_v7()),
                label: "rebind source".to_owned(),
                owner_team_id: None,
                teams: vec![],
                grants: vec![],
            },
        )
        .await
        .expect("provision rebind source");
        machine_registration_service::rebind(
            &pool,
            &admin,
            &RebindMachineRequest {
                client_id: format!("m2m-rebound-{}", Uuid::now_v7()),
                from_machine_client_id: rebound.id,
                label: "rebound".to_owned(),
                keep_old_active: false,
            },
        )
        .await
        .expect("rebind machine");

        // The human the directory exists for: SAML-provisioned, never requested anything.
        let (human_id, _) = human(&pool, "saml-alice", "alice@corp.example", true).await;

        let page = list_profiles(&pool, &admin, &query(None))
            .await
            .expect("list");
        let listed: Vec<Uuid> = page.entries.iter().map(|e| e.profile_id).collect();
        assert!(listed.contains(&*human_id), "SAML human must appear");
        assert!(
            !listed.contains(&provisioned.profile_id),
            "provisioned machine leaked into the human directory"
        );
        assert_eq!(
            listed.iter().filter(|id| **id == *human_id).count(),
            1,
            "exactly one row for the human"
        );
        // The rebound machine shares the SOURCE profile (rebind adds a second m2m link to the
        // SAME profile), so the same-never-appears assertion covers both links.
        assert_eq!(
            page.total, 1,
            "the operator's own machine-minted profiles and any seeded humans except alice are absent"
        );
    }

    // ── §5 the list affordance ───────────────────────────────────────────────

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn human_with_no_standing_row_appears_in_default_view_as_denied(pool: PgPool) {
        let admin = admin(&pool).await;
        let (never_requested, _) = human(&pool, "never-requested", "nr@corp.example", true).await;
        let (approved, _) = human(&pool, "approved-one", "ap@corp.example", true).await;
        standing(&pool, *approved, "approved").await;

        // Default view: needs-access.
        let page = list_profiles(&pool, &admin, &query(None))
            .await
            .expect("list");
        assert_eq!(
            page.total, 1,
            "only the never-requested human is needs-access"
        );
        let entry = &page.entries[0];
        assert_eq!(entry.profile_id, *never_requested);
        assert_eq!(entry.standing, "denied", "absence renders as denied");
        assert_eq!(entry.standing_updated, None);

        // The approved principal is findable only under its own state or `all`.
        let all = list_profiles(&pool, &admin, &query(Some("all")))
            .await
            .expect("list all");
        assert_eq!(all.total, 2);
        let approved_only = list_profiles(&pool, &admin, &query(Some("approved")))
            .await
            .expect("list approved");
        assert_eq!(approved_only.total, 1);
        assert_eq!(approved_only.entries[0].profile_id, *approved);
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn needs_access_covers_revoked_and_deactivated_too(pool: PgPool) {
        let admin = admin(&pool).await;
        let (revoked, _) = human(&pool, "revoked-one", "rv@corp.example", true).await;
        standing(&pool, *revoked, "revoked").await;
        let (deactivated, _) = human(&pool, "deactivated-one", "da@corp.example", true).await;
        standing(&pool, *deactivated, "deactivated").await;

        let page = list_profiles(&pool, &admin, &query(None))
            .await
            .expect("list");
        assert_eq!(page.total, 2, "revoked and deactivated both lack access");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn unknown_standing_filter_is_bad_request(pool: PgPool) {
        let admin = admin(&pool).await;
        let err = list_profiles(&pool, &admin, &query(Some("unapproved")))
            .await
            .expect_err("unapproved is the retired name, not a filter");
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn email_contains_matches_literals_and_returns_matched_email(pool: PgPool) {
        let admin = admin(&pool).await;
        let (hit, _) = human(
            &pool,
            "literal-hit",
            "alice.under_score%tag@corp.example",
            true,
        )
        .await;
        let (_miss, _) = human(&pool, "literal-miss", "bob@corp.example", true).await;

        // A needle containing all three LIKE metacharacters must match LITERALLY.
        let mut q = query(None);
        q.email_contains = Some("under_score%tag".to_owned());
        let page = list_profiles(&pool, &admin, &q).await.expect("contains");
        assert_eq!(page.total, 1, "% and _ must not act as wildcards");
        assert_eq!(page.entries[0].profile_id, *hit);
        assert_eq!(
            page.entries[0].matched_email.as_deref(),
            Some("alice.under_score%tag@corp.example"),
            "the matched address rides the row"
        );

        // Case-insensitive, over verified emails.
        q.email_contains = Some("ALICE".to_owned());
        let page = list_profiles(&pool, &admin, &q).await.expect("contains");
        assert_eq!(page.total, 1);

        // With no filter active, matched_email stays off the rows.
        let plain = list_profiles(&pool, &admin, &query(Some("all")))
            .await
            .expect("plain");
        assert!(plain.entries.iter().all(|e| e.matched_email.is_none()));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn email_contains_never_matches_an_unverified_email(pool: PgPool) {
        let admin = admin(&pool).await;
        let (_p, _) = human(&pool, "unverified", "secret@corp.example", false).await;

        let mut q = query(None);
        q.email_contains = Some("secret".to_owned());
        let page = list_profiles(&pool, &admin, &q).await.expect("contains");
        assert_eq!(page.total, 0, "unverified addresses are not enumerated");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn team_filter_accepts_slug_and_uuid(pool: PgPool) {
        let admin = admin(&pool).await;
        let (member, _) = human(&pool, "team-member", "tm@corp.example", true).await;
        let (outsider, _) = human(&pool, "team-outsider", "to@corp.example", true).await;
        let team_id = team(&pool, "platform").await;
        membership(&pool, team_id, *member, "member").await;
        let _ = outsider;

        let mut q = query(Some("all"));
        q.team = Some("platform".to_owned());
        let page = list_profiles(&pool, &admin, &q).await.expect("by slug");
        assert_eq!(page.total, 1);
        assert_eq!(page.entries[0].profile_id, *member);

        q.team = Some(team_id.to_string());
        let page = list_profiles(&pool, &admin, &q).await.expect("by uuid");
        assert_eq!(page.total, 1);
        assert_eq!(page.entries[0].profile_id, *member);
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn list_rows_carry_row_shape_and_clamps_apply(pool: PgPool) {
        let admin = admin(&pool).await;
        let (a, _) = human(&pool, "row-a", "row-a@corp.example", true).await;
        let (b, _) = human(&pool, "row-b", "row-b@corp.example", true).await;
        let t = team(&pool, "sized").await;
        membership(&pool, t, *a, "member").await;
        standing(&pool, *b, "requested").await;
        governance(&pool, *b).await;
        sqlx::query(
            "INSERT INTO kb_join_requests (id, team_id, requesting_profile_id, status, source) \
             VALUES ($1, $2, $3, 'pending', 'web')",
        )
        .bind(Uuid::now_v7())
        .bind(t)
        .bind(*b)
        .execute(&pool)
        .await
        .expect("seed join request");

        // limit=1 → one row, total still counts both.
        let mut q = query(None);
        q.limit = Some(1);
        let page = list_profiles(&pool, &admin, &q).await.expect("page");
        assert_eq!(page.entries.len(), 1);
        assert_eq!(
            page.total, 2,
            "total is the filtered population, not the page"
        );

        // Every row carries the full §5 shape.
        let all = list_profiles(&pool, &admin, &query(Some("all")))
            .await
            .expect("all");
        for entry in &all.entries {
            assert!(!entry.handle.is_empty());
            assert!(!entry.display_name.is_empty());
            assert!(entry.team_count >= 0);
        }
        let row_b = all
            .entries
            .iter()
            .find(|e| e.profile_id == *b)
            .expect("row b");
        assert_eq!(row_b.standing, "requested");
        assert!(row_b.is_system_admin);
        assert!(row_b.has_pending_request);
        assert_eq!(row_b.email.as_deref(), Some("row-b@corp.example"));
        assert_eq!(row_b.provisioned_via.as_deref(), Some("saml:okta"));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn list_clamps_limit_and_offset(pool: PgPool) {
        let admin = admin(&pool).await;
        let (_only, _) = human(&pool, "clamp-one", "clamp@corp.example", true).await;

        // limit above the cap clamps down (returns everything there is, no error).
        let mut q = query(None);
        q.limit = Some(10_000);
        let page = list_profiles(&pool, &admin, &q).await.expect("clamp limit");
        assert_eq!(page.entries.len(), 1);

        // limit below 1 clamps up to 1.
        q.limit = Some(0);
        let page = list_profiles(&pool, &admin, &q).await.expect("floor limit");
        assert_eq!(page.entries.len(), 1);

        // Offset beyond the population: empty page, total still honest.
        q.limit = None;
        q.offset = Some(5_000);
        let page = list_profiles(&pool, &admin, &q).await.expect("deep offset");
        assert!(page.entries.is_empty());
        assert_eq!(page.total, 1);

        // Negative offset floors at 0.
        q.offset = Some(-50);
        let page = list_profiles(&pool, &admin, &q)
            .await
            .expect("negative offset");
        assert_eq!(page.entries.len(), 1);

        // Offset above the 10_000 cap clamps down to the cap.
        q.offset = Some(99_999);
        let page = list_profiles(&pool, &admin, &q)
            .await
            .expect("capped offset");
        assert!(page.entries.is_empty());
        assert_eq!(page.total, 1);
    }

    // ── §6 the card ──────────────────────────────────────────────────────────

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn card_composes_identity_admission_governance_and_queue_state(pool: PgPool) {
        let admin = admin(&pool).await;
        let (pid, _) = human(&pool, "card-one", "card@corp.example", true).await;
        let t = team(&pool, "carders").await;
        membership(&pool, t, *pid, "maintainer").await;
        let request_id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_join_requests (id, team_id, requesting_profile_id, status, message, source) \
             VALUES ($1, $2, $3, 'pending', 'let me in', 'web') RETURNING id",
        )
        .bind(Uuid::now_v7())
        .bind(t)
        .bind(*pid)
        .fetch_one(&pool)
        .await
        .expect("seed join request");
        let review_id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_principal_review_requests (id, profile_id, message) \
             VALUES ($1, $2, 'second look') RETURNING id",
        )
        .bind(Uuid::now_v7())
        .bind(*pid)
        .fetch_one(&pool)
        .await
        .expect("seed review request");

        let card = profile_card(&pool, &admin, pid).await.expect("card");
        assert_eq!(card.handle, "card-one");
        assert_eq!(card.standing, "denied", "no standing row renders denied");
        assert!(card.standing_updated.is_none());
        assert!(!card.is_system_admin);
        assert_eq!(card.email.as_deref(), Some("card@corp.example"));
        assert_eq!(card.provisioned_via.as_deref(), Some("saml:okta"));
        // The profile ALSO holds its trigger-minted `personal-<handle>` team; assert on the
        // seeded membership rather than the count.
        let carders = card
            .teams
            .iter()
            .find(|t| t.team_slug == "carders")
            .expect("seeded membership");
        assert_eq!(carders.role, "maintainer");
        let request = card.open_join_request.expect("open join request");
        assert_eq!(request.id, request_id);
        assert_eq!(request.message.as_deref(), Some("let me in"));
        let review = card.open_reconsideration.expect("open reconsideration");
        assert_eq!(review.id, review_id);
        // Hints, not doors: the enablement commands, copy-runnable.
        assert!(card
            .hints
            .iter()
            .any(|h| h.starts_with("temper admin access approve ")));
        assert!(card
            .hints
            .iter()
            .any(|h| h.contains("admin requests review")));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn card_is_404_for_absent_profile(pool: PgPool) {
        let admin = admin(&pool).await;
        let err = profile_card(&pool, &admin, ProfileId::from(Uuid::now_v7()))
            .await
            .expect_err("absent");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn card_default_email_follows_the_pinned_fallback_rule(pool: PgPool) {
        let admin = admin(&pool).await;
        let (pid, _) = human(&pool, "fallback", "stale@corp.example", true).await;

        // A NEWER default verified link displaces the saml link from the primary slot.
        sqlx::query("UPDATE kb_profile_auth_links SET is_default = false WHERE profile_id = $1")
            .bind(*pid)
            .execute(&pool)
            .await
            .expect("clear old default");
        auth_link(&pool, *pid, "google", "default@corp.example", true, true).await;
        let card = profile_card(&pool, &admin, pid).await.expect("card");
        assert_eq!(card.email.as_deref(), Some("default@corp.example"));
        assert_eq!(card.provisioned_via.as_deref(), Some("google"));

        // All links unverified → email withheld, provenance still named (step 3).
        let (uvid, _) = human(&pool, "unverified-card", "uv@corp.example", false).await;
        let card = profile_card(&pool, &admin, uvid).await.expect("card");
        assert_eq!(
            card.email, None,
            "an unverified email never renders unmarked"
        );
        assert_eq!(card.provisioned_via.as_deref(), Some("saml:okta"));

        // Default link exists but is UNVERIFIED → falls to the earliest VERIFIED link.
        let (mixed, _) = human(&pool, "mixed", "verified-first@corp.example", true).await;
        auth_link(
            &pool,
            *mixed,
            "enterprise",
            "unverified-default@corp.example",
            false,
            true,
        )
        .await;
        let card = profile_card(&pool, &admin, mixed).await.expect("card");
        assert_eq!(card.email.as_deref(), Some("verified-first@corp.example"));
        assert_eq!(card.provisioned_via.as_deref(), Some("saml:okta"));
        // The unverified address still shows on the links list — unmarked, not hidden.
        assert!(card.auth_links.iter().any(|l| l.email.as_deref()
            == Some("unverified-default@corp.example")
            && !l.email_verified));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn card_carries_pending_invitations_without_the_token(pool: PgPool) {
        let admin = admin(&pool).await;
        let (inviter, _) = human(&pool, "inviter", "inviter@corp.example", true).await;
        governance(&pool, *inviter).await; // invitations need an inviter profile only
        let (invitee, _) = human(&pool, "invitee", "invitee@corp.example", true).await;
        let t = team(&pool, "invited").await;
        let token = format!("tok-{}", Uuid::now_v7());
        sqlx::query(
            "INSERT INTO kb_team_invitations \
             (id, team_id, invited_email, invited_by_profile_id, role, token, status, expires_at) \
             VALUES ($1, $2, 'invitee@corp.example', $3, 'member', $4, 'pending', \
                     now() + interval '7 days')",
        )
        .bind(Uuid::now_v7())
        .bind(t)
        .bind(*inviter)
        .bind(&token)
        .execute(&pool)
        .await
        .expect("seed invitation");

        let card = profile_card(&pool, &admin, invitee).await.expect("card");
        assert_eq!(card.pending_invitations.len(), 1);
        assert_eq!(card.pending_invitations[0].team_slug, "invited");
        assert_eq!(card.pending_invitations[0].role, "member");

        // The pinned rule, asserted over the whole serialized card: no redemption token.
        let json = serde_json::to_string(&card).expect("serialize");
        assert!(
            !json.contains(&token),
            "the bearer token leaked into the card"
        );
        assert!(
            !json.contains("token"),
            "no token-bearing field may exist on the card wire shape"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn invitation_shared_by_two_verified_profiles_attributed_to_neither(pool: PgPool) {
        let admin = admin(&pool).await;
        let (inviter, _) = human(&pool, "shared-inviter", "si@corp.example", true).await;
        let (first, _) = human(&pool, "shared-a", "shared@corp.example", true).await;
        let (second, _) = human(&pool, "shared-b", "shared@corp.example", true).await;
        let t = team(&pool, "shared").await;
        sqlx::query(
            "INSERT INTO kb_team_invitations \
             (id, team_id, invited_email, invited_by_profile_id, role, token) \
             VALUES ($1, $2, 'shared@corp.example', $3, 'member', $4)",
        )
        .bind(Uuid::now_v7())
        .bind(t)
        .bind(*inviter)
        .bind(format!("shared-{}", Uuid::now_v7()))
        .execute(&pool)
        .await
        .expect("seed shared invitation");

        for profile in [*first, *second] {
            let card = profile_card(&pool, &admin, ProfileId::from(profile))
                .await
                .expect("card");
            assert!(
                card.pending_invitations.is_empty(),
                "a shared-address invitation must attribute to neither card"
            );
        }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn email_resolution_exact_case_insensitive_and_ambiguity_refusing(pool: PgPool) {
        let admin = admin(&pool).await;
        let (pid, _) = human(&pool, "exact-one", "Case.Exact@corp.example", true).await;

        // Exact, case-insensitive → the card.
        let card = profile_card_by_email(&pool, &admin, "case.exact@CORP.example")
            .await
            .expect("exact resolution");
        assert_eq!(card.profile_id, *pid);

        // Zero matches → 404.
        let err = profile_card_by_email(&pool, &admin, "nobody@corp.example")
            .await
            .expect_err("absent");
        assert!(matches!(err, ApiError::NotFound(_)));

        // The lookalike hazard (review C1): a verified `…corp.example.evil.io` address is a
        // DIFFERENT identity. Querying the original must still resolve to the original —
        // substring resolution could never guarantee that.
        let (lookalike, _) =
            human(&pool, "lookalike", "case.exact@corp.example.evil.io", true).await;
        let card = profile_card_by_email(&pool, &admin, "case.exact@CORP.example")
            .await
            .expect("the original address still resolves exactly");
        assert_eq!(card.profile_id, *pid);
        assert_ne!(card.profile_id, *lookalike);

        // More than one verified owner → 404 whose body NAMES the collision.
        let (_dup, _) = human(&pool, "exact-two", "case.exact@corp.example", true).await;
        let err = profile_card_by_email(&pool, &admin, "case.exact@corp.example")
            .await
            .expect_err("ambiguous");
        match err {
            ApiError::NotFound(message) => {
                assert!(
                    message.contains("2 profiles"),
                    "body must name the collision count: {message}"
                );
                assert!(
                    message.contains(pid.to_string().as_str()),
                    "body must name the colliding ids: {message}"
                );
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }
}
