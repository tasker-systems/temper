//! Body-source resolution for `--body` on resource create/update. Used by
//! both local-mode (rewriting vault files) and cloud-mode (PATCH body trio)
//! write paths.
//!
//! Resolution order, first match wins:
//! 1. `--body @<path>` — read file contents; ignore stdin.
//! 2. `--body -` — read stdin explicitly. Errors if stdin is a TTY.
//! 3. Implicit: stdin if non-TTY *and* input is actually ready; else None
//!    (caller decides fallback).
//!
//! The implicit branch (3) is a convenience so `cat tmpfile.md | temper
//! resource update <ref>` works without `--body -`. It must NOT block forever:
//! when temper is spawned with stdin connected to a non-TTY pipe that is open
//! but idle (common under agent/CI harnesses), an unconditional
//! `read_to_string` blocks until the harness kills the process. The implicit
//! branch therefore polls for readiness first — a genuine pipe has data ready
//! immediately, while an idle pipe is treated as "no body provided". Explicit
//! `--body -` keeps its blocking contract: the user asked for stdin.

use std::io::Read;

use crate::error::{Result, TemperError};

/// How long the implicit (no-`--body`-flag) stdin auto-detect waits for input to
/// become available before concluding no body was piped. A genuine pipe (`cat x |
/// temper …`) has data ready in well under a millisecond, so this delay is only
/// ever paid when stdin is an open-but-idle non-TTY (e.g. a pipe an agent harness
/// leaves connected) — the case that previously hung until the caller's timeout.
const STDIN_POLL_TIMEOUT_MS: i32 = 300;

