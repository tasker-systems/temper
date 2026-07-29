//! `temper version [--checksum] [--verify]` — version reporting,
//! running-binary self-attestation, and offline manifest verification.
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
//!
//! **`--verify` is a different, offline claim.** It checks every file the
//! release manifest (`crate::manifest`) lists against the install directory
//! beside the running binary — corruption and drift, not provenance. See the
//! `OFFLINE_VERIFY_NOTE` disclaimer for why that distinction is load-bearing.
//! `--verify` and `--checksum` compose: passed together, the verdict and the
//! running binary's self-attestation both appear in one payload — they are
//! complementary facts ("does this install match what was published" vs
//! "here is this binary's own hash"), not alternatives, so neither flag is
//! ever silently dropped in favor of the other.
//!
//! **Windows carries no manifest today**, by the design's own deferral
//! (`install.ps1` ships no `.temper-manifest.json`). `--verify` on Windows
//! therefore always resolves through the same `load_from_dir` -> `None` path
//! as a `cargo install` build and reports `unverifiable` — never `verified`.
//! That is emergent from "no manifest present" rather than a Windows-specific
//! branch, and is the intended behavior, not a gap.

use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{Result, TemperError};
use crate::format::OutputFormat;
use crate::manifest::{self, Verdict};

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

/// Disclaimer carried in offline `--verify` output. Load-bearing for the same
/// reason as `CHECKSUM_NOTE`: it stops a `verified` verdict from being read
/// as provenance it did not earn. Checking a directory against a manifest
/// *in that same directory* detects corruption and drift — an actor who
/// replaced the binary could replace the manifest beside it too.
const OFFLINE_VERIFY_NOTE: &str = "Offline verification compares the installed files against a \
    manifest in the same directory. It detects corruption and drift, but not an attacker, who \
    could replace both. Run `temper version --verify --online` for attestation-backed provenance.";

/// Offline verification report for `temper version --verify`. `checksum` is
/// present only when `--checksum` was *also* passed (skipped in
/// serialization otherwise) — mirrors the `VersionReport.checksum` pattern so
/// there is one convention for "optional attestation block" in this module,
/// not two. The two flags compose rather than compete: `--verify --checksum`
/// must not silently drop the checksum half of what was asked for.
#[derive(Debug, Serialize)]
pub struct VerifyReport {
    version: &'static str,
    install_dir: String,
    verdict: Verdict,
    #[serde(skip_serializing_if = "Option::is_none")]
    checksum: Option<BinaryChecksum>,
    note: &'static str,
}

/// Resolve the install directory of the running binary. Mirrors
/// `update.rs::detect_install_layout`: the on-PATH `temper` is a symlink into
/// the install dir, so the exe path must be canonicalized to the real file
/// first — otherwise `.parent()` resolves to `~/.local/bin`, not the install
/// dir the manifest lives beside, and every verdict would be wrong.
fn resolve_install_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe()
        .map_err(|e| TemperError::Config(format!("cannot resolve current executable: {e}")))?;
    let real = std::fs::canonicalize(&exe).unwrap_or(exe);
    real.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| TemperError::Config("running binary has no parent directory".into()))
}

