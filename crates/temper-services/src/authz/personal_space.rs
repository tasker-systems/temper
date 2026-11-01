//! Where a system admin's arm stops: someone else's personal space, and what the admin is told.
//!
//! Personal space is a context owned by a person or by their personal team, a resource homed in
//! one, a cognitive map that is one person's (`cogmap_personal_owner`, task `01a1933d`), and a
//! person's personal team itself. Only its person acts on it, and erasure (task `01a191b4`).
//!
//! The stop exists to keep an admin from acting on someone's personal space **by mistake**: a wrong
//! UUID, a stale ref, an act aimed at the wrong person. It is not a defence against a trusted admin
//! (decision `01a19396-93c0-77ca-809a-c1ba1608e423`). So an admin it refuses is told plainly what
//! they hit: a `403` that names the kind of object and its person's handle, never its content.
//! A system admin may already list personal teams and their rosters for audit, so the handle
//! discloses nothing they could not read. Every door that stops an admin at personal space renders
//! the refusal through [`refusal`], so the doors say the same sentence.
//!
//! Callers that are not system admins never reach this: their ordinary gates answer as before, and
//! where existence would leak, they still answer `404`.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_substrate::payloads::{AnchorTable, RefTarget};

use crate::error::{ApiError, ApiResult};

/// What kind of personal space an admin was stopped at — the only thing about the subject, beside
/// its person, that the refusal names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersonalSpaceKind {
    /// A context owned by a person or by their personal team.
    Context,
    /// A resource homed in such a context.
    Resource,
    /// A cognitive map that is one person's.
    Cogmap,
    /// A person's personal team, as the target of a share, bind or reassign.
    PersonalTeam,
}

/// An admin stopped at someone else's personal space: which kind, and whose. `Copy` and free of
/// any content, so an authority arm can carry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PersonalSpaceStop {
    pub(crate) kind: PersonalSpaceKind,
    pub(crate) owner: ProfileId,
}

impl PersonalSpaceStop {
    pub(crate) fn new(kind: PersonalSpaceKind, owner: ProfileId) -> Self {
        Self { kind, owner }
    }
}

/// The refusal sentence. `handle` is the person's handle, or `None` when the profile no longer
/// carries one (an erased person), which reads "someone else's".
pub(crate) fn sentence(kind: PersonalSpaceKind, handle: Option<&str>) -> String {
    let whose = match handle {
        Some(h) => format!("@{h}'s"),
        None => "someone else's".to_string(),
    };
    match kind {
        PersonalSpaceKind::Context => {
            format!("this context is {whose} personal space; the admin arm does not reach it")
        }
        PersonalSpaceKind::Resource => format!(
            "this resource is homed in {whose} personal space; the admin arm does not reach it"
        ),
        PersonalSpaceKind::Cogmap => {
            format!("this cognitive map is {whose} personal space; the admin arm does not reach it")
        }
        PersonalSpaceKind::PersonalTeam => format!(
            "this team is {whose} personal team, which is personal space; the admin arm does not \
             reach it"
        ),
    }
}

/// The `403` an admin stopped at personal space receives: [`sentence`] with the person's handle.
pub(crate) async fn refusal(pool: &PgPool, stop: PersonalSpaceStop) -> ApiResult<ApiError> {
    let handle = sqlx::query_scalar!(
        r#"SELECT handle AS "handle?" FROM kb_profiles WHERE id = $1"#,
        *stop.owner,
    )
    .fetch_optional(pool)
    .await?
    .flatten();
    Ok(ApiError::ForbiddenDetail(sentence(
        stop.kind,
        handle.as_deref(),
    )))
}

/// The person whose personal space `subject` is, when that is someone other than `caller`. The
/// SQL `context_personal_owner`, `resource_personal_owner` and `cogmap_personal_owner` are the one
/// definition of whose; this asks them.
pub(crate) async fn others_personal_space(
    pool: &PgPool,
    subject: RefTarget,
    caller: ProfileId,
) -> ApiResult<Option<PersonalSpaceStop>> {
    let (kind, owner) = match subject.kind {
        AnchorTable::Contexts => (
            PersonalSpaceKind::Context,
            sqlx::query_scalar!(r#"SELECT context_personal_owner($1) AS "o""#, subject.id)
                .fetch_one(pool)
                .await?,
        ),
        AnchorTable::Resources => (
            PersonalSpaceKind::Resource,
            sqlx::query_scalar!(r#"SELECT resource_personal_owner($1) AS "o""#, subject.id)
                .fetch_one(pool)
                .await?,
        ),
        AnchorTable::Cogmaps => (
            PersonalSpaceKind::Cogmap,
            cogmap_personal_owner(pool, subject.id).await?,
        ),
        _ => return Ok(None),
    };
    Ok(owner
        .filter(|o| *o != *caller)
        .map(|o| PersonalSpaceStop::new(kind, ProfileId::from(o))))
}

/// `cogmap_personal_owner(map)`: the person whose personal space a map is, or `None`.
pub(crate) async fn cogmap_personal_owner(
    pool: &PgPool,
    cogmap_id: Uuid,
) -> ApiResult<Option<Uuid>> {
    Ok(
        sqlx::query_scalar!(r#"SELECT cogmap_personal_owner($1) AS "o""#, cogmap_id)
            .fetch_one(pool)
            .await?,
    )
}

/// A person's personal team that is not `caller`'s, as a stop. An agent's personal team is not
/// personal space (`team_service::personal_space_owner`).
pub(crate) async fn others_personal_team(
    pool: &PgPool,
    team_id: Uuid,
    caller: ProfileId,
) -> ApiResult<Option<PersonalSpaceStop>> {
    Ok(
        crate::services::team_service::personal_space_owner(pool, team_id)
            .await?
            .filter(|o| *o != caller)
            .map(|o| PersonalSpaceStop::new(PersonalSpaceKind::PersonalTeam, o)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_names_itself_and_whose_without_content() {
        for (kind, noun) in [
            (
                PersonalSpaceKind::Context,
                "this context is @ana's personal space",
            ),
            (
                PersonalSpaceKind::Resource,
                "this resource is homed in @ana's personal space",
            ),
            (
                PersonalSpaceKind::Cogmap,
                "this cognitive map is @ana's personal space",
            ),
            (
                PersonalSpaceKind::PersonalTeam,
                "this team is @ana's personal team",
            ),
        ] {
            let s = sentence(kind, Some("ana"));
            assert!(s.starts_with(noun), "{s}");
            assert!(s.ends_with("the admin arm does not reach it"), "{s}");
        }
        assert!(sentence(PersonalSpaceKind::Cogmap, None).contains("someone else's personal space"));
    }
}
