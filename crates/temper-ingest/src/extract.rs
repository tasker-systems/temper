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
    let result = extract_file(path, None, &config)
        .await
        .map_err(|e| map_kreuzberg_error(path, &e.to_string()))?;

    Ok(ExtractionResult {
        content: result.content,
        mime_type: result.mime_type.into_owned(),
    })
}

/// Turn a kreuzberg extraction failure into a legible `EmbedError`.
///
/// The bundled extractor is built without the PDF capability, so a `.pdf` source fails with a
/// generic "Unsupported format: application/pdf" — which gives the caller no hint that PDF simply
/// isn't compiled into this binary (issue #420 item 2). Detect that specific case and say so, with
/// an actionable workaround; every other failure keeps its original message. The guard is narrow
/// (extension `pdf` AND an unsupported-format error) so that once PDF support lands, a genuinely
/// corrupt PDF still surfaces its real error rather than this one.
#[cfg(feature = "extract")]
fn map_kreuzberg_error(path: &Path, err: &str) -> EmbedError {
    let is_pdf = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    if is_pdf && err.to_ascii_lowercase().contains("unsupported") {
        return EmbedError::Extraction(format!(
            "PDF support is not built into this binary: cannot extract '{}'. \
             Convert it to text or markdown first (e.g. `pdftotext file.pdf out.txt`) \
             and pass that with --from, or provide the text via --body.",
            path.display()
        ));
    }
    EmbedError::Extraction(format!("failed to extract '{}': {}", path.display(), err))
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

    #[cfg(feature = "extract")]
    #[test]
    fn pdf_unsupported_format_gets_a_legible_message() {
        // A .pdf failing with an unsupported-format error means PDF isn't compiled in.
        let err = map_kreuzberg_error(
            Path::new("/tmp/report.pdf"),
            "Unsupported format: application/pdf",
        );
        let msg = err.to_string();
        assert!(
            msg.contains("PDF support is not built into this binary"),
            "expected the PDF-not-built message, got: {msg}"
        );
        assert!(
            msg.contains("pdftotext"),
            "should suggest a workaround: {msg}"
        );
    }

    #[cfg(feature = "extract")]
    #[test]
    fn pdf_non_unsupported_error_keeps_its_real_message() {
        // A different .pdf failure (e.g. once PDF support lands and a file is corrupt) must keep
        // its own error, not be mislabeled as "not built in".
        let err = map_kreuzberg_error(Path::new("/tmp/corrupt.pdf"), "xref table is damaged");
        let msg = err.to_string();
        assert!(
            msg.contains("xref table is damaged") && !msg.contains("not built into this binary"),
            "corrupt-PDF error must survive verbatim, got: {msg}"
        );
    }

    #[cfg(feature = "extract")]
    #[test]
    fn non_pdf_unsupported_error_is_not_pdf_labeled() {
        // An unsupported non-PDF format must not borrow the PDF message.
        let err = map_kreuzberg_error(Path::new("/tmp/thing.xyz"), "Unsupported format: xyz");
        let msg = err.to_string();
        assert!(
            !msg.contains("PDF support"),
            "non-PDF unsupported format must not claim to be PDF, got: {msg}"
        );
    }
}
