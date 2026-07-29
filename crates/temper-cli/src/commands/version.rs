//! `temper version [--checksum] [--verify [--online]]` — version reporting,
//! running-binary self-attestation, and offline (or online) manifest
//! verification.
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
//! **`--verify --online` re-fetches the published manifest instead of
//! trusting the local copy.** The offline check compares an install dir
//! against a manifest *in that same dir* — an actor who replaced the binary
//! could replace the manifest beside it too. `--online` closes that gap by
//! fetching `temper-v{VERSION}-{triple}.manifest.json` from the release's own
//! GitHub download path (mirroring `install.sh`'s asset naming exactly, see
//! `published_manifest_url`) and comparing against *that*. It still does
//! not verify who produced the release — that is `temper update`'s
//! attestation check, or `gh attestation verify` run out-of-band (see
//! `ONLINE_VERIFY_NOTE`). A host whose OS/arch temper does not ship, or a
//! network failure, both render [`manifest::Verdict::Unverifiable`] — never
//! an error, and never a false `Verified`.
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

/// Disclaimer carried in `--verify --online` output. Load-bearing for the same
/// reason as `OFFLINE_VERIFY_NOTE`: it states plainly what re-fetching the
/// published manifest does and does not prove. It closes the "an attacker
/// replaced both files" gap `OFFLINE_VERIFY_NOTE` names, but it is still not a
/// signature check — that is `temper update`'s attestation verification, or
/// `gh attestation verify` run out-of-band against the release archive.
const ONLINE_VERIFY_NOTE: &str = "Online verification re-fetches the release manifest published \
    on GitHub for this exact version and host, rather than trusting the copy installed beside \
    the binary — so a compromised local manifest can no longer hide a compromised binary. It \
    does not verify who produced the release; that is `temper update`'s attestation check, or \
    `gh attestation verify` run out-of-band.";

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
    finish_verify_report(dir, verdict, include_checksum, OFFLINE_VERIFY_NOTE)
}

/// `temper version --verify --online`'s report builder. Mirrors
/// [`build_verify_report`] in every respect except where the verdict comes
/// from: [`online_verdict`] (a freshly re-fetched published manifest) instead
/// of [`manifest::load_from_dir`] (the local copy beside the binary), and it
/// carries [`ONLINE_VERIFY_NOTE`] rather than [`OFFLINE_VERIFY_NOTE`].
fn build_verify_report_online(dir: &Path, include_checksum: bool) -> Result<VerifyReport> {
    let verdict = online_verdict(dir);
    finish_verify_report(dir, verdict, include_checksum, ONLINE_VERIFY_NOTE)
}

/// Assemble the final [`VerifyReport`] from an already-computed `verdict`,
/// folding in the running binary's self-attestation when `include_checksum`
/// is set. Shared tail of [`build_verify_report`] and
/// [`build_verify_report_online`] so the two verdict sources (local file vs.
/// network fetch) do not each carry their own copy of the checksum-composition
/// logic.
fn finish_verify_report(
    dir: &Path,
    verdict: Verdict,
    include_checksum: bool,
    note: &'static str,
) -> Result<VerifyReport> {
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
        note,
    })
}

/// Map the running host to one of the three shipped release triples. Split
/// out from [`shipped_target_triple`] so the mapping itself is unit-testable
/// against explicit `(os, arch)` pairs, independent of whatever host actually
/// runs the test suite.
fn shipped_target_triple_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

/// The running host's shipped target triple, resolved from compile-time
/// `std::env::consts::{OS, ARCH}` — the same host the running binary was
/// built for. `None` means this host is not one of the three temper ships
/// release archives for; callers must render that as
/// [`Verdict::Unverifiable`], never an error and never a [`Verdict::Mismatch`]
/// — "we cannot tell" is not "it is wrong".
fn shipped_target_triple() -> Option<&'static str> {
    shipped_target_triple_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// The published manifest asset URL. Mirrors `install.sh`'s own
