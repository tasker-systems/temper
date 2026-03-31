//! Document extraction — delegates to temper-embed.

use std::path::Path;

use crate::error::{Result, TemperError};

pub use temper_embed::extract::ExtractionResult;

pub fn extract_to_markdown(path: &Path) -> Result<ExtractionResult> {
    temper_embed::extract::extract_to_markdown(path)
        .map_err(|e| TemperError::Extraction(e.to_string()))
}
