//! Body-source resolution for `--body` on resource create/update. Used by
//! both local-mode (rewriting vault files) and cloud-mode (PATCH body trio)
//! write paths.
//!
//! Resolution order, first match wins:
//! 1. `--body @<path>` — read file contents; ignore stdin.
//! 2. `--body -` — read stdin explicitly. Errors if stdin is a TTY.
//! 3. Implicit: stdin if non-TTY; else None (caller decides fallback).

use std::io::Read;

use crate::error::{Result, TemperError};

/// Returns Ok(Some(body)) if a body was resolved, Ok(None) for "no body
/// available" (TTY stdin, no flag), Err on resolution failure.
pub fn resolve_body_source<R: Read>(
    flag: Option<&str>,
    stdin_is_tty: bool,
    mut stdin_reader: R,
) -> Result<Option<String>> {
    match flag {
        Some(s) if s.starts_with('@') => {
            let path = &s[1..];
            let content = std::fs::read_to_string(path)
                .map_err(|e| TemperError::Vault(format!("read --body @{path}: {e}")))?;
            if content.is_empty() {
                return Err(TemperError::Project(format!(
                    "--body @{path} resolved to empty content; refusing to write empty body"
                )));
            }
            Ok(Some(content))
        }
        Some("-") => {
            if stdin_is_tty {
                return Err(TemperError::Project(
                    "--body - requires non-TTY stdin".to_string(),
                ));
            }
            let mut buf = String::new();
            stdin_reader
                .read_to_string(&mut buf)
                .map_err(|e| TemperError::Vault(format!("read stdin: {e}")))?;
            if buf.is_empty() {
                return Err(TemperError::Project(
                    "--body - resolved to empty stdin; refusing to write empty body".to_string(),
                ));
            }
            Ok(Some(buf))
        }
        Some(other) => Err(TemperError::Project(format!(
            "--body argument must be '-' or '@<path>', got: {other}"
        ))),
        None => {
            if !stdin_is_tty {
                let mut buf = String::new();
                stdin_reader
                    .read_to_string(&mut buf)
                    .map_err(|e| TemperError::Vault(format!("read stdin: {e}")))?;
                // Treat implicit empty stdin as "no body provided". An empty
                // implicit stdin most commonly means no pipe was connected
                // (e.g., a spawned thread in a test harness). For an explicit
                // empty body, callers use `--body -`.
                if buf.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(buf))
                }
            } else {
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn resolves_body_at_path_explicit() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), "# From file").unwrap();
        let path_str = format!("@{}", temp.path().display());
        let result = resolve_body_source(
            Some(path_str.as_str()),
            /*stdin_is_tty:*/ true,
            Cursor::new(b""),
        )
        .unwrap();
        assert_eq!(result.unwrap(), "# From file");
    }

    #[test]
    fn resolves_explicit_dash_reads_stdin() {
        let result = resolve_body_source(
            Some("-"),
            /*stdin_is_tty:*/ false,
            Cursor::new(b"# From stdin"),
        )
        .unwrap();
        assert_eq!(result.unwrap(), "# From stdin");
    }

    #[test]
    fn resolves_explicit_dash_errors_on_tty() {
        let result = resolve_body_source(Some("-"), /*stdin_is_tty:*/ true, Cursor::new(b""));
        assert!(result.is_err());
    }

    #[test]
    fn implicit_uses_stdin_when_non_tty() {
        let result = resolve_body_source(
            None,
            /*stdin_is_tty:*/ false,
            Cursor::new(b"# Implicit"),
        )
        .unwrap();
        assert_eq!(result.unwrap(), "# Implicit");
    }

    #[test]
    fn implicit_returns_none_when_tty_and_no_flag() {
        let result = resolve_body_source(None, /*stdin_is_tty:*/ true, Cursor::new(b"")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn implicit_returns_none_for_empty_stdin() {
        // An empty implicit stdin (no bytes) is treated as "no body provided".
        // This prevents spawned threads (test harness) with unconnected stdin
        // from accidentally issuing a body-replacing empty PATCH.
        // For an explicit empty body, callers use `--body -`.
        let result = resolve_body_source(None, /*stdin_is_tty:*/ false, Cursor::new(b"")).unwrap();
        assert!(
            result.is_none(),
            "empty implicit stdin must be treated as no-body"
        );
    }

    #[test]
    fn errors_when_at_path_file_is_empty() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), "").unwrap();
        let path_str = format!("@{}", temp.path().display());
        let result = resolve_body_source(
            Some(path_str.as_str()),
            /*stdin_is_tty:*/ true,
            Cursor::new(b""),
        );
        let err = result.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("empty"),
            "expected empty-body error, got: {msg}"
        );
    }

    #[test]
    fn errors_when_explicit_dash_stdin_is_empty() {
        let result = resolve_body_source(Some("-"), /*stdin_is_tty:*/ false, Cursor::new(b""));
        let err = result.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("empty"),
            "expected empty-body error, got: {msg}"
        );
    }
}
