//! Context addressing by ref: a bare UUID or a decorated `@owner/slug` form.
//!
//! UUID-primary, mirroring resource refs (`temper_workflow::operations::parse_ref`).
//! Pure string parsing — no DB, no principal. Resolution to a `ContextId`
//! (owner lookup + visibility gate) lives server-side in temper-api.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::validation::validate_owner_pattern;

/// The owner half of a decorated context ref.
///
/// Derives `Serialize`/`Deserialize` so it can ride in API request bodies as a
/// typed owner descriptor (e.g. `ContextCreateRequest::owner`), not a flat
/// `(owner_table, owner_id)` pair.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextOwnerRef {
    /// `@me` — the calling principal's own profile.
    Me,
    /// `@<handle>` — a personal profile addressed by its global-unique handle.
    Handle(String),
    /// `+<team-slug>` — a team addressed by its global-unique slug.
    Team(String),
}

/// A parsed context reference. UUID-primary; the decorated form carries an
/// owner + the context's per-owner-unique slug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextRef {
    /// Canonical: the `kb_contexts.id` UUID.
    Id(Uuid),
    /// Decorated: resolved via the `(owner_table, owner_id, slug)` natural key.
    OwnerSlug {
        owner: ContextOwnerRef,
        slug: String,
    },
}

/// Why a context ref string could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContextRefError {
    #[error("not a context ref: bare names are not addressable — use a UUID or `@owner/slug` (got {0:?})")]
    BareName(String),
    #[error("context ref is missing the `/slug` after the owner (got {0:?})")]
    MissingSlug(String),
    #[error("context ref slug must be lowercase alphanumeric with hyphens (got {0:?})")]
    BadSlug(String),
    #[error("context ref owner is invalid: {0}")]
    BadOwner(#[from] crate::validation::OwnerPatternError),
}

/// Same slug rules contexts already enforce: lowercase alnum + hyphens, leading-alnum.
fn validate_slug(slug: &str) -> Result<(), ContextRefError> {
    let ok = !slug.is_empty()
        && slug.as_bytes()[0].is_ascii_alphanumeric()
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(ContextRefError::BadSlug(slug.to_owned()))
    }
}

/// Build the decorated context ref for display/round-trip.
///
/// `owner_addressable` is the bare handle (profiles) or team-slug (teams),
/// WITHOUT a sigil. `owner_table` is the substrate discriminator
/// (`"kb_profiles"` or `"kb_teams"`).
///
/// Returns `@<handle>/<context_slug>` for profile owners, or
/// `+<team-slug>/<context_slug>` for team owners.
pub fn decorated_context_ref(
    owner_table: &str,
    owner_addressable: &str,
    context_slug: &str,
) -> String {
    let sigil = if owner_table == "kb_teams" { '+' } else { '@' };
    format!("{sigil}{owner_addressable}/{context_slug}")
}

/// Parse a context ref. Pure — no DB, no principal. See [`ContextRef`].
pub fn parse_context_ref(s: &str) -> Result<ContextRef, ContextRefError> {
    let s = s.trim();

    // Bare UUID — canonical.
    if let Ok(id) = Uuid::parse_str(s) {
        return Ok(ContextRef::Id(id));
    }

    let first = s.as_bytes().first().copied();
    if first != Some(b'@') && first != Some(b'+') {
        return Err(ContextRefError::BareName(s.to_owned()));
    }

    // Decorated: `<owner>/<slug>` where owner keeps its sigil.
    let (owner_part, slug) = s
        .split_once('/')
        .ok_or_else(|| ContextRefError::MissingSlug(s.to_owned()))?;

    validate_owner_pattern(owner_part)?; // validates `@handle` / `+team`
    validate_slug(slug)?;

    let owner = if owner_part == "@me" {
        ContextOwnerRef::Me
    } else if let Some(handle) = owner_part.strip_prefix('@') {
        ContextOwnerRef::Handle(handle.to_owned())
    } else {
        // `+` guaranteed by validate_owner_pattern's sigil check
        ContextOwnerRef::Team(owner_part[1..].to_owned())
    };

    Ok(ContextRef::OwnerSlug {
        owner,
        slug: slug.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn parses_bare_uuid() {
        let u = Uuid::now_v7();
        assert_eq!(
            parse_context_ref(&u.to_string()).unwrap(),
            ContextRef::Id(u)
        );
    }

    #[test]
    fn parses_me_slug() {
        let r = parse_context_ref("@me/temper").unwrap();
        assert_eq!(
            r,
            ContextRef::OwnerSlug {
                owner: ContextOwnerRef::Me,
                slug: "temper".into()
            }
        );
    }

    #[test]
    fn parses_handle_slug() {
        let r = parse_context_ref("@j-cole-taylor/temper").unwrap();
        assert_eq!(
            r,
            ContextRef::OwnerSlug {
                owner: ContextOwnerRef::Handle("j-cole-taylor".into()),
                slug: "temper".into()
            }
        );
    }

    #[test]
    fn parses_team_slug() {
        let r = parse_context_ref("+tasker-systems/general").unwrap();
        assert_eq!(
            r,
            ContextRef::OwnerSlug {
                owner: ContextOwnerRef::Team("tasker-systems".into()),
                slug: "general".into()
            }
        );
    }

    #[test]
    fn rejects_bare_name() {
        assert!(matches!(
            parse_context_ref("temper"),
            Err(ContextRefError::BareName(_))
        ));
    }

    #[test]
    fn rejects_owner_without_slug() {
        assert!(matches!(
            parse_context_ref("@me"),
            Err(ContextRefError::MissingSlug(_))
        ));
        assert!(matches!(
            parse_context_ref("+team"),
            Err(ContextRefError::MissingSlug(_))
        ));
    }

    #[test]
    fn rejects_empty_slug() {
        assert!(parse_context_ref("@me/").is_err());
    }

    #[test]
    fn rejects_bad_owner() {
        assert!(parse_context_ref("@/temper").is_err());
        assert!(parse_context_ref("@UPPER/temper").is_err());
        assert!(parse_context_ref("temper/x").is_err()); // no sigil
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(
            parse_context_ref("  @me/temper  ").unwrap(),
            ContextRef::OwnerSlug {
                owner: ContextOwnerRef::Me,
                slug: "temper".into()
            }
        );
    }

    #[test]
    fn decorates_profile_and_team() {
        assert_eq!(
            decorated_context_ref("kb_profiles", "j-cole-taylor", "temper"),
            "@j-cole-taylor/temper"
        );
        assert_eq!(
            decorated_context_ref("kb_teams", "tasker-systems", "general"),
            "+tasker-systems/general"
        );
    }
}