/// `ARCHIVE`/`MANIFEST`/`URL_BASE` construction (`scripts/install/install.sh`
/// lines ~117-122) exactly — `temper-v{version}-{target}.manifest.json` under
/// the tag's release download path. Kept as one function so the naming has a
/// single definition: a divergence here 404s at runtime, invisible to a test
/// that constructs its own expectation instead of deriving it from what the
/// installer actually uses.
fn published_manifest_url(version: &str, target: &str) -> String {
    format!(
        "https://github.com/tasker-systems/temper/releases/download/v{version}/\
         temper-v{version}-{target}.manifest.json"
    )
}

/// Re-fetch the published release manifest for `version`/`target` over the
/// network. Matches the HTTP posture `update.rs::resolve_latest_tag`
/// establishes — a `temper-cli/{VERSION}` user agent, 10s connect / 30s total
/// timeout (reqwest has no default timeout, so a black-holed network must not
/// hang forever), and HTTP 403 mapped to an explicit rate-limit message rather
/// than a bare 403 that reads like an auth wall — so this crate carries
/// exactly one HTTP client configuration, not a second one invented here.
/// Runs its own short-lived tokio runtime for the one request, same as
/// `resolve_latest_tag`, rather than making this module's `run` async.
fn fetch_published_manifest(version: &str, target: &str) -> Result<manifest::ReleaseManifest> {
    let url = published_manifest_url(version, target);
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .user_agent(format!("temper-cli/{VERSION}"))
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| TemperError::Network(format!("building HTTP client: {e}")))?;
        let resp =
            client.get(&url).send().await.map_err(|e| {
                TemperError::Network(format!("fetching published manifest {url}: {e}"))
            })?;
        // A 403 is almost always the unauthenticated rate limit (shared NAT/CI
        // IPs hit it) — flag it as transient rather than leaving a bare "403
        // Forbidden" that reads like an auth wall. Mirrors
        // `resolve_latest_tag`'s handling of the same status.
        if resp.status() == reqwest::StatusCode::FORBIDDEN {
            return Err(TemperError::Network(
                "GitHub rate-limited or forbidden (HTTP 403) while fetching the published \
                 manifest. Your install is unchanged — retry in a few minutes."
                    .to_string(),
            ));
        }
        if !resp.status().is_success() {
            return Err(TemperError::Network(format!(
                "fetching published manifest {url} returned HTTP {}",
                resp.status()
            )));
        }
        resp.json::<manifest::ReleaseManifest>()
            .await
            .map_err(|e| TemperError::Api(format!("parsing published manifest JSON: {e}")))
    })
}

/// Turn a manifest-fetch outcome into a [`Verdict`]. Pure and injectable — the
/// `Result` is passed in rather than fetched here — so the two distinct
/// failure shapes stay separately testable without a live network call: a
/// successful fetch that disagrees with the install dir renders
/// [`Verdict::Mismatch`] naming the files, while the fetch itself failing
/// renders [`Verdict::Unverifiable`] naming the network problem. Collapsing
/// those two into one message would blur "this check could not run" with
/// "this check ran and your install is wrong" — completely different user
/// actions.
fn verdict_from_manifest_result(
    version: &str,
    target: &str,
    result: Result<manifest::ReleaseManifest>,
    dir: &Path,
) -> Verdict {
    match result {
        Ok(published) => manifest::verify_dir(&published, dir),
        Err(e) => Verdict::Unverifiable {
            reason: format!(
                "could not reach the published manifest for v{version} ({target}): {e} — this \
                 says nothing about whether your install matches what was published, only that \
                 this check could not run. Retry, or fall back to the offline \
                 `temper version --verify`."
            ),
        },
    }
}

/// The reason rendered when the running host maps to none of the three
/// shipped triples. Split out from [`online_verdict`] so the message itself
/// is unit-testable against explicit `(os, arch)` inputs rather than only
/// through whatever host happens to run the test suite.
fn unmapped_host_reason(os: &str, arch: &str) -> String {
    format!(
        "no published manifest for this host ({os}-{arch}) — temper ships \
         aarch64-apple-darwin, x86_64-unknown-linux-gnu, and x86_64-pc-windows-msvc only, so \
         there is nothing published to compare against"
    )
}

