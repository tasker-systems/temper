//! Vault layout and kb:// URI construction.
//!
//! Centralizes every rule about how `(owner, context, doc_type, slug)` maps to
//! filesystem paths, manifest-relative path strings, and canonical kb:// URIs.
//! Shared by temper-cli, temper-api, and temper-mcp so all three produce
//! byte-identical paths and URIs for the same inputs.

use std::path::{Path, PathBuf};

/// Owns layout rules for a specific vault root. Construct once per operation;
/// methods are pure functions of the inputs.
#[derive(Debug)]
pub struct Vault<'a> {
    vault_root: &'a Path,
}

/// A parsed vault-relative path. Borrows from the input string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedVaultPath<'a> {
    /// Owner sigil + identifier, e.g., "@me" or "+platform-eng".
    pub owner: &'a str,
    pub context: &'a str,
    pub doc_type: &'a str,
    /// Filename stem (no .md extension).
    pub slug: &'a str,
}

/// A parsed kb:// URI. Borrows from the input string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedKbUri<'a> {
    /// Owner sigil + identifier, e.g., "@me" or "+platform-eng".
    pub owner: &'a str,
    pub context: &'a str,
    pub doc_type: &'a str,
    /// Identifier — slug or UUID string, caller decides.
    pub ident: &'a str,
}

impl<'a> Vault<'a> {
    /// Construct a Vault for a given vault root directory.
    pub fn new(vault_root: &'a Path) -> Self {
        Self { vault_root }
    }

    /// Absolute directory where files of a given (owner, context, doc_type) live.
    /// Returns `<vault_root>/<owner>/<context>/<doc_type>/`.
    pub fn doc_type_dir(&self, owner: &str, context: &str, doc_type: &str) -> PathBuf {
        self.vault_root.join(owner).join(context).join(doc_type)
    }

    /// Absolute file path for a specific resource.
    /// Returns `<vault_root>/<owner>/<context>/<doc_type>/<slug>.md`.
    pub fn doc_file(&self, owner: &str, context: &str, doc_type: &str, slug: &str) -> PathBuf {
        self.doc_type_dir(owner, context, doc_type)
            .join(format!("{slug}.md"))
    }

    /// Vault-relative path string used in manifest entries and discovery events.
    /// Returns `<owner>/<context>/<doc_type>/<slug>.md`.
    pub fn rel_path(&self, owner: &str, context: &str, doc_type: &str, slug: &str) -> String {
        format!("{owner}/{context}/{doc_type}/{slug}.md")
    }

