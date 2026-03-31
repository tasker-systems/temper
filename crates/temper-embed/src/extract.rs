//! Document extraction — markdown/text passthrough, kreuzberg for other formats.

use std::path::Path;

use crate::error::{EmbedError, Result};

/// The result of extracting a file to text.
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    pub content: String,
    pub mime_type: String,
}

/// Extract a file to markdown text.
pub fn extract_to_markdown(path: &Path) -> Result<ExtractionResult> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match extension.as_str() {
        "md" | "markdown" => Ok(ExtractionResult {
            content: std::fs::read_to_string(path)?,
            mime_type: "text/markdown".to_string(),
        }),
        "txt" | "text" => Ok(ExtractionResult {
            content: std::fs::read_to_string(path)?,
            mime_type: "text/plain".to_string(),
        }),
        _ => extract_with_kreuzberg(path),
    }
}

#[cfg(feature = "extract")]
fn extract_with_kreuzberg(path: &Path) -> Result<ExtractionResult> {
    use kreuzberg::{extract_file_sync, ExtractionConfig};

    let config = ExtractionConfig::default();
    let result = extract_file_sync(path, None, &config).map_err(|e| {
        EmbedError::Extraction(format!("failed to extract '{}': {}", path.display(), e))
    })?;

    Ok(ExtractionResult {
        content: result.content,
        mime_type: result.mime_type.into_owned(),
    })
}

#[cfg(not(feature = "extract"))]
fn extract_with_kreuzberg(path: &Path) -> Result<ExtractionResult> {
    Err(EmbedError::Extraction(format!(
        "cannot extract '{}': the 'extract' feature is required for non-text files",
        path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_extract_markdown() {
        let mut f = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(f, "# Hello").unwrap();
        let r = extract_to_markdown(f.path()).unwrap();
        assert!(r.content.contains("# Hello"));
        assert_eq!(r.mime_type, "text/markdown");
    }

    #[test]
    fn test_extract_plain_text() {
        let mut f = NamedTempFile::with_suffix(".txt").unwrap();
        writeln!(f, "Hello").unwrap();
        let r = extract_to_markdown(f.path()).unwrap();
        assert!(r.content.contains("Hello"));
        assert_eq!(r.mime_type, "text/plain");
    }

    #[test]
    #[cfg(not(feature = "extract"))]
    fn test_non_text_without_feature() {
        let f = NamedTempFile::with_suffix(".pdf").unwrap();
        assert!(extract_to_markdown(f.path()).is_err());
    }
}