/// Build the online verdict for `dir`: re-fetch the published manifest for
/// the running version and host triple, and compare against it — rather than
/// trusting the local copy an actor who replaced the binary could have
/// replaced too. An unmapped host and a network failure both render
/// [`Verdict::Unverifiable`], never an error and never a false `Verified`.
fn online_verdict(dir: &Path) -> Verdict {
    let Some(target) = shipped_target_triple() else {
        return Verdict::Unverifiable {
            reason: unmapped_host_reason(std::env::consts::OS, std::env::consts::ARCH),
        };
    };
    verdict_from_manifest_result(
        VERSION,
        target,
        fetch_published_manifest(VERSION, target),
        dir,
    )
}

/// `temper version [--checksum] [--verify [--online]]`.
///
/// `--verify` takes precedence over the plain [`VersionReport`] shape and
/// renders a [`VerifyReport`] instead — but the flags themselves compose:
/// `--verify --checksum` folds the binary self-attestation into the verify
/// report rather than silently dropping `--checksum`, and `--verify --online`
/// (clap enforces `online` only ever arrives alongside `verify` via
/// `requires = "verify"`) sources the verdict from a freshly re-fetched
/// published manifest instead of the local copy, while still composing with
/// `--checksum` the same way the offline path does.
pub fn run(checksum: bool, verify: bool, online: bool, fmt: OutputFormat) -> Result<()> {
    if verify {
        let install_dir = resolve_install_dir()?;
        let report = if online {
            build_verify_report_online(&install_dir, checksum)?
        } else {
            build_verify_report(&install_dir, checksum)?
        };
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

    /// The published-manifest URL must match the asset naming the release
    /// actually uses (`scripts/install/install.sh` lines ~117-122: `ARCHIVE`,
    /// `URL_BASE`, `MANIFEST`) — a mismatch here 404s at runtime and is
    /// invisible to a unit test that constructs its own expectation. This
    /// expected string is transcribed from install.sh's own construction
    /// (`URL_BASE="…/releases/download/${VERSION}"`,
    /// `MANIFEST="temper-${VERSION}-${TARGET}.manifest.json"`, where
    /// `VERSION` there already carries the `v` prefix), not invented.
    #[test]
    fn published_manifest_url_matches_release_asset_naming() {
        let url = published_manifest_url("0.3.0", "x86_64-unknown-linux-gnu");
        assert_eq!(
            url,
            "https://github.com/tasker-systems/temper/releases/download/v0.3.0/\
             temper-v0.3.0-x86_64-unknown-linux-gnu.manifest.json"
        );
    }

    /// The three shipped triples map correctly from `(os, arch)`.
    #[test]
    fn shipped_target_triple_maps_the_three_shipped_hosts() {
        assert_eq!(
            shipped_target_triple_for("macos", "aarch64"),
            Some("aarch64-apple-darwin")
        );
        assert_eq!(
            shipped_target_triple_for("linux", "x86_64"),
            Some("x86_64-unknown-linux-gnu")
        );
        assert_eq!(
            shipped_target_triple_for("windows", "x86_64"),
            Some("x86_64-pc-windows-msvc")
        );
    }

    /// A host temper does not ship for maps to `None` — the signal callers
    /// must render as `Unverifiable`, never an error and never `Mismatch`.
    #[test]
    fn shipped_target_triple_is_none_for_unmapped_hosts() {
        assert_eq!(shipped_target_triple_for("linux", "aarch64"), None);
        assert_eq!(shipped_target_triple_for("freebsd", "x86_64"), None);
        assert_eq!(shipped_target_triple_for("macos", "x86_64"), None);
    }

    /// The unmapped-host reason names the host and the three shipped triples
    /// — this is the exact message `online_verdict` renders (never an error,
    /// never a false `Verified`) when the running host is not one temper
    /// ships for. Calls the real production function, not a re-derived copy.
    #[test]
    fn unmapped_host_reason_names_the_host_and_shipped_triples() {
        let reason = unmapped_host_reason("freebsd", "x86_64");
        assert!(reason.contains("no published manifest"), "{reason}");
        assert!(reason.contains("freebsd-x86_64"), "{reason}");
        assert!(reason.contains("aarch64-apple-darwin"), "{reason}");
        assert!(reason.contains("x86_64-unknown-linux-gnu"), "{reason}");
        assert!(reason.contains("x86_64-pc-windows-msvc"), "{reason}");
    }

    /// A network failure renders `Unverifiable` with a reason that says the
    /// check could not run — distinct wording from a `Mismatch`, which would
    /// instead name disagreeing files. Exercised via
    /// `verdict_from_manifest_result` directly (no live network call), the
    /// same dependency-injection seam that keeps `update.rs::resolve_latest_tag`
    /// itself untested while its pure helpers are.
    #[test]
    fn network_failure_renders_unverifiable_not_mismatch_or_error() {
        let tmp = tempfile::tempdir().unwrap();
        let err = TemperError::Network("connection refused".to_string());
        let verdict =
            verdict_from_manifest_result("0.3.0", "x86_64-unknown-linux-gnu", Err(err), tmp.path());
        match verdict {
            Verdict::Unverifiable { reason } => {
                assert!(
                    reason.contains("could not reach"),
                    "reason must say the check could not run: {reason}"
                );
                assert!(
                    !reason.contains("disagrees"),
                    "a network failure must not be worded like a content disagreement: {reason}"
                );
            }
            other => panic!("expected Unverifiable, got {other:?}"),
        }
    }

    /// A successful fetch that matches the install dir renders `Verified` —
    /// the online path reuses `manifest::verify_dir` rather than a second
    /// hand-rolled comparison.
    #[test]
    fn successful_fetch_matching_dir_renders_verified() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("temper"), b"binary-bytes").unwrap();
        let published = manifest::ReleaseManifest {
            version: "0.3.0".to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            files: vec![manifest::ManifestEntry {
                path: "temper".to_string(),
                sha256: format!("{:x}", Sha256::digest(b"binary-bytes")),
                size: 12,
            }],
        };
        let verdict = verdict_from_manifest_result(
            "0.3.0",
            "x86_64-unknown-linux-gnu",
            Ok(published),
            tmp.path(),
        );
        assert!(matches!(verdict, Verdict::Verified));
    }

    /// A successful fetch that disagrees with the install dir renders
    /// `Mismatch` naming the file — distinguishable from the `Unverifiable`
    /// a network failure produces.
    #[test]
    fn successful_fetch_disagreeing_with_dir_renders_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("temper"), b"TAMPERED").unwrap();
        let published = manifest::ReleaseManifest {
            version: "0.3.0".to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            files: vec![manifest::ManifestEntry {
                path: "temper".to_string(),
                sha256: format!("{:x}", Sha256::digest(b"binary-bytes")),
                size: 12,
            }],
        };
        let verdict = verdict_from_manifest_result(
            "0.3.0",
            "x86_64-unknown-linux-gnu",
            Ok(published),
            tmp.path(),
        );
        match verdict {
            Verdict::Mismatch { mismatches } => assert_eq!(mismatches[0].path, "temper"),
            other => panic!("expected Mismatch, got {other:?}"),
        }
    }

    // `--online` without `--verify` is a usage error, not a silent no-op —
    // enforced declaratively via clap's `requires = "verify"` on the `online`
    // arg. Actually exercised (not just documented) in
    // `cli.rs::version_flag_tests::online_alone_is_a_usage_error`, which
    // parses `["temper", "version", "--online"]` through the real `Cli`
    // command and asserts it is rejected.

    /// `--verify --online` must compose with `--checksum` exactly like the
    /// offline path does: the running binary's self-attestation hash rides
    /// along regardless of where the verdict came from. Exercised via
    /// `finish_verify_report` directly — the shared tail
    /// `build_verify_report_online` delegates to — rather than through
    /// `build_verify_report_online` itself, which would perform a real
    /// network fetch on any host mapped to a shipped triple.
    #[test]
    fn online_report_composes_verdict_and_checksum_via_finish_verify_report() {
        let tmp = tempfile::tempdir().unwrap();
        let verdict = Verdict::Unverifiable {
            reason: "test".to_string(),
        };
        let report = finish_verify_report(tmp.path(), verdict, true, ONLINE_VERIFY_NOTE)
            .expect("checksum of test binary");
        assert!(report.checksum.is_some());
        assert_eq!(report.note, ONLINE_VERIFY_NOTE);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("binary_sha256"));
    }
}
