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
pub async fn extract_to_markdown(path: &Path) -> Result<ExtractionResult> {
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
        "html" | "htm" => {
            let html = std::fs::read_to_string(path)?;
            let result = html_to_markdown_rs::convert(&html, None).map_err(|e| {
                EmbedError::Extraction(format!(
                    "failed to convert HTML '{}': {}",
                    path.display(),
                    e
                ))
            })?;
            Ok(ExtractionResult {
                content: result.content.unwrap_or_default(),
                mime_type: "text/markdown".to_string(),
            })
        }
        _ => extract_with_kreuzberg(path).await,
    }
}

#[cfg(feature = "extract")]
async fn extract_with_kreuzberg(path: &Path) -> Result<ExtractionResult> {
    use kreuzberg::{extract_file, ExtractionConfig};

    let config = ExtractionConfig::default();
    let result = extract_file(path, None, &config).await.map_err(|e| {
        EmbedError::Extraction(format!("failed to extract '{}': {}", path.display(), e))
    })?;

    Ok(ExtractionResult {
        content: result.content,
        mime_type: result.mime_type.into_owned(),
    })
}

#[cfg(not(feature = "extract"))]
async fn extract_with_kreuzberg(path: &Path) -> Result<ExtractionResult> {
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

    #[tokio::test]
    async fn test_extract_markdown() {
        let mut f = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(f, "# Hello").unwrap();
        let r = extract_to_markdown(f.path()).await.unwrap();
        assert!(r.content.contains("# Hello"));
        assert_eq!(r.mime_type, "text/markdown");
    }

    #[tokio::test]
    async fn test_extract_plain_text() {
        let mut f = NamedTempFile::with_suffix(".txt").unwrap();
        writeln!(f, "Hello").unwrap();
        let r = extract_to_markdown(f.path()).await.unwrap();
        assert!(r.content.contains("Hello"));
        assert_eq!(r.mime_type, "text/plain");
    }

    #[tokio::test]
    async fn test_extract_html() {
        let mut f = NamedTempFile::with_suffix(".html").unwrap();
        write!(
            f,
            "<html><body><h1>Title</h1><p>Hello world</p></body></html>"
        )
        .unwrap();
        let r = extract_to_markdown(f.path()).await.unwrap();
        assert!(
            r.content.contains("# Title"),
            "expected markdown heading, got: {}",
            r.content
        );
        assert!(r.content.contains("Hello world"));
        assert!(!r.content.contains("<h1>"));
        assert_eq!(r.mime_type, "text/markdown");
    }

    #[tokio::test]
    #[cfg(not(feature = "extract"))]
    async fn test_non_text_without_feature() {
        let f = NamedTempFile::with_suffix(".pdf").unwrap();
        assert!(extract_to_markdown(f.path()).await.is_err());
    }
}
