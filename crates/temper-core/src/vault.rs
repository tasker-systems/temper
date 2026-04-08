//! Vault layout and kb:// URI construction.
//!
//! Centralizes every rule about how `(owner, context, doc_type, slug)` maps to
//! filesystem paths, manifest-relative path strings, and canonical kb:// URIs.
//! Shared by temper-cli, temper-api, and temper-mcp so all three produce
//! byte-identical paths and URIs for the same inputs.

use std::path::{Path, PathBuf};

/// Owns layout rules for a specific vault root. Construct once per operation;
/// methods are pure functions of the inputs.
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
}
