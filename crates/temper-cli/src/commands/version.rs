//! `temper version [--checksum]` — version reporting and running-binary
//! self-attestation.
//!
//! clap injects the terse `temper --version` / `temper -V` for free (the root
//! `#[command(version = ...)]` in `cli.rs`). This subcommand is the richer,
//! `OutputFormat`-aware surface: it renders a typed [`VersionReport`] as
//! JSON/TOON and, with `--checksum`, folds in the SHA-256 of the running
//! binary resolved via [`std::env::current_exe`].
//!
//! **The checksum is deliberately NOT the published release checksum.** The
//! release pipeline's `.sha256` sidecar is computed over the whole archive
//! (`temper-v<ver>-<triple>.tar.gz`, which also ships `lib/libonnxruntime.*`),
//! not the bare binary — so a locally-computed binary hash will never equal
//! the published archive checksum. This surface is pure self-attestation of
//! the installed binary; verifying a downloaded archive against the published
//! sidecar is `temper update`'s job. The `CHECKSUM_NOTE` carried in the
//! output makes that distinction explicit so no caller mistakes the two.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{Result, TemperError};
use crate::format::OutputFormat;

/// The compiled crate version (`CARGO_PKG_VERSION`) — the same value clap's
/// `--version` / `-V` reports. A test in this module pins it to the repo-root
/// `/VERSION` release source of truth so the two can never silently diverge.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Disclaimer carried in `--checksum` output. Load-bearing: it prevents any
/// caller from reading the binary hash as the published archive checksum.
const CHECKSUM_NOTE: &str = "SHA-256 of the running binary only. The published release \
    `.sha256` sidecar is computed over the whole archive (temper-v<ver>-<triple>.tar.gz, which \
    also ships lib/libonnxruntime.*), not this bare binary — the two will not match. \
    `temper update` verifies the archive checksum at install time.";

/// Top-level `temper version` output. `checksum` is present only when
/// `--checksum` was passed (skipped in serialization otherwise), so the
/// default shape stays a single-field `{ "version": "x.y.z" }`.
#[derive(Debug, Serialize)]
pub struct VersionReport {
    version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    checksum: Option<BinaryChecksum>,
}

/// Self-attestation block for the running binary. Field names are chosen to
/// read as "the binary's own hash" (`binary_sha256`, `binary_path`), never a
/// bare `sha256` that could be mistaken for the archive checksum.
#[derive(Debug, Serialize)]
pub struct BinaryChecksum {
    algorithm: &'static str,
    binary_sha256: String,
    binary_path: String,
    note: &'static str,
}

/// Compute the SHA-256 of the currently-running binary, resolved via
/// [`std::env::current_exe`]. Mirrors the `Sha256::digest` pattern in
/// `commands/skill.rs::compute_config_hash`. Returns `(hex_digest, path)`.
pub fn compute_self_checksum() -> Result<(String, String)> {
    let exe = std::env::current_exe()
        .map_err(|e| TemperError::Config(format!("cannot resolve current executable: {e}")))?;
    let bytes = std::fs::read(&exe)
        .map_err(|e| TemperError::Config(format!("cannot read {}: {e}", exe.display())))?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    Ok((digest, exe.display().to_string()))
}

/// `temper version [--checksum]`.
pub fn run(checksum: bool, fmt: OutputFormat) -> Result<()> {
    let checksum = if checksum {
        let (binary_sha256, binary_path) = compute_self_checksum()?;
        Some(BinaryChecksum {
            algorithm: "sha256",
            binary_sha256,
            binary_path,
            note: CHECKSUM_NOTE,
        })
    } else {
        None
    };

    let report = VersionReport {
        version: VERSION,
        checksum,
    };

    let rendered = crate::format::render(&report, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The compiled crate version and the repo-root `/VERSION` (the release
    /// source of truth `release-tag.yml` derives the `v{VERSION}` git tag from)
    /// must be the same string. If someone bumps one without the other,
    /// `temper --version` reports a stale number against a correctly-tagged
    /// release. This is the "cheap insurance" that makes the two provably one.
    ///
    /// `include_str!` resolves relative to THIS file
    /// (`crates/temper-cli/src/commands/version.rs`), so repo root is four
    /// directories up.
    #[test]
    fn crate_version_matches_repo_version_file() {
        let repo_version = include_str!("../../../../VERSION").trim();
        assert_eq!(
            VERSION, repo_version,
            "crate version ({VERSION}) and /VERSION ({repo_version}) diverge — \
             bump both together (see RELEASING.md)."
        );
    }

    /// `compute_self_checksum` returns a 64-char lowercase hex SHA-256 and a
    /// non-empty resolved path (here, the test binary itself). Mirrors the
    /// hash-shape coverage in `skill.rs`.
    #[test]
    fn self_checksum_is_hex_sha256_with_path() {
        let (digest, path) = compute_self_checksum().expect("checksum of test binary");
        assert_eq!(digest.len(), 64, "sha256 hex is 64 chars: {digest}");
        assert!(
            digest.chars().all(|c| c.is_ascii_hexdigit()),
            "digest must be hex: {digest}"
        );
        assert!(!path.is_empty(), "current_exe path must resolve");
    }

    /// The default (no `--checksum`) shape omits the `checksum` key entirely,
    /// so `temper version` stays a clean single-field object.
    #[test]
    fn checksum_key_absent_when_not_requested() {
        let report = VersionReport {
            version: VERSION,
            checksum: None,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(
            !json.contains("checksum"),
            "no checksum key when not requested: {json}"
        );
        assert!(json.contains("\"version\""), "version key present: {json}");
    }

    /// With `--checksum`, the rendered payload carries the binary hash, its
    /// path, and the disclaimer — and the disclaimer must NOT imply equivalence
    /// to the published archive checksum.
    #[test]
    fn checksum_report_serializes_with_archive_disclaimer() {
        let report = VersionReport {
            version: VERSION,
            checksum: Some(BinaryChecksum {
                algorithm: "sha256",
                binary_sha256: "deadbeef".to_string(),
                binary_path: "/usr/bin/temper".to_string(),
                note: CHECKSUM_NOTE,
            }),
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("binary_sha256"), "hash field present: {json}");
        assert!(json.contains("binary_path"), "path field present: {json}");
        // The disclaimer must reference the archive and negate a match.
        assert!(
            CHECKSUM_NOTE.contains("archive"),
            "note must mention the archive"
        );
        assert!(
            CHECKSUM_NOTE.contains("will not match"),
            "note must disclaim archive-checksum equivalence"
        );
    }
}
