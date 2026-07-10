pub use temper_core::error::{Result, TemperError};

/// CLI-only error surface for commands whose failures don't belong in the
/// shared [`TemperError`] vocabulary.
///
/// `temper update`'s self-install failures (subprocess, checksum, atomic swap,
/// cargo-build refusal) are the motivating case: they never originate on a
/// server surface, so folding an `Install` variant into `temper-core`'s
/// `TemperError` would push install semantics onto every crate that consumes it
/// (`temper-api`, `temper-services`, `temper-mcp`) — surfaces that never
/// install anything. Keeping the variant here contains that blast radius.
///
/// The [`From<TemperError>`] pass-through means ordinary `?`-propagation of core
/// errors still works inside a CLI command that returns [`CliResult`]; only the
/// genuinely CLI-local failures use [`CliError::Install`].
#[derive(Debug)]
pub enum CliError {
    /// A self-update / installer failure (installer subprocess, checksum,
    /// atomic swap) or a precondition refusal (`cargo install` build).
    Install(String),
    /// A core error propagated unchanged (network/API version resolution,
    /// render, etc.).
    Temper(TemperError),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Install(msg) => write!(f, "Install error: {msg}"),
            CliError::Temper(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CliError::Install(_) => None,
            CliError::Temper(e) => Some(e),
        }
    }
}

impl From<TemperError> for CliError {
    fn from(e: TemperError) -> Self {
        CliError::Temper(e)
    }
}

/// Result alias for CLI commands that can raise a [`CliError`].
pub type CliResult<T> = std::result::Result<T, CliError>;
