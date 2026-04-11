//! Shared validation functions for temper metadata fields.

/// Validate a temper-owner field value.
/// Valid patterns: `@handle` (personal) or `+team` (team).
/// Handle/team must be lowercase alphanumeric with hyphens, starting with alphanumeric.
pub fn validate_owner_pattern(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("temper-owner cannot be empty".to_owned());
    }
    let first = value.as_bytes()[0];
    if first != b'@' && first != b'+' {
        return Err(format!(
            "temper-owner must start with '@' (personal) or '+' (team), got: {value}"
        ));
    }
    let handle = &value[1..];
    if handle.is_empty() {
        return Err("temper-owner handle cannot be empty after sigil".to_owned());
    }
    if !handle.as_bytes()[0].is_ascii_alphanumeric() {
        return Err(format!(
            "temper-owner handle must start with alphanumeric, got: {value}"
        ));
    }
    if !handle
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(format!(
            "temper-owner handle must be lowercase alphanumeric with hyphens, got: {value}"
        ));
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
