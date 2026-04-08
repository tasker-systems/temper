//! Styled terminal output for temper.
//!
//! Uses `anstyle` for ANSI style definitions and `anstream` for auto-detecting
//! terminal capabilities. Output gracefully degrades to plain text when piped
//! or when the terminal doesn't support colors.

mod styles;
pub mod table;

use std::io::Write;

pub use styles::clap_styles;
pub use table::{Alignment, Column, TableRenderer};

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
pub fn hint(msg: impl std::fmt::Display) {
    let mut out = anstream::stdout().lock();
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

/// Print inline progress to stderr (no trailing newline).
pub fn progress(msg: impl std::fmt::Display) {
    let mut out = anstream::stderr().lock();
    write!(out, "{DIM}{msg}{DIM:#}").ok();
}
