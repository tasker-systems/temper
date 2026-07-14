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

    #[cfg(feature = "extract")]
    #[tokio::test]
    async fn extracts_a_text_layer_pdf_verbatim() {
        // A real text-layer PDF (pdf-lib producer, base-14 Helvetica/WinAnsi). Every line of the
        // source text must come back character-for-character: a lossy extraction is a bug, not a
        // reason to loosen this assertion.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/simple.pdf");
        let r = extract_to_markdown(&path).await.expect("pdf must extract");

        assert_eq!(r.mime_type, "application/pdf");

        // pdfium separates lines with CRLF. That is the extractor's real output, so assert it
        // rather than normalizing it away.
        let expected = [
            "Temper Cloud Architecture",
            "The upload pipeline processes files through four stages:",
            "1. Extract text content from uploaded files",
            "2. Chunk the extracted text by markdown headers",
            "3. Generate 768-dimensional vector embeddings",
            "4. Store chunks with embeddings in PostgreSQL",
            "Each chunk includes a content hash for deduplication",
            "and a header path for hierarchical context.",
        ]
        .join("\r\n");
        assert_eq!(r.content.trim(), expected);
    }

    #[tokio::test]
    #[cfg(not(feature = "extract"))]
    async fn test_non_text_without_feature() {
        let f = NamedTempFile::with_suffix(".pdf").unwrap();
        assert!(extract_to_markdown(f.path()).await.is_err());
    }

    #[cfg(feature = "extract")]
    #[tokio::test]
    async fn corrupt_pdf_surfaces_its_real_error() {
        // A PDF that pdfium cannot parse must report why, naming the file. Driven through the
        // public path rather than a helper fed a fabricated message, so it can only pass if
        // production really behaves this way.
        let mut f = NamedTempFile::with_suffix(".pdf").unwrap();
        write!(f, "%PDF-1.7\nnot actually a pdf").unwrap();
        f.flush().unwrap();

        let err = extract_to_markdown(f.path())
            .await
            .expect_err("a corrupt pdf must not extract");
        let msg = err.to_string();
        assert!(
            msg.contains("failed to extract") && msg.contains(&f.path().display().to_string()),
            "a corrupt PDF must surface its real error and name the file, got: {msg}"
        );
    }

    #[cfg(feature = "extract")]
    #[tokio::test]
    async fn genuinely_unsupported_format_reports_itself() {
        let mut f = NamedTempFile::with_suffix(".xyz").unwrap();
        write!(f, "\x00\x01\x02 not a document").unwrap();
        f.flush().unwrap();

        let err = extract_to_markdown(f.path())
            .await
            .expect_err("an unknown binary format must not extract");
        assert!(err.to_string().contains("failed to extract"), "got: {err}");
    }
}