/// Build the offline verdict for an install directory. Absent manifest =>
/// [`Verdict::Unverifiable`], never [`Verdict::Mismatch`] — "we cannot tell"
/// is not "it is wrong", the same distinction `CARGO_REFUSAL` draws at
/// `update.rs:58`. The reason names both manifest-less shapes that exist
/// today — a `cargo install` build and a Windows script install — so the
/// message stays accurate rather than implying only one of them.
///
/// `include_checksum` folds in the running binary's self-attestation
/// ([`BinaryChecksum`], via [`compute_self_checksum`]) when the caller also
/// passed `--checksum` — `--verify --checksum` must report both facts, not
/// whichever flag happened to be checked first.
fn build_verify_report(dir: &Path, include_checksum: bool) -> Result<VerifyReport> {
    let verdict = match manifest::load_from_dir(dir) {
        Some(m) => manifest::verify_dir(&m, dir),
        None => Verdict::Unverifiable {
            reason: "no release manifest beside this binary — this is not a manifest-bearing \
                     release install (e.g. a `cargo install` build, or a Windows install, which \
                     ships no manifest today), so there is nothing to verify against"
                .to_string(),
        },
    };
    let checksum = if include_checksum {
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
    Ok(VerifyReport {
        version: VERSION,
        install_dir: dir.display().to_string(),
        verdict,
        checksum,
        note: OFFLINE_VERIFY_NOTE,
    })
}

/// `temper version [--checksum] [--verify]`.
///
/// `--verify` takes precedence over the plain [`VersionReport`] shape and
/// renders a [`VerifyReport`] instead — but the two flags themselves compose:
/// `--verify --checksum` folds the binary self-attestation into the verify
/// report rather than silently dropping `--checksum`.
pub fn run(checksum: bool, verify: bool, fmt: OutputFormat) -> Result<()> {
    if verify {
        let install_dir = resolve_install_dir()?;
        let report = build_verify_report(&install_dir, checksum)?;
        let rendered = crate::format::render(&report, fmt)?;
        crate::output::plain(rendered);
        return Ok(());
    }

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

    /// A `cargo install` build has no manifest beside it. That must render as
    /// Unverifiable — never Mismatch. "We cannot tell" is not "it is wrong",
    /// the same distinction CARGO_REFUSAL draws at update.rs:58.
    #[test]
    fn absent_manifest_renders_unverifiable_not_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let report = build_verify_report(tmp.path(), false).unwrap();
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("unverifiable"), "got: {json}");
        assert!(
            !json.contains("mismatch"),
            "must not claim a mismatch: {json}"
        );
    }

    /// The offline verdict must carry its limitation. A `Verified` that reads
    /// as unqualified provenance overclaims: an actor who replaced the binary
    /// could replace the manifest beside it.
    #[test]
    fn offline_verdict_carries_its_limitation() {
        assert!(OFFLINE_VERIFY_NOTE.contains("same directory"));
        assert!(
            OFFLINE_VERIFY_NOTE.contains("not") && OFFLINE_VERIFY_NOTE.contains("attacker"),
            "the note must disclaim adversarial meaning"
        );
    }

    /// EVERY verdict variant must actually serialize inside a `VerifyReport`.
    /// This is the test whose absence let a real defect through upstream in
    /// `manifest::Verdict`: the original shape used newtype variants under
    /// internal tagging, which compiles and then fails at RUNTIME. A test
    /// that only pattern-matches the verdict cannot see that — the failure
    /// lives on the render path, so this test calls `serde_json::to_string`
    /// on a full report carrying each of the three verdicts.
    #[test]
    fn verify_report_serializes_for_every_verdict() {
        let base = |verdict: Verdict| VerifyReport {
            version: VERSION,
            install_dir: "/opt/temper".to_string(),
            verdict,
            checksum: None,
            note: OFFLINE_VERIFY_NOTE,
        };

        let verified =
            serde_json::to_string(&base(Verdict::Verified)).expect("Verified report serializes");
        assert!(verified.contains("\"verdict\":\"verified\""), "{verified}");

        let mismatch = serde_json::to_string(&base(Verdict::Mismatch {
            mismatches: vec![manifest::Mismatch {
                path: "temper".into(),
                expected: "aa".into(),
                actual: Some("bb".into()),
            }],
        }))
        .expect("Mismatch report serializes");
        assert!(mismatch.contains("\"verdict\":\"mismatch\""), "{mismatch}");
        assert!(mismatch.contains("\"path\":\"temper\""), "{mismatch}");

        let unverifiable = serde_json::to_string(&base(Verdict::Unverifiable {
            reason: "no release manifest beside this binary".into(),
        }))
        .expect("Unverifiable report serializes");
        assert!(
            unverifiable.contains("\"verdict\":\"unverifiable\""),
            "{unverifiable}"
        );
    }

    /// `build_verify_report` resolves a real directory (not just a bare
    /// `Verdict`) — this pins the `install_dir` field to the directory it was
    /// asked to check, independent of whatever the running test binary's own
    /// install directory happens to be.
    #[test]
    fn verify_report_install_dir_matches_requested_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let report = build_verify_report(tmp.path(), false).unwrap();
        assert_eq!(report.install_dir, tmp.path().display().to_string());
    }

    /// `--verify --checksum` must compose, not compete: the rendered payload
    /// carries BOTH the verdict tag and the running binary's self-attestation
    /// hash. This is the regression test for the defect the coordinator
    /// flagged — `--checksum` silently doing nothing when passed alongside
    /// `--verify` is exactly the kind of "flag with no effect and no
    /// diagnostic" the project's filter-surface-honesty goal forbids.
    #[test]
    fn verify_with_checksum_carries_both_verdict_and_binary_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let report = build_verify_report(tmp.path(), true).expect("checksum of test binary");
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"verdict\":\"unverifiable\""), "{json}");
        assert!(
            json.contains("binary_sha256"),
            "checksum must not be silently dropped when --checksum was requested: {json}"
        );
        assert!(report.checksum.is_some());
    }

    /// Without `--checksum`, `--verify` output stays free of the checksum
    /// key entirely — the same "absent, not null" convention `VersionReport`
    /// already uses for its own `checksum` field.
    #[test]
    fn verify_without_checksum_omits_the_checksum_key() {
        let tmp = tempfile::tempdir().unwrap();
        let report = build_verify_report(tmp.path(), false).unwrap();
        let json = serde_json::to_string(&report).unwrap();
        assert!(
            !json.contains("binary_sha256"),
            "no checksum key when not requested: {json}"
        );
    }
}
