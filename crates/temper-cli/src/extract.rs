//! Document extraction — delegates to temper-ingest.

use std::path::Path;

use crate::error::{Result, TemperError};

pub use temper_ingest::extract::ExtractionResult;

pub async fn extract_to_markdown(path: &Path) -> Result<ExtractionResult> {
    temper_ingest::extract::extract_to_markdown(path)
        .await
        .map_err(|e| TemperError::Extraction(e.to_string()))
}
