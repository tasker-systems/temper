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
//! least one `kb_profile_auth_links` row with a provider other than the mint doors'
//! [`MACHINE_PROVIDER_TAG`] (bound into the query from that constant, not restated). Machine agents
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

use crate::auth::{SystemAdmin, MACHINE_PROVIDER_TAG};
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
        DirectoryPageQuery {
            needle: needle.as_deref(),
            filter: &filter,
            team_slug: team_slug.as_deref(),
            team_id,
            limit,
            offset,
        },
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
                DirectoryPageQuery {
                    needle: needle.as_deref(),
                    filter: &filter,
                    team_slug: team_slug.as_deref(),
                    team_id,
                    limit: 1,
                    offset: 0,
                },
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

/// The filters one admin-directory page read runs under. Both the page read and the empty-page
/// recount use the same statement, so the recount clones this with `limit: 1, offset: 0`.
struct DirectoryPageQuery<'a> {
    needle: Option<&'a str>,
    filter: &'a StandingFilter,
    team_slug: Option<&'a str>,
    team_id: Option<Uuid>,
    limit: i64,
    offset: i64,
}

/// The list statement, parameterized so the page read and the empty-page recount run
/// byte-identical predicates.
async fn fetch_directory_page(
    pool: &PgPool,
    page: DirectoryPageQuery<'_>,
) -> ApiResult<Vec<DirectoryRow>> {
    let DirectoryPageQuery {
        needle,
        filter,
        team_slug,
        team_id,
        limit,
        offset,
    } = page;
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
            -- §4, the pinned human discriminator: >=1 non-machine auth link. The provider tag
            -- is BOUND, not inlined: it is the same constant the machine mint doors write, so
            -- this denylist cannot drift from the doors that create the rows it excludes.
            SELECT 1 FROM kb_profile_auth_links al
            WHERE al.profile_id = p.id AND al.auth_provider <> $8
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
        ORDER BY p.created DESC, p.id DESC
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
        MACHINE_PROVIDER_TAG,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// The principal state card (spec §6) by profile UUID. Reads existing tables only; composes
/// the §6 default-link fallback in Rust from the same link rows the card renders.
///
/// **This door applies NO human discriminator, deliberately**: the caller already holds the
/// target UUID, and an admin inspecting a known machine agent (or a connection profile's
/// owner) gets the same composed view as for a human — machine links render, but a machine
/// holds no secrets here and the act is sealed-gated. The DISCOVERY surface (the list) is
/// where the discriminator binds; the card is addressability, not enumeration.
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

    // Absence (no standing row) renders `denied` via COALESCE, with `standing_updated` None
    // exactly when no row exists — that pair is the absence discriminator. Approve refuses
    // from true absence AND from deactivated, so neither class is advertised approve.
    let standing_absent =
        identity.standing_updated.is_none() && identity.standing == Standing::Denied.as_str();
    let (standing, standing_updated) =
        standing_wire(Some(identity.standing), identity.standing_updated);
    let (email, provisioned_via) = derive_default_identity(&links);

    // Hints, not doors (spec §6): existing commands, printed as text. They add no mutation
    // path — each names an act that already exists — and each is MACHINE-LEGAL for the class
    // it targets, so the card never advertises a command that would refuse: approve applies
    // from denied/requested/revoked only (standing-row ABSENCE and deactivated both refuse it,
    // temper-principal/src/transition.rs); deactivated's door is reactivate (restoring the
    // prior standing); a never-signed-in principal has no legal admin act at all until their
    // first sign-in provisions them, so the card advertises nothing rather than a 400.
    let mut hints = Vec::new();
    if !standing_absent {
        match standing.as_str() {
            s if s == Standing::Denied.as_str()
                || s == Standing::Requested.as_str()
                || s == Standing::Revoked.as_str() =>
            {
                hints.push(format!("temper admin access approve {profile_id}"));
            }
            s if s == Standing::Deactivated.as_str() => {
                hints.push(format!("temper admin access reactivate {profile_id}"));
            }
            _ => {}
        }
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
        WHERE al.email_verified AND al.email IS NOT NULL
          AND lower(al.email COLLATE "C") = lower($1 COLLATE "C")
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
    //! Two tripwires over this module's own source, both from the spec's security posture:
    //!
    //! 1. (§7, review L1) the F-3 pattern only holds for functions that DEMAND the sealed
    //!    [`SystemAdmin`] proof. Any function item with ANY `pub` visibility whose signature
    //!    does not bind a `&SystemAdmin` PARAMETER fails here, at the commit that adds it —
    //!    not at review.
    //! 2. (review M2) the directory is read-only. The PRODUCTION half of the file (everything
    //!    before the first `#[cfg(test)]`) must contain no statement-leading write SQL in its
    //!    string literals. The cut at `#[cfg(test)]` is deliberate: test fixtures legitimately
    //!    write, production must not.
    //!
    //! Both matchers are unit-tested against synthetic input below — a tripwire that cannot
    //! bite is decoration, so the bite is part of the test suite.

    /// True when `trimmed` STARTS a function item under ANY `pub` visibility form:
    /// `pub fn`, `pub async fn`, `pub(crate) fn`, `pub(super) async fn`, `pub(in path) fn` —
    /// the regex shape `^pub(\([^)]*\))? (async )?fn`, hand-rolled (no regex dependency).
    fn is_pub_fn_decl(trimmed: &str) -> bool {
        let Some(rest) = trimmed.strip_prefix("pub") else {
            return false;
        };
        let rest = match rest.strip_prefix('(') {
            // scoped form `pub(in crate::x)` — the path carries no nested parens
            Some(scoped) => {
                let Some(close) = scoped.find(')') else {
                    return false;
                };
                scoped[close + 1..].trim_start()
            }
            // plain `pub` must be followed by whitespace (`publisher` is not a declaration)
            None => {
                if !rest.starts_with(char::is_whitespace) {
                    return false;
                }
                rest.trim_start()
            }
        };
        let rest = rest.strip_prefix("async ").unwrap_or(rest);
        rest.starts_with("fn ")
    }

    /// True when the signature binds a parameter of EXACTLY `&SystemAdmin`: a `:` (whitespace-
    /// tolerant) must sit immediately before the `&`, and the type must END at the word
    /// boundary. So `admin: &SystemAdmin` and `admin:&SystemAdmin` pass, while
    /// `Option<&SystemAdmin>` (the `&` is preceded by `<`, not `:`) and a trailing identifier
    /// (`&SystemAdminFacade`) both fail. A RETURNED proof (`-> &SystemAdmin`) is not a demanded
    /// parameter and fails too.
    fn binds_sealed_proof(sig: &str) -> bool {
        for (pos, _) in sig.match_indices("&SystemAdmin") {
            let colon_ok = sig[..pos].trim_end().ends_with(':');
            let boundary_ok = sig[pos + "&SystemAdmin".len()..]
                .chars()
                .next()
                .map(|c| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(true);
            if colon_ok && boundary_ok {
                return true;
            }
        }
        false
    }

    /// Signatures span lines; accumulate from the declaration marker to the body/closing brace
    /// and judge the WHOLE signature, not its first line.
    fn unguarded_pub_fns(source: &str) -> Vec<String> {
        let mut offenders: Vec<String> = Vec::new();
        let mut current: Option<String> = None;
        for line in source.lines() {
            let trimmed = line.trim_start();
            if is_pub_fn_decl(trimmed) {
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
                    if !binds_sealed_proof(sig) {
                        offenders.push(sig.clone());
                    }
                    current = None;
                }
            }
        }
        offenders
    }

    /// The production half of the file: everything before the first test-module gate. Test
    /// fixtures legitimately write; the read-only invariant is about production.
    fn production_source(source: &str) -> &str {
        match source.find("#[cfg(test)]") {
            Some(cut) => &source[..cut],
            None => source,
        }
    }

    /// Statement-leading write SQL inside a string literal: a keyword at the literal's start
    /// or directly after a `;`. Mid-string prose (`"the UPDATE ran"`) and identifier
    /// continuations (`UPDATED_AT`) do not match; a data-modifying CTE would slip past, which
    /// is the accepted false-negative of a token scan.
    fn statement_leading_write_sql(literal: &str) -> bool {
        const KEYWORDS: [&str; 4] = ["INSERT INTO", "UPDATE", "DELETE FROM", "CALL"];
        literal.split(';').any(|segment| {
            let head = segment.trim_start();
            KEYWORDS.iter().any(|kw| {
                head.starts_with(kw)
                    && head[kw.len()..]
                        .chars()
                        .next()
                        .map(|c| c.is_whitespace() || c == '(')
                        .unwrap_or(true)
            })
        })
    }

    /// Collect the file's string literals — plain `"…"` (with escapes) and raw `r#"…"#` —
    /// the only places SQL can live.
    fn string_literals(source: &str) -> Vec<String> {
        let chars: Vec<char> = source.chars().collect();
        let mut literals = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            // raw string: r, then 1+ '#', then opening quote
            if chars[i] == 'r' && i + 1 < chars.len() && chars[i + 1] == '#' {
                let mut hashes = 0;
                let mut j = i + 1;
                while j < chars.len() && chars[j] == '#' {
                    hashes += 1;
                    j += 1;
                }
                if j < chars.len() && chars[j] == '"' {
                    let mut k = j + 1;
                    let mut closed = false;
                    while k < chars.len() {
                        if chars[k] == '"' {
                            let mut seen = 0;
                            let mut m = k + 1;
                            while m < chars.len() && chars[m] == '#' {
                                seen += 1;
                                m += 1;
                            }
                            if seen == hashes {
                                literals.push(chars[j + 1..k].iter().collect());
                                i = m;
                                closed = true;
                                break;
                            }
                        }
                        k += 1;
                    }
                    if closed {
                        continue;
                    }
                }
                i += 1;
                continue;
            }
            if chars[i] == '"' {
                let mut j = i + 1;
                let mut buf = String::new();
                while j < chars.len() && chars[j] != '"' {
                    if chars[j] == '\\' && j + 1 < chars.len() {
                        buf.push(chars[j]);
                        buf.push(chars[j + 1]);
                        j += 2;
                        continue;
                    }
                    buf.push(chars[j]);
                    j += 1;
                }
                literals.push(buf);
                i = j + 1;
                continue;
            }
            i += 1;
        }
        literals
    }

    fn write_sql_offenders(source: &str) -> Vec<String> {
        string_literals(source)
            .into_iter()
            .filter(|l| statement_leading_write_sql(l))
            .collect()
    }

    // ── the tripwires over the real file ─────────────────────────────────────

    #[test]
    fn every_public_fn_demands_the_sealed_admin_proof() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/services/admin_directory_service.rs"
        );
        let source = std::fs::read_to_string(path).expect("directory module source");
        let offenders = unguarded_pub_fns(&source);
        assert!(
            offenders.is_empty(),
            "directory service fn(s) taking no sealed &SystemAdmin proof: {offenders:#?}"
        );
    }

    #[test]
    fn the_production_module_contains_no_write_sql() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/services/admin_directory_service.rs"
        );
        let source = std::fs::read_to_string(path).expect("directory module source");
        let offenders = write_sql_offenders(production_source(&source));
        assert!(
            offenders.is_empty(),
            "read-only invariant: statement-leading write SQL in the production module: \
             {offenders:#?}"
        );
    }

    // ── the bite: synthetic input must trip BOTH matchers ────────────────────

    #[test]
    fn the_gate_matcher_bites_on_its_escape_shapes() {
        // The reviewer's exact escapes: a scoped visibility and an Option-wrapped proof.
        assert!(!unguarded_pub_fns(
            "pub(crate) fn unguarded(pool: &PgPool) -> ApiResult<()> {
}
"
        )
        .is_empty());
        assert!(
            !unguarded_pub_fns(
                "pub async fn sneaky(pool: &PgPool, admin: Option<&SystemAdmin>) {
}
"
            )
            .is_empty(),
            "Option<&SystemAdmin> must NOT count as demanding the proof"
        );
        assert!(
            !unguarded_pub_fns(
                "pub async fn facade(admin: &SystemAdminFacade) {
}
"
            )
            .is_empty(),
            "a trailing identifier must NOT satisfy the binding check"
        );
        assert!(
            !unguarded_pub_fns(
                "pub fn returns_proof() -> &SystemAdmin {
}
"
            )
            .is_empty(),
            "returning the proof is not demanding it"
        );
        // The honest shapes must pass.
        assert!(unguarded_pub_fns(
            "pub async fn guarded(pool: &PgPool, admin: &SystemAdmin) -> ApiResult<()> {
}
"
        )
        .is_empty());
        assert!(unguarded_pub_fns(
            "pub(super) fn tight(admin:&SystemAdmin) {
}
"
        )
        .is_empty());
        assert!(unguarded_pub_fns(
            "pub(in crate::services) async fn scoped(
    pool: &PgPool,
    admin: &SystemAdmin,
) -> ApiResult<()> {
}
"
        )
        .is_empty());
    }

    #[test]
    fn the_write_sql_scanner_bites_and_spares() {
        assert!(!write_sql_offenders("sqlx::query(\"INSERT INTO kb_x VALUES (1)\")").is_empty());
        assert!(!write_sql_offenders("\"SELECT 1; DELETE FROM kb_y\"").is_empty());
        assert!(!write_sql_offenders("let q = r#\"UPDATE kb_z SET x = 1\"#;").is_empty());
        assert!(!write_sql_offenders("\"CALL do_thing()\"").is_empty());
        // The negatives: SELECT statements, mid-string prose, identifier continuations.
        assert!(write_sql_offenders("\"SELECT * FROM kb_x WHERE updated_at > now()\"").is_empty());
        assert!(write_sql_offenders("\"the UPDATE ran late\"").is_empty());
        assert!(write_sql_offenders("\"UPDATED_AT, CALLED_AT, INSERTION_ORDER\"").is_empty());
        // And the real production file proves the negative case end-to-end
        // (`the_production_module_contains_no_write_sql` asserts this too).
    }
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
