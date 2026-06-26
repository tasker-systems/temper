//! Shared validation functions for temper metadata fields.

/// Why a `temper-owner` value failed [`validate_owner_pattern`]. A typed error (not a `String`) so
/// callers in a library API can match on the specific fault rather than parse a message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OwnerPatternError {
    #[error("temper-owner cannot be empty")]
    Empty,
    #[error("temper-owner must start with '@' (personal) or '+' (team), got: {0}")]
    BadSigil(String),
    #[error("temper-owner handle cannot be empty after sigil")]
    EmptyHandle,
    #[error("temper-owner handle must start with alphanumeric, got: {0}")]
    BadHandleStart(String),
    #[error("temper-owner handle must be lowercase alphanumeric with hyphens, got: {0}")]
    BadHandleChars(String),
}

/// Validate a temper-owner field value.
/// Valid patterns: `@handle` (personal) or `+team` (team).
/// Handle/team must be lowercase alphanumeric with hyphens, starting with alphanumeric.
pub fn validate_owner_pattern(value: &str) -> Result<(), OwnerPatternError> {
    if value.is_empty() {
        return Err(OwnerPatternError::Empty);
    }
    let first = value.as_bytes()[0];
    if first != b'@' && first != b'+' {
        return Err(OwnerPatternError::BadSigil(value.to_owned()));
    }
    let handle = &value[1..];
    if handle.is_empty() {
        return Err(OwnerPatternError::EmptyHandle);
    }
    if !handle.as_bytes()[0].is_ascii_alphanumeric() {
        return Err(OwnerPatternError::BadHandleStart(value.to_owned()));
    }
    if !handle
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(OwnerPatternError::BadHandleChars(value.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_personal_owner() {
        assert!(validate_owner_pattern("@alice").is_ok());
        assert!(validate_owner_pattern("@j-cole-taylor").is_ok());
    }

    #[test]
    fn valid_team_owner() {
        assert!(validate_owner_pattern("+tasker-systems").is_ok());
        assert!(validate_owner_pattern("+team1").is_ok());
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_owner_pattern("").is_err());
    }

    #[test]
    fn rejects_no_sigil() {
        assert!(validate_owner_pattern("alice").is_err());
    }

    #[test]
    fn rejects_uppercase() {
        assert!(validate_owner_pattern("@Alice").is_err());
    }

    #[test]
    fn rejects_empty_handle() {
        assert!(validate_owner_pattern("@").is_err());
    }

    #[test]
    fn rejects_handle_starting_with_hyphen() {
        assert!(validate_owner_pattern("@-alice").is_err());
    }
}