    /// Parse a vault-relative path back into components.
    /// Returns `None` if the path is malformed (missing owner sigil, wrong segment
    /// count, or non-`.md` filename).
    ///
    /// Associated function — no Vault instance needed. Callers that only parse
    /// manifest paths do not need to construct a Vault.
    pub fn parse_rel(rel: &str) -> Option<ParsedVaultPath<'_>> {
        let parts: Vec<&str> = rel.split('/').collect();
        if parts.len() != 4 {
            return None;
        }
        // Reject any empty segment (leading/trailing slashes, double slashes).
        if parts.iter().any(|s| s.is_empty()) {
            return None;
        }
        let owner = parts[0];
        // Sigil must be followed by at least one character.
        if owner.len() < 2 || !(owner.starts_with('@') || owner.starts_with('+')) {
            return None;
        }
        let context = parts[1];
        let doc_type = parts[2];
        let filename = parts[3];
        let slug = filename.strip_suffix(".md")?;
        // Reject empty slug (e.g., a file named ".md").
        if slug.is_empty() {
            return None;
        }
        Some(ParsedVaultPath {
            owner,
            context,
            doc_type,
            slug,
        })
    }

    // ----- URI operations (pure, no vault_root needed) -----

    /// Build a canonical kb:// URI from components.
    /// Returns `kb://<owner>/<context>/<doc_type>/<ident>`.
    ///
    /// Associated function — no Vault instance needed. API/MCP use this without
    /// touching the filesystem.
    pub fn canonical_uri(owner: &str, context: &str, doc_type: &str, ident: &str) -> String {
        format!("kb://{owner}/{context}/{doc_type}/{ident}")
    }

    /// Parse a kb:// URI into components. Rejects legacy no-sigil URIs.
    ///
    /// Associated function — no Vault instance needed.
    pub fn parse_uri(uri: &str) -> Option<ParsedKbUri<'_>> {
        let rest = uri.strip_prefix("kb://")?;
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() != 4 {
            return None;
        }
        // Reject any empty segment (leading/trailing slashes, double slashes).
        if parts.iter().any(|s| s.is_empty()) {
            return None;
        }
        let owner = parts[0];
        // Sigil must be followed by at least one character.
        if owner.len() < 2 || !(owner.starts_with('@') || owner.starts_with('+')) {
            return None;
        }
        Some(ParsedKbUri {
            owner,
            context: parts[1],
            doc_type: parts[2],
            ident: parts[3],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from("/tmp/test-vault")
    }

    #[test]
    fn doc_type_dir_personal_owner() {
        let root = root();
        let v = Vault::new(&root);
        assert_eq!(
            v.doc_type_dir("@me", "temper", "task"),
            PathBuf::from("/tmp/test-vault/@me/temper/task")
        );
    }

    #[test]
    fn doc_type_dir_team_owner() {
        let root = root();
        let v = Vault::new(&root);
        assert_eq!(
            v.doc_type_dir("+platform-eng", "temper", "task"),
            PathBuf::from("/tmp/test-vault/+platform-eng/temper/task")
        );
    }

    #[test]
    fn doc_file_builds_full_path_with_md_extension() {
        let root = root();
        let v = Vault::new(&root);
        assert_eq!(
            v.doc_file("@me", "temper", "task", "my-task"),
            PathBuf::from("/tmp/test-vault/@me/temper/task/my-task.md")
        );
    }

    #[test]
    fn rel_path_returns_vault_relative_string() {
        let root = root();
        let v = Vault::new(&root);
        assert_eq!(
            v.rel_path("@me", "temper", "task", "my-task"),
            "@me/temper/task/my-task.md".to_string()
        );
    }

    #[test]
    fn rel_path_team_owner() {
        let root = root();
        let v = Vault::new(&root);
        assert_eq!(
            v.rel_path("+team-x", "general", "goal", "q4-launch"),
            "+team-x/general/goal/q4-launch.md".to_string()
        );
    }

    #[test]
    fn parse_rel_valid_personal_owner() {
        let parsed = Vault::parse_rel("@me/temper/task/my-task.md").unwrap();
        assert_eq!(parsed.owner, "@me");
        assert_eq!(parsed.context, "temper");
        assert_eq!(parsed.doc_type, "task");
        assert_eq!(parsed.slug, "my-task");
    }

    #[test]
    fn parse_rel_valid_team_owner() {
        let parsed = Vault::parse_rel("+platform-eng/temper/goal/q4.md").unwrap();
        assert_eq!(parsed.owner, "+platform-eng");
        assert_eq!(parsed.context, "temper");
        assert_eq!(parsed.doc_type, "goal");
        assert_eq!(parsed.slug, "q4");
    }

    #[test]
    fn parse_rel_rejects_no_sigil() {
        assert!(Vault::parse_rel("temper/task/my-task.md").is_none());
    }

    #[test]
    fn parse_rel_rejects_too_few_segments() {
        assert!(Vault::parse_rel("@me/temper/task").is_none());
        assert!(Vault::parse_rel("@me/temper").is_none());
        assert!(Vault::parse_rel("@me").is_none());
        assert!(Vault::parse_rel("").is_none());
    }

    #[test]
    fn parse_rel_rejects_non_md_extension() {
        assert!(Vault::parse_rel("@me/temper/task/my-task.txt").is_none());
        assert!(Vault::parse_rel("@me/temper/task/my-task").is_none());
    }

    #[test]
    fn parse_rel_rejects_empty_segments_and_bare_sigil() {
        // Empty mid-segment (double slash)
        assert!(Vault::parse_rel("@me//task/foo.md").is_none());
        // Leading slash produces an empty first segment
        assert!(Vault::parse_rel("/@me/temper/task/foo.md").is_none());
        // Bare sigil with no identifier
        assert!(Vault::parse_rel("@/temper/task/foo.md").is_none());
        // Empty slug (file named just ".md")
        assert!(Vault::parse_rel("@me/temper/task/.md").is_none());
    }

    #[test]
    fn parse_rel_round_trips_with_rel_path() {
        let root = root();
        let v = Vault::new(&root);
        let rel = v.rel_path("@me", "temper", "task", "round-trip");
        let parsed = Vault::parse_rel(&rel).unwrap();
        assert_eq!(parsed.owner, "@me");
        assert_eq!(parsed.context, "temper");
        assert_eq!(parsed.doc_type, "task");
        assert_eq!(parsed.slug, "round-trip");
    }

    #[test]
    fn canonical_uri_personal_with_slug() {
        assert_eq!(
            Vault::canonical_uri("@me", "temper", "task", "my-task"),
            "kb://@me/temper/task/my-task".to_string()
        );
    }

    #[test]
    fn canonical_uri_team_with_slug() {
        assert_eq!(
            Vault::canonical_uri("+team-x", "general", "goal", "q4"),
            "kb://+team-x/general/goal/q4".to_string()
        );
    }

    #[test]
    fn canonical_uri_with_uuid_ident() {
        assert_eq!(
            Vault::canonical_uri(
                "@me",
                "temper",
                "task",
                "019d6880-5c21-7bb2-86fb-a0cc612b5cf5"
            ),
            "kb://@me/temper/task/019d6880-5c21-7bb2-86fb-a0cc612b5cf5".to_string()
        );
    }

    #[test]
    fn parse_uri_valid_personal() {
        let parsed = Vault::parse_uri("kb://@me/temper/task/my-task").unwrap();
        assert_eq!(parsed.owner, "@me");
        assert_eq!(parsed.context, "temper");
        assert_eq!(parsed.doc_type, "task");
        assert_eq!(parsed.ident, "my-task");
    }

    #[test]
    fn parse_uri_valid_team() {
        let parsed = Vault::parse_uri("kb://+platform/general/goal/q4").unwrap();
        assert_eq!(parsed.owner, "+platform");
        assert_eq!(parsed.context, "general");
        assert_eq!(parsed.doc_type, "goal");
        assert_eq!(parsed.ident, "q4");
    }

    #[test]
    fn parse_uri_rejects_legacy_no_sigil() {
        assert!(Vault::parse_uri("kb://temper/task/my-task").is_none());
    }

    #[test]
    fn parse_uri_rejects_missing_scheme() {
        assert!(Vault::parse_uri("@me/temper/task/my-task").is_none());
        assert!(Vault::parse_uri("http://@me/temper/task/my-task").is_none());
    }

    #[test]
    fn parse_uri_rejects_too_few_segments() {
        assert!(Vault::parse_uri("kb://@me/temper/task").is_none());
        assert!(Vault::parse_uri("kb://@me/temper").is_none());
        assert!(Vault::parse_uri("kb://@me").is_none());
        assert!(Vault::parse_uri("kb://").is_none());
    }

    #[test]
    fn parse_uri_round_trips_with_canonical_uri() {
        let uri = Vault::canonical_uri("@me", "temper", "task", "round-trip");
        let parsed = Vault::parse_uri(&uri).unwrap();
        assert_eq!(parsed.owner, "@me");
        assert_eq!(parsed.context, "temper");
        assert_eq!(parsed.doc_type, "task");
        assert_eq!(parsed.ident, "round-trip");
    }

    #[test]
    fn parse_uri_rejects_empty_segments_and_bare_sigil() {
        // Empty mid-segment
        assert!(Vault::parse_uri("kb://@me//task/foo").is_none());
        // Bare sigil with no identifier
        assert!(Vault::parse_uri("kb://@/temper/task/foo").is_none());
        // Empty ident
        assert!(Vault::parse_uri("kb://@me/temper/task/").is_none());
    }
}
