//! Pure text helpers shared across the substrate (writes, scenario).

/// Lowercase, alphanumeric-or-dash slug. Moved verbatim from the retired
/// `synthesis::bootstrap::slugify` (the synthesis scaffolding is deleted at collapse). Mirrors the
/// surfaces' `slugify`; shared by `writes` + the access-scenario loader's context-slug derivation.
pub fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|seg| !seg.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slugify_lowercases_and_dashes() {
        assert_eq!(slugify("Hello World!"), "hello-world");
        assert_eq!(slugify("  A  B  "), "a-b");
    }
}