/// Returns Ok(Some(body)) if a body was resolved, Ok(None) for "no body
/// available" (TTY stdin, an idle non-TTY stdin, or no flag), Err on resolution
/// failure.
///
/// `stdin_ready` is consulted only for the implicit branch (no flag, non-TTY
/// stdin): it reports whether stdin has input ready to read, so an open-but-idle
/// pipe is treated as no-body rather than blocking. It is evaluated lazily and is
/// never called for `--body @path`/`--body -` (which have explicit semantics).
pub fn resolve_body_source<R: Read>(
    flag: Option<&str>,
    stdin_is_tty: bool,
    mut stdin_reader: R,
    stdin_ready: impl FnOnce() -> bool,
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
            // Implicit auto-detect: only read when stdin is a non-TTY that
            // actually has input ready. An open-but-idle non-TTY pipe (no piped
            // body) is treated as "no body provided" instead of blocking on a
            // read that may never reach EOF.
            if !stdin_is_tty && stdin_ready() {
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

/// Whether real stdin (fd 0) has input ready within `STDIN_POLL_TIMEOUT_MS`.
/// This is the production `stdin_ready` for [`resolve_body_source`]'s implicit
/// branch — pass it as `stdin_has_input_within` at the call site.
#[cfg(unix)]
pub fn stdin_has_input_within() -> bool {
    fd_has_input_within(libc::STDIN_FILENO, STDIN_POLL_TIMEOUT_MS)
}

/// Non-unix fallback: preserve the historical blocking-read behavior (no poll
/// primitive available). Reports "ready" so the implicit branch reads as before.
#[cfg(not(unix))]
pub fn stdin_has_input_within() -> bool {
    true
}

/// Returns true if `fd` has input (data, EOF, or hangup) ready within
/// `timeout_ms`. A `false` return means the fd is open but idle — reading it
/// would block. On an unexpected poll error (near-impossible for a valid fd) we
/// report `true` so the caller falls back to the read rather than silently
/// dropping a piped body.
#[cfg(unix)]
fn fd_has_input_within(fd: std::os::fd::RawFd, timeout_ms: i32) -> bool {
    loop {
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pfd` is a single valid, stack-owned `pollfd`; `poll` reads
        // `nfds`=1 entries through the pointer and writes back only `revents`. It
        // does not retain the pointer past the call.
        let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue; // interrupted by a signal — retry the poll
            }
            return true; // unexpected error: don't gate, let the read proceed
        }
        // rc == 0 → timed out (idle); rc > 0 → POLLIN/POLLHUP/POLLERR is set.
        return rc > 0;
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    /// Implicit branch with input available (the common `cat x | temper` case).
    fn ready() -> bool {
        true
    }
    /// Implicit branch with stdin open-but-idle (the harness hang case).
    fn idle() -> bool {
        false
    }

    #[test]
    fn resolves_body_at_path_explicit() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), "# From file").unwrap();
        let path_str = format!("@{}", temp.path().display());
        let result = resolve_body_source(
            Some(path_str.as_str()),
            /*stdin_is_tty:*/ true,
            Cursor::new(b""),
            ready,
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
            ready,
        )
        .unwrap();
        assert_eq!(result.unwrap(), "# From stdin");
    }

    #[test]
    fn explicit_dash_ignores_readiness_gate() {
        // `--body -` is an explicit request for stdin: it must read regardless of
        // the readiness probe (which is only for the implicit branch).
        let result = resolve_body_source(
            Some("-"),
            /*stdin_is_tty:*/ false,
            Cursor::new(b"# From stdin"),
            idle,
        )
        .unwrap();
        assert_eq!(result.unwrap(), "# From stdin");
    }

    #[test]
    fn resolves_explicit_dash_errors_on_tty() {
        let result = resolve_body_source(
            Some("-"),
            /*stdin_is_tty:*/ true,
            Cursor::new(b""),
            ready,
        );
        assert!(result.is_err());
    }

    #[test]
    fn implicit_uses_stdin_when_non_tty_and_ready() {
        let result = resolve_body_source(
            None,
            /*stdin_is_tty:*/ false,
            Cursor::new(b"# Implicit"),
            ready,
        )
        .unwrap();
        assert_eq!(result.unwrap(), "# Implicit");
    }

    #[test]
    fn implicit_returns_none_when_stdin_not_ready() {
        // The hang regression guard: a non-TTY stdin that is open but idle (no
        // input ready) must be treated as no-body — the reader is NOT drained,
        // even though bytes would be available, because the gate short-circuits.
        let result = resolve_body_source(
            None,
            /*stdin_is_tty:*/ false,
            Cursor::new(b"# Would block in production"),
            idle,
        )
        .unwrap();
        assert!(
            result.is_none(),
            "idle non-TTY stdin must resolve to no-body, not block"
        );
    }

    #[test]
    fn implicit_returns_none_when_tty_and_no_flag() {
        let result =
            resolve_body_source(None, /*stdin_is_tty:*/ true, Cursor::new(b""), ready).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn implicit_returns_none_for_empty_stdin() {
        // An empty implicit stdin (no bytes) is treated as "no body provided".
        // This prevents spawned threads (test harness) with unconnected stdin
        // from accidentally issuing a body-replacing empty PATCH.
        // For an explicit empty body, callers use `--body -`.
        let result =
            resolve_body_source(None, /*stdin_is_tty:*/ false, Cursor::new(b""), ready).unwrap();
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
            ready,
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
        let result = resolve_body_source(
            Some("-"),
            /*stdin_is_tty:*/ false,
            Cursor::new(b""),
            ready,
        );
        let err = result.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("empty"),
            "expected empty-body error, got: {msg}"
        );
    }

    // --- readiness probe (the OS poll), tested on controlled pipes so it never
    // depends on the test harness's fd 0 ---

    #[cfg(unix)]
    fn make_pipe() -> (std::os::fd::RawFd, std::os::fd::RawFd) {
        let mut fds = [0 as libc::c_int; 2];
        // SAFETY: `fds` is a valid 2-element array; `pipe` writes the read/write
        // fds into it and returns 0 on success.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe() failed");
        (fds[0], fds[1])
    }

    #[cfg(unix)]
    fn close_fd(fd: std::os::fd::RawFd) {
        // SAFETY: closing a fd we own exactly once.
        unsafe {
            libc::close(fd);
        }
    }

    #[cfg(unix)]
    #[test]
    fn fd_idle_pipe_reports_not_ready() {
        let (read_fd, write_fd) = make_pipe();
        // No writer activity: the read end is open but has no data and no EOF.
        assert!(
            !fd_has_input_within(read_fd, 50),
            "an open, idle pipe must report not-ready"
        );
        close_fd(write_fd);
        close_fd(read_fd);
    }

    #[cfg(unix)]
    #[test]
    fn fd_pipe_with_data_reports_ready() {
        let (read_fd, write_fd) = make_pipe();
        let byte = b"x";
        // SAFETY: writing one byte into the write end we own.
        let n = unsafe { libc::write(write_fd, byte.as_ptr() as *const libc::c_void, 1) };
        assert_eq!(n, 1, "write() failed");
        assert!(
            fd_has_input_within(read_fd, 50),
            "a pipe with pending data must report ready"
        );
        close_fd(write_fd);
        close_fd(read_fd);
    }

    #[cfg(unix)]
    #[test]
    fn fd_pipe_eof_reports_ready() {
        let (read_fd, write_fd) = make_pipe();
        // Closing the write end signals EOF on the read end: poll reports ready
        // (POLLHUP/readable), and the subsequent read returns 0 bytes.
        close_fd(write_fd);
        assert!(
            fd_has_input_within(read_fd, 50),
            "a pipe at EOF must report ready so the read can drain/return empty"
        );
        close_fd(read_fd);
    }
}
