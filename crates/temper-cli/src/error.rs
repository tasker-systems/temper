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

// There is deliberately **no** `impl From<ClientError> for TemperError`, here or
// anywhere in the workspace. Do not add one.
//
// It would be a genuine ergonomic win — every `client.foo().await?` would just
// work — and that is exactly the problem. `ClientError` carries structure that
// `TemperError` can only preserve if the conversion is deliberate:
// `SystemAccessRequired(details)` must survive as its own variant for the
// enriched 403 renderer to fire, and `is_network()` must survive to distinguish
// a down server from a rejecting one. A blanket `From` makes every `?` a silent
// flattening site, and the loss is invisible at the call site.
//
// This is not hypothetical. Two rival hand-written lifters once disagreed on
// exactly those two properties, so which guidance a gated user saw depended on
// which helper a call site happened to import — the enriched access-gate block
// was unreachable on every path a normal user hits, for months, while its unit
// tests stayed green. See PR #486.
//
// The single lifter is `actions::runtime::client_err_to_temper`. Because no
// `From` impl exists, every conversion must be written by hand and is therefore
// greppable — which is the only reason auditing this surface is tractable at
// all. Adding the impl would not just risk a regression; it would remove the
// property that lets anyone find the next one.

/// Result alias for CLI commands that can raise a [`CliError`].
pub type CliResult<T> = std::result::Result<T, CliError>;
