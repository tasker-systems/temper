use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbedError {
    #[error("extraction error: {0}")]
    Extraction(String),

    #[error("embedding error: {0}")]
    Embedding(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, EmbedError>;
