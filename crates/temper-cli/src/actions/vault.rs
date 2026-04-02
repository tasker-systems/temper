use std::path::Path;

use crate::actions::types::VaultDocument;
use crate::error::{Result, TemperError};
use crate::vault::parse_frontmatter;

/// Read a vault markdown file and return a VaultDocument with parsed frontmatter and body.
pub fn read_document(path: &Path) -> Result<VaultDocument> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| TemperError::Vault(format!("Failed to read {}: {e}", path.display())))?;

    let frontmatter = parse_frontmatter(&raw).unwrap_or(serde_yaml::Value::Null);

    let note_type = frontmatter
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let title = frontmatter
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Split off the frontmatter block to get just the body
    let body = extract_body(&raw);

    Ok(VaultDocument {
        path: path.to_string_lossy().into_owned(),
        note_type,
        title,
        frontmatter,
        body,
    })
}

/// Strip the leading `---\n...\n---\n` frontmatter block and return the remaining body.
fn extract_body(content: &str) -> String {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content.to_string();
    }
    let rest = &trimmed[3..];
    if let Some(end) = rest.find("---") {
        // skip past the closing `---` and any trailing newline
        let after = &rest[end + 3..];
        after.trim_start_matches('\n').to_string()
    } else {
        content.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_body_with_frontmatter() {
        let content = "---\ntype: task\ntitle: My Task\n---\n\n# Body here\n\nSome text.";
        let body = extract_body(content);
        assert_eq!(body, "# Body here\n\nSome text.");
    }

    #[test]
    fn test_extract_body_no_frontmatter() {
        let content = "# Just a heading\n\nSome text.";
        let body = extract_body(content);
        assert_eq!(body, content);
    }

    #[test]
    fn test_read_document_roundtrip() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("test.md");
        std::fs::write(
            &file,
            "---\ntype: session\ntitle: Test Session\n---\n\nSession body.\n",
        )?;
        let doc = read_document(&file)?;
        assert_eq!(doc.note_type, "session");
        assert_eq!(doc.title, "Test Session");
        assert_eq!(doc.body, "Session body.\n");
        Ok(())
    }
}
