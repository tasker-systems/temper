//! Styled terminal output for temper.
//!
//! Uses `anstyle` for ANSI style definitions and `anstream` for auto-detecting
//! terminal capabilities. Output gracefully degrades to plain text when piped
//! or when the terminal doesn't support colors.

mod styles;

use std::io::Write;

pub use styles::clap_styles;

use styles::{DIM, ERROR, HEADER, HINT, LABEL, SUCCESS, WARNING};

/// Print a success message (green checkmark prefix).
pub fn success(msg: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    writeln!(out, "{SUCCESS}✓{SUCCESS:#} {SUCCESS}{msg}{SUCCESS:#}").ok();
}

/// Print an error message to stderr (red X prefix).
pub fn error(msg: impl std::fmt::Display) {
    let mut out = anstream::stderr().lock();
    writeln!(out, "{ERROR}✗ {msg}{ERROR:#}").ok();
}

/// Print a warning message (yellow exclamation prefix).
pub fn warning(msg: impl std::fmt::Display) {
    let mut out = anstream::stderr().lock();
    writeln!(out, "{WARNING}! {msg}{WARNING:#}").ok();
}

/// Print a section header (bold).
pub fn header(msg: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    writeln!(out, "{HEADER}{msg}{HEADER:#}").ok();
}

/// Print a labeled value ("  Label: value" with the label bolded).
pub fn label(name: impl std::fmt::Display, value: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    writeln!(out, "  {LABEL}{name}:{LABEL:#} {value}").ok();
}

/// Print dimmed/muted text (for secondary information).
pub fn dim(msg: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    writeln!(out, "{DIM}{msg}{DIM:#}").ok();
}

/// Print a hint/suggestion (dimmed, for guidance text).
///
/// Goes to **stderr**, not stdout: temper defaults to JSON output on a non-TTY
/// stdout (how agents invoke it), and a hint written there corrupts the payload.
/// Guidance is for humans; the payload is for parsers.
pub fn hint(msg: impl std::fmt::Display) {
    let mut out = anstream::stderr().lock();
    writeln!(out, "{HINT}{msg}{HINT:#}").ok();
}

/// Print a status line with a colored icon based on health/status.
pub fn status_icon(healthy: bool, msg: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    if healthy {
        writeln!(out, "  {SUCCESS}✓{SUCCESS:#} {msg}").ok();
    } else {
        writeln!(out, "  {ERROR}✗{ERROR:#} {msg}").ok();
    }
}

/// Print a list item with a bullet prefix.
pub fn item(msg: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    writeln!(out, "  • {msg}").ok();
}

/// Print a blank line.
pub fn blank() {
    let mut out = anstream::stdout().lock();
    writeln!(out).ok();
}

/// Print plain text to stdout (for output that doesn't need styling).
pub fn plain(msg: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
    writeln!(out, "{msg}").ok();
}

/// Print unstyled text to **stderr** — the diagnostic twin of [`plain`].
///
/// For the prose lines of a multi-line *diagnostic* block. A block opened by
/// [`error`] (stderr) and closed by [`hint`] (stderr) must not put its middle
/// on stdout: redirecting one stream would leave the reader with a fragment,
/// and on a JSON-by-default stdout the fragment corrupts the payload. A block
/// belongs to one stream.
pub fn plain_err(msg: impl std::fmt::Display) {
    let mut out = anstream::stderr().lock();
    writeln!(out, "{msg}").ok();
}

/// Print a blank line to **stderr** — the diagnostic twin of [`blank`].
///
/// Use inside a stderr block so its spacing travels with it; see [`plain_err`].
pub fn blank_err() {
    let mut out = anstream::stderr().lock();
    writeln!(out).ok();
}

/// Print a success message to **stderr** — the diagnostic twin of [`success`].
///
/// For a command whose "success" output is *entirely prose* — `auth
/// request-access` is the motivating case: it takes no `fmt` parameter and
/// never calls `format::render`, so it has no payload to put on stdout. The
/// familiar "payload on stdout, guidance on stderr" split is only a defense
/// when a payload actually exists; where none does, every line is guidance and
/// belongs on stderr. Check for the `render` call before reaching for [`success`].
pub fn success_err(msg: impl std::fmt::Display) {
    let mut out = anstream::stderr().lock();
    writeln!(out, "{SUCCESS}✓{SUCCESS:#} {SUCCESS}{msg}{SUCCESS:#}").ok();
}

/// Print dimmed/muted text to **stderr** — the diagnostic twin of [`dim`].
///
/// Secondary information accompanying a stderr block, so it travels with the
/// block rather than fragmenting across streams; see [`plain_err`].
pub fn dim_err(msg: impl std::fmt::Display) {
    let mut out = anstream::stderr().lock();
    writeln!(out, "{DIM}{msg}{DIM:#}").ok();
}

/// Print inline progress to stderr (no trailing newline).
pub fn progress(msg: impl std::fmt::Display) {
    let mut out = anstream::stderr().lock();
    write!(out, "{DIM}{msg}{DIM:#}").ok();
}

/// Print a newline-terminated progress line to stderr (dimmed).
///
/// For step-by-step progress an agent or CI job tails line-by-line — e.g. the
/// per-segment `N of M` output during a long segmented ingest. On stderr (never
/// stdout) so it never corrupts the JSON document a caller parses on stdout.
pub fn progress_line(msg: impl std::fmt::Display) {
    let mut out = anstream::stderr().lock();
    writeln!(out, "{DIM}{msg}{DIM:#}").ok();
}
