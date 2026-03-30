//! Document extraction module.
//!
//! Provides extraction of files to markdown/plain text. Markdown and plain text
//! files are read directly without any external dependency. All other formats
//! (PDF, Office documents, HTML, etc.) require the `extract` feature, which
//! pulls in the `kreuzberg` crate.

use std::path::Path;

use crate::error::{Result, TemperError};

/// The result of extracting a file to text.
pub struct ExtractionResult {
    /// The extracted text content (markdown or plain text).
    pub content: String,
    /// The detected or inferred MIME type of the source file.
    pub mime_type: String,
}

/// Extract a file to markdown text.
///
/// Markdown and plain text files are read directly without kreuzberg.
/// All other formats require the `extract` feature to be enabled.
///
/// # Errors
///
/// Returns an error if the file cannot be read, or if the format requires
/// kreuzberg and the `extract` feature is not enabled.
pub fn extract_to_markdown(path: &Path) -> Result<ExtractionResult> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match extension.as_str() {
        "md" | "markdown" => {
            let content = std::fs::read_to_string(path)?;
            Ok(ExtractionResult {
                content,
                mime_type: "text/markdown".to_string(),
            })
        }
        "txt" | "text" => {
            let content = std::fs::read_to_string(path)?;
            Ok(ExtractionResult {
                content,
                mime_type: "text/plain".to_string(),
            })
        }
        _ => extract_with_kreuzberg(path),
    }
}

#[cfg(feature = "extract")]
fn extract_with_kreuzberg(path: &Path) -> Result<ExtractionResult> {
    use kreuzberg::{extract_file_sync, ExtractionConfig};

    let config = ExtractionConfig::default();
    let result = extract_file_sync(path, None, &config).map_err(|e| {
        TemperError::Extraction(format!("failed to extract '{}': {}", path.display(), e))
    })?;

    Ok(ExtractionResult {
        content: result.content,
        mime_type: result.mime_type.into_owned(),
    })
}

#[cfg(not(feature = "extract"))]
fn extract_with_kreuzberg(path: &Path) -> Result<ExtractionResult> {
    Err(TemperError::Extraction(format!(
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
    fn test_extract_markdown_file() {
        let mut file = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(file, "# Hello\n\nThis is markdown.").unwrap();
        let path = file.path().to_owned();

        let result = extract_to_markdown(&path).unwrap();
        assert!(result.content.contains("# Hello"));
        assert_eq!(result.mime_type, "text/markdown");
    }

    #[test]
    fn test_extract_markdown_extension_alias() {
        let mut file = NamedTempFile::with_suffix(".markdown").unwrap();
        writeln!(file, "## Section").unwrap();
        let path = file.path().to_owned();

        let result = extract_to_markdown(&path).unwrap();
        assert_eq!(result.mime_type, "text/markdown");
    }

    #[test]
    fn test_extract_plain_text_file() {
        let mut file = NamedTempFile::with_suffix(".txt").unwrap();
        writeln!(file, "Hello, world!").unwrap();
        let path = file.path().to_owned();

        let result = extract_to_markdown(&path).unwrap();
        assert!(result.content.contains("Hello, world!"));
        assert_eq!(result.mime_type, "text/plain");
    }

    #[test]
    fn test_extract_text_extension_alias() {
        let mut file = NamedTempFile::with_suffix(".text").unwrap();
        writeln!(file, "plain content").unwrap();
        let path = file.path().to_owned();

        let result = extract_to_markdown(&path).unwrap();
        assert_eq!(result.mime_type, "text/plain");
    }

    #[test]
    #[cfg(not(feature = "extract"))]
    fn test_non_text_without_extract_feature_returns_error() {
        let file = NamedTempFile::with_suffix(".pdf").unwrap();
        let path = file.path().to_owned();

        let result = extract_to_markdown(&path);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("'extract' feature is required"));
    }

    #[test]
    fn test_content_is_deterministic_for_text() {
        let content = "# Deterministic\n\nSame content every time.\n";
        let mut file = NamedTempFile::with_suffix(".md").unwrap();
        file.write_all(content.as_bytes()).unwrap();
        let path = file.path().to_owned();

        let result1 = extract_to_markdown(&path).unwrap();
        let result2 = extract_to_markdown(&path).unwrap();
        assert_eq!(result1.content, result2.content);
    }
}
