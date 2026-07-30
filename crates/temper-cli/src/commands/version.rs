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
//! trusting the local copy, AND verifies the release attestation over THAT
//! SAME MANIFEST.** The offline check compares an install dir against a
//! manifest *in that same dir* — an actor who replaced the binary could
//! replace the manifest beside it too. `--online` closes that gap by
//! fetching `temper-v{VERSION}-{triple}.manifest.json` from the release's own
//! GitHub download path (mirroring `install.sh`'s asset naming exactly, see
//! `published_manifest_url`) and comparing against *that*.
//!
//! Once the per-file comparison agrees, the attestation check that follows
//! MUST be keyed on the manifest's own digest, never the archive's. The
//! object whose contents this verdict actually depends on is the manifest —
//! the archive is never inspected on this path — so an attacker with
//! release-asset write who tampers the manifest (to match a tampered
//! install) while leaving the genuine, still-attested archive `.sha256`
//! sidecar untouched must not be able to borrow the archive's real
//! attestation to vouch for a manifest it says nothing about.
//! `build-cli-binaries.yml`'s `attest-build-provenance` step lists the
//! manifest as its own `subject-path` entry alongside the archive, so
//! `/attestations/sha256:{manifest_digest}` resolves independently — see
//! `verify_manifest_attestation`, which hashes the EXACT bytes
//! `fetch_published_manifest_bytes` returned and that the comparison just
//! used, never a second, later fetch of "the same" manifest (that would
//! reopen a TOCTOU gap between what was compared and what was attested).
//! This is what makes `ONLINE_VERIFY_NOTE`'s claim true rather than
//! aspirational. A host whose OS/arch temper does not ship, a network
//! failure, or an attestation failure of any class, all render
//! [`manifest::Verdict::Unverifiable`] — never an error, and never a false
//! `Verified`.
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

use crate::attestation_fetch::{self, AttestationVerifyError};
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
/// published manifest AND verifying the release attestation together prove.
/// It closes the "an attacker replaced both files" gap `OFFLINE_VERIFY_NOTE`
/// names, and — as of [`online_verdict`] — it is now also a genuine signature
/// check, not just a manifest re-fetch. Deliberately says "the manifest's own
/// digest", not "the archive's": the manifest is the object this verdict's
/// comparison actually inspects, so it is the object whose provenance must be
/// checked — a genuine, still-attested archive says nothing about a tampered
/// manifest sitting beside it.
const ONLINE_VERIFY_NOTE: &str = "Online verification re-fetches the release manifest published \
    on GitHub for this exact version and host, rather than trusting the copy installed beside \
    the binary — so a compromised local manifest can no longer hide a compromised binary. Once \
    the manifest agrees, it also verifies GitHub's build-provenance attestation over the sha256 \
    of that exact manifest (not the archive) against a pinned Sigstore trust root — the manifest \
    is its own attested subject in the release workflow, and it is the object this check's \
    comparison actually depends on. A failure at any step (network, trust root, or identity) \
    renders `unverifiable`, never a false `verified`.";

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
    /// Present only on the `--online` path, and only when that path reached a
    /// point where planting the offline baseline was warranted. The offline
    /// path has nothing to plant from, so it always omits this.
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline: Option<BaselineReport>,
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

/// The reason rendered offline when no manifest sits beside the binary. A
/// constant so the one property that matters about it stays assertable: it must
/// not claim to know WHY the manifest is absent.
///
/// It used to assert "this is not a manifest-bearing release install", which is
/// **false for an entire cohort of installs**. A binary at v0.2.6 or earlier
/// self-updates by piping its OWN `include_str!`-embedded `install.sh`
/// (`update.rs`), and that older script writes no manifest — so updating from
/// v0.2.6 to a manifest-bearing release lands that release's genuine bytes in a
/// directory with no baseline beside them. The install really is
/// manifest-bearing; only the baseline is missing, and telling that user their
/// install "is not a release install" is simply wrong.
///
/// Absence has at least four causes now — a `cargo install` build, a Windows
/// install, a release published before manifests shipped, and that update hop —
/// and this code can distinguish none of them. So it enumerates rather than
/// concludes, and names the recovery, which is the same in every case.
const NO_LOCAL_MANIFEST_REASON: &str =
    "no release manifest beside this binary, so an offline check has nothing to compare against. \
     This is not a finding about your install — several ordinary shapes land here: a \
     `cargo install` build (not a release artifact at all), a Windows install, a release \
     published before per-file manifests shipped, or a self-update performed by a pre-manifest \
     binary (which installs a manifest-bearing release using its own older installer, leaving no \
     baseline behind). Run `temper version --verify --online` to check the published manifest and \
     its attestation; on success that also plants the baseline, after which this offline check \
     works on its own.";

/// Outcome of the baseline plant attempted by `--verify --online`. Reported so
/// the write is never silent: it happens inside a command whose name reads
/// read-only, and a user who is told their install is `verified` deserves to
/// know whether the offline check will work next time.
///
/// Deliberately a plain struct with a boolean rather than an internally-tagged
/// enum. An internally-tagged enum with a newtype variant compiles here and
/// then fails at runtime on the render path — that exact defect shipped once in
/// this module's `Verdict` and was caught only by serializing. There is no
/// reason to re-open the shape that caused it for a two-state report.
#[derive(Debug, Serialize)]
pub struct BaselineReport {
    planted: bool,
    path: String,
    /// Present only when `planted` is false, naming what stopped the write.
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    note: &'static str,
}

const BASELINE_PLANTED_NOTE: &str =
    "The published manifest was persisted beside the binary as the offline baseline. It was \
     written only because the per-file comparison agreed AND the build-provenance attestation \
     over those exact bytes verified — so this is a manifest whose provenance was checked, not a \
     record of trust in the local install. `temper version --verify` now works offline.";

const BASELINE_NOT_WRITTEN_NOTE: &str =
    "The verdict above stands on its own — it did not depend on writing anything. Only the \
     offline baseline was not established, so `temper version --verify` will keep reporting \
     `unverifiable` until an online check can write it.";

/// Persist an attestation-verified published manifest beside the binary as the
/// offline baseline, but only if no baseline is there already.
///
/// **Why this is not the retro-fit it resembles.** Minting a manifest for an
/// archive that was never attested would manufacture provenance. This does the
/// opposite: the caller has already fetched the published manifest, matched it
/// file-by-file against this directory, and verified GitHub's build-provenance
/// attestation over the sha256 of these exact bytes. Every fact the baseline
/// will later assert offline has been checked against a signature at the moment
/// it is written. What is being persisted is a verified result, not an
/// assumption.
///
/// **Never overwrites.** An existing baseline that disagrees with the attested
/// one is a different situation — offline `--verify` renders it `Mismatch`,
/// which is loud, actionable, and points here. Silently replacing it would
/// convert that signal into a repair the user never saw. Absence is the case
/// this closes.
///
/// **Best-effort by construction.** A read-only or root-owned install directory
/// must never turn a `Verified` verdict into an error: the verdict was earned
/// by checks that had nothing to do with writing a file. Failure is reported,
/// not raised.
fn plant_baseline(dir: &Path, manifest_bytes: &[u8]) -> BaselineReport {
    let path = dir.join(manifest::MANIFEST_FILENAME);
    let path_str = path.display().to_string();

    if path.exists() {
        return BaselineReport {
            planted: false,
            path: path_str,
            reason: Some(
                "a baseline is already present beside this binary; left untouched".to_string(),
            ),
            note: BASELINE_NOT_WRITTEN_NOTE,
        };
    }

    match std::fs::write(&path, manifest_bytes) {
        Ok(()) => BaselineReport {
            planted: true,
            path: path_str,
            reason: None,
            note: BASELINE_PLANTED_NOTE,
        },
        Err(e) => BaselineReport {
            planted: false,
            path: path_str,
            reason: Some(format!("could not write the baseline: {e}")),
            note: BASELINE_NOT_WRITTEN_NOTE,
        },
    }
}

/// Build the offline verdict for an install directory. Absent manifest =>
/// [`Verdict::Unverifiable`], never [`Verdict::Mismatch`] — "we cannot tell"
/// is not "it is wrong", the same distinction `CARGO_REFUSAL` draws at
/// `update.rs:58`. See [`NO_LOCAL_MANIFEST_REASON`] for why that reason
/// enumerates causes rather than naming one.
///
/// `include_checksum` folds in the running binary's self-attestation
/// ([`BinaryChecksum`], via [`compute_self_checksum`]) when the caller also
/// passed `--checksum` — `--verify --checksum` must report both facts, not
/// whichever flag happened to be checked first.
fn build_verify_report(dir: &Path, include_checksum: bool) -> Result<VerifyReport> {
    let verdict = match manifest::load_from_dir(dir) {
        Some(m) => manifest::verify_dir(&m, dir),
        None => Verdict::Unverifiable {
            reason: NO_LOCAL_MANIFEST_REASON.to_string(),
        },
    };
    finish_verify_report(dir, verdict, include_checksum, OFFLINE_VERIFY_NOTE, None)
}

/// `temper version --verify --online`'s report builder. Mirrors
/// [`build_verify_report`] in every respect except where the verdict comes
/// from: [`online_verdict`] (a freshly re-fetched published manifest) instead
/// of [`manifest::load_from_dir`] (the local copy beside the binary), and it
/// carries [`ONLINE_VERIFY_NOTE`] rather than [`OFFLINE_VERIFY_NOTE`].
fn build_verify_report_online(dir: &Path, include_checksum: bool) -> Result<VerifyReport> {
    let outcome = online_verdict(dir);
    // Plant only from bytes the outcome certifies — see `OnlineOutcome`, whose
    // whole job is that `verified_manifest_bytes` cannot be `Some` unless the
    // final verdict was `Verified`. This call site never re-derives that
    // condition, so the two cannot drift apart.
    let baseline = outcome
        .verified_manifest_bytes
        .as_deref()
        .map(|bytes| plant_baseline(dir, bytes));
    finish_verify_report(
        dir,
        outcome.verdict,
        include_checksum,
        ONLINE_VERIFY_NOTE,
        baseline,
    )
}

/// What [`online_verdict`] returns: the verdict, plus — and **only** when that
/// verdict is [`Verdict::Verified`] — the exact published manifest bytes that
/// earned it.
///
/// The pairing is the point. [`plant_baseline`] must never be handed bytes that
/// did not clear both the per-file comparison and the attestation check, or it
/// would persist an unverified manifest as a permanent record of verification —
/// which is precisely the false provenance this whole surface exists to avoid.
/// Rather than restate that condition at the call site (where it could drift
/// from the condition [`finish_online_verdict`] actually applies),
/// [`OnlineOutcome::verified_with`] derives it from the finished verdict, and
/// every other constructor cannot produce bytes at all.
struct OnlineOutcome {
    verdict: Verdict,
    verified_manifest_bytes: Option<Vec<u8>>,
}

impl OnlineOutcome {
    /// An `Unverifiable` outcome from a reason string. Carries no bytes — there
    /// is nothing verified to carry.
    fn unverifiable(reason: String) -> Self {
        Self {
            verdict: Verdict::Unverifiable { reason },
            verified_manifest_bytes: None,
        }
    }

    /// Wrap an already-settled verdict that was reached WITHOUT a completed
    /// attestation check (an unmapped host, a fetch failure, or a manifest
    /// mismatch). Never carries bytes, even if some were fetched: a mismatching
    /// manifest is exactly the thing that must not become a baseline.
    fn from_verdict(verdict: Verdict) -> Self {
        Self {
            verdict,
            verified_manifest_bytes: None,
        }
    }

    /// The only constructor that can carry bytes, and it still refuses unless
    /// the FINAL verdict is `Verified` — `finish_online_verdict` downgrades a
    /// matching manifest to `Unverifiable` when its attestation fails, and that
    /// downgrade must take the bytes with it.
    fn verified_with(verdict: Verdict, manifest_bytes: Vec<u8>) -> Self {
        let verified_manifest_bytes =
            matches!(verdict, Verdict::Verified).then_some(manifest_bytes);
        Self {
            verdict,
            verified_manifest_bytes,
        }
    }
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
    baseline: Option<BaselineReport>,
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
        baseline,
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
///
/// `pub(crate)`: `update.rs` reuses this exact mapping (rather than a second
/// copy) to pick the archive it downloads for the running host.
pub(crate) fn shipped_target_triple() -> Option<&'static str> {
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

/// Re-fetch the published release manifest's RAW BYTES for `version`/`target`
/// over the network — kept verbatim, not just parsed, so [`online_verdict`]
/// can bind its attestation check to the exact bytes just compared
/// (`verify_manifest_attestation`) rather than re-fetching "the same"
/// manifest a second time, which would reopen a TOCTOU gap between what was
/// compared and what was attested. Matches the HTTP posture
/// `update.rs::resolve_latest_tag` establishes via the shared
/// `attestation_fetch::release_http_client_builder` — a `temper-cli/{VERSION}`
/// user agent, 10s connect / 30s total timeout, and HTTP 403 mapped to an
/// explicit rate-limit message rather than a bare 403 that reads like an auth
/// wall.
async fn fetch_published_manifest_bytes(
    client: &reqwest::Client,
    version: &str,
    target: &str,
) -> Result<Vec<u8>> {
    let url = published_manifest_url(version, target);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| TemperError::Network(format!("fetching published manifest {url}: {e}")))?;
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
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| TemperError::Network(format!("reading published manifest body: {e}")))?;
    Ok(bytes.to_vec())
}

/// Parse the published manifest's raw bytes into a [`manifest::ReleaseManifest`].
/// Pure and split out from the fetch so [`online_verdict`] can hash and parse
/// the exact same byte buffer without a second fetch.
fn parse_manifest_bytes(bytes: &[u8]) -> Result<manifest::ReleaseManifest> {
    serde_json::from_slice(bytes)
        .map_err(|e| TemperError::Api(format!("parsing published manifest JSON: {e}")))
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

/// The (digest, tag) identity `verify_manifest_attestation` checks the
/// release attestation against, computed from EXACTLY `manifest_bytes` — the
/// same buffer [`online_verdict`] just ran through [`parse_manifest_bytes`]
/// and [`manifest::verify_dir`] for the per-file comparison. Split out as a
/// pure function so the property this whole restructuring exists to
/// guarantee — "the digest checked is the digest of what was compared,
/// never a re-fetch, and never the sibling archive's digest" — is
/// unit-tested without a network call.
fn manifest_attestation_identity(manifest_bytes: &[u8]) -> (String, String) {
    (
        format!("{:x}", Sha256::digest(manifest_bytes)),
        format!("v{VERSION}"),
    )
}

/// Verify GitHub's build-provenance attestation over the sha256 of
/// `manifest_bytes` — deliberately the MANIFEST's digest, not the archive's.
/// The manifest is what [`online_verdict`]'s per-file comparison actually
/// depends on (the archive is never inspected on this path), so it is the
/// manifest's own provenance that must be checked: an attacker with
/// release-asset write who tampers the manifest to match a tampered install,
/// while leaving the genuine, still-attested archive `.sha256` sidecar
/// untouched, must not be able to borrow the archive's real attestation to
/// vouch for a manifest it says nothing about. `build-cli-binaries.yml`
/// attests the manifest as its own `subject-path` entry alongside the
/// archive, so `/attestations/sha256:{manifest_digest}` resolves
/// independently of the archive's.
async fn verify_manifest_attestation(
    client: &reqwest::Client,
    manifest_bytes: &[u8],
) -> std::result::Result<(), AttestationVerifyError> {
    let (digest, tag) = manifest_attestation_identity(manifest_bytes);
    attestation_fetch::verify_release_attestation_online(client, &digest, &tag).await
}

/// Combine an already-computed manifest verdict with an already-computed
/// attestation result into the final online verdict. Pure and injectable —
/// mirrors [`verdict_from_manifest_result`]'s own testability pattern.
/// Attestation only ever overrides a `Verified` manifest verdict (to
/// `Unverifiable` on failure); a `Mismatch` or a manifest-fetch
/// `Unverifiable` passes through untouched — attestation can only ever
/// *downgrade* a `Verified` manifest match, never rescue or override a
/// verdict the manifest comparison already settled.
fn finish_online_verdict(
    manifest_verdict: Verdict,
    attestation_result: std::result::Result<(), AttestationVerifyError>,
) -> Verdict {
    match manifest_verdict {
        Verdict::Verified => match attestation_result {
            Ok(()) => Verdict::Verified,
            Err(e) => Verdict::Unverifiable {
                reason: e.to_string(),
            },
        },
        other => other,
    }
}

/// Build the online verdict for `dir`: re-fetch the published manifest's raw
/// bytes for the running version and host triple, compare them against the
/// install dir, and — only once that comparison is `Verified` — verify
/// GitHub's build-provenance attestation over the sha256 of THOSE SAME BYTES
/// (never the sibling archive's digest, and never a second, later fetch of
/// "the same" manifest — see `verify_manifest_attestation` for why the
/// manifest, not the archive, is the correct object here). An unmapped host,
/// a network failure, or an attestation failure of any class all render
/// [`Verdict::Unverifiable`], never an error and never a false `Verified`.
///
/// The manifest check alone only ever proved corruption/drift, not
/// provenance — the gap `ONLINE_VERIFY_NOTE` used to admit before this
/// module verified anything. Attestation is only worth checking once the
/// manifest itself agrees with the install: a manifest mismatch or fetch
/// failure already answers the question on its own, and running an
/// attestation lookup on top of that would just blur one honest verdict
/// with another (and would waste a network round-trip on a verdict that's
/// already decided).
fn online_verdict(dir: &Path) -> OnlineOutcome {
    let Some(target) = shipped_target_triple() else {
        return OnlineOutcome::unverifiable(unmapped_host_reason(
            std::env::consts::OS,
            std::env::consts::ARCH,
        ));
    };

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            return OnlineOutcome::unverifiable(format!(
                "could not start a runtime for online verification: {e}"
            ));
        }
    };

    rt.block_on(async {
        let client = match attestation_fetch::release_http_client_builder(VERSION).build() {
            Ok(c) => c,
            Err(e) => {
                return OnlineOutcome::unverifiable(format!("building HTTP client: {e}"));
            }
        };

        let manifest_bytes = match fetch_published_manifest_bytes(&client, VERSION, target).await {
            Ok(bytes) => bytes,
            Err(e) => {
                return OnlineOutcome::from_verdict(verdict_from_manifest_result(
                    VERSION,
                    target,
                    Err(e),
                    dir,
                ));
            }
        };

        let manifest_verdict = verdict_from_manifest_result(
            VERSION,
            target,
            parse_manifest_bytes(&manifest_bytes),
            dir,
        );

        // Skip the network round-trip entirely when the manifest itself
        // already disagrees — attestation cannot rescue a bad comparison,
        // and there is nothing to bind it to that would change the verdict.
        if !matches!(manifest_verdict, Verdict::Verified) {
            return OnlineOutcome::from_verdict(manifest_verdict);
        }

        let attestation_result = verify_manifest_attestation(&client, &manifest_bytes).await;
        OnlineOutcome::verified_with(
            finish_online_verdict(manifest_verdict, attestation_result),
            manifest_bytes,
        )
    })
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
            baseline: None,
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

    /// Manifest bytes shaped like a real published manifest, for the baseline
    /// tests below. Serialized rather than hand-written so a plant is provable
    /// to round-trip through `manifest::load_from_dir` — the consumer the
    /// baseline exists for.
    fn published_manifest_bytes() -> Vec<u8> {
        serde_json::to_vec_pretty(&manifest::ReleaseManifest {
            version: "0.3.0".to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            files: vec![manifest::ManifestEntry {
                path: "temper".to_string(),
                sha256: format!("{:x}", Sha256::digest(b"binary-bytes")),
                size: 12,
            }],
        })
        .unwrap()
    }

    /// The transition case this whole mechanism exists for: an install dir with
    /// the right bytes and NO baseline beside them (what a pre-manifest binary's
    /// self-update leaves behind). Planting must write the manifest verbatim and
    /// leave it readable by the offline path.
    ///
    /// Byte-equality is asserted, not just parseability: the attestation was
    /// verified over the sha256 of these EXACT bytes, so a baseline that merely
    /// re-serializes to something equivalent would no longer be the object whose
    /// provenance was checked.
    #[test]
    fn plant_baseline_writes_verbatim_when_absent_and_offline_verify_then_works() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("temper"), b"binary-bytes").unwrap();
        let bytes = published_manifest_bytes();

        let report = plant_baseline(tmp.path(), &bytes);
        assert!(report.planted, "expected a plant, got {report:?}");
        assert!(report.reason.is_none());

        let on_disk = std::fs::read(tmp.path().join(manifest::MANIFEST_FILENAME)).unwrap();
        assert_eq!(
            on_disk, bytes,
            "the planted baseline must be byte-identical to the attested bytes"
        );

        // The point of planting: the offline path now reaches a verdict on its
        // own instead of `Unverifiable`.
        let offline = build_verify_report(tmp.path(), false).unwrap();
        assert!(
            matches!(offline.verdict, Verdict::Verified),
            "offline verify should now be Verified, got {:?}",
            offline.verdict
        );
    }

    /// Planting must never overwrite. A baseline that disagrees with the
    /// attested manifest is a signal the user should SEE (offline `--verify`
    /// renders `Mismatch`); silently replacing it would erase that signal and
    /// perform a repair nobody was told about.
    ///
    /// The bite: the pre-existing bytes are distinguishable, and the assertion
    /// is that they SURVIVE — not merely that `planted` is false, which would
    /// also pass if the write happened and the flag were misreported.
    #[test]
    fn plant_baseline_never_overwrites_an_existing_baseline() {
        let tmp = tempfile::tempdir().unwrap();
        let existing = b"{\"version\":\"already-here\"}";
        std::fs::write(tmp.path().join(manifest::MANIFEST_FILENAME), existing).unwrap();

        let report = plant_baseline(tmp.path(), &published_manifest_bytes());

        assert!(!report.planted);
        assert!(
            report
                .reason
                .as_deref()
                .unwrap()
                .contains("already present"),
            "reason should say why: {:?}",
            report.reason
        );
        let on_disk = std::fs::read(tmp.path().join(manifest::MANIFEST_FILENAME)).unwrap();
        assert_eq!(on_disk, existing, "the existing baseline was overwritten");
    }

    /// A read-only install dir must not turn a `Verified` verdict into an
    /// error. The verdict was earned by a per-file comparison and a signature
    /// check; neither depended on writing a file, so a failed write reports
    /// itself and nothing more.
    #[test]
    #[cfg(unix)]
    fn plant_baseline_failure_is_reported_not_raised() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("readonly");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();

        let report = plant_baseline(&dir, &published_manifest_bytes());

        // Restore before the tempdir's own cleanup runs.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        assert!(!report.planted);
        assert!(
            report
                .reason
                .as_deref()
                .unwrap()
                .contains("could not write the baseline"),
            "reason should name the write failure: {:?}",
            report.reason
        );
    }

    /// **The load-bearing test of the plant path.** An attestation failure
    /// downgrades a matching manifest to `Unverifiable`, and that downgrade must
    /// take the bytes with it — otherwise a manifest that matched the install
    /// but whose signature did NOT verify would be persisted as the permanent
    /// offline record of "verified", manufacturing exactly the provenance this
    /// surface refuses to manufacture.
    ///
    /// This runs `finish_online_verdict` for real rather than constructing the
    /// downgraded verdict by hand, so it pins the actual composition
    /// `online_verdict` performs.
    #[test]
    fn attestation_failure_takes_the_bytes_with_it() {
        let bytes = published_manifest_bytes();

        let downgraded = finish_online_verdict(
            Verdict::Verified,
            Err(AttestationVerifyError::NotOurs(
                "signed for another repo".into(),
            )),
        );
        let outcome = OnlineOutcome::verified_with(downgraded, bytes.clone());

        assert!(
            matches!(outcome.verdict, Verdict::Unverifiable { .. }),
            "attestation failure must downgrade the verdict"
        );
        assert!(
            outcome.verified_manifest_bytes.is_none(),
            "an unattested manifest must never be available to plant"
        );

        // Control: the same bytes DO survive when the attestation verifies, so
        // the assertion above is about the attestation result and not about
        // `verified_with` simply never carrying bytes.
        let passed = finish_online_verdict(Verdict::Verified, Ok(()));
        let ok = OnlineOutcome::verified_with(passed, bytes);
        assert!(
            ok.verified_manifest_bytes.is_some(),
            "a verified, attested manifest must be plantable"
        );
    }

    /// A verdict settled before any attestation check ran — a mismatch, a fetch
    /// failure, an unmapped host — can never carry bytes. Constructed through
    /// the only constructor those paths use.
    #[test]
    fn verdicts_settled_without_attestation_carry_no_bytes() {
        let mismatch = OnlineOutcome::from_verdict(Verdict::Mismatch {
            mismatches: vec![manifest::Mismatch {
                path: "temper".to_string(),
                expected: "a".to_string(),
                actual: Some("b".to_string()),
            }],
        });
        assert!(mismatch.verified_manifest_bytes.is_none());

        let unreachable = OnlineOutcome::unverifiable("network down".to_string());
        assert!(unreachable.verified_manifest_bytes.is_none());
        assert!(matches!(unreachable.verdict, Verdict::Unverifiable { .. }));
    }

    /// `BaselineReport` must survive the render path, for the same reason
    /// `verify_report_serializes_for_every_verdict` exists: a shape that
    /// compiles and then fails at `serde_json::to_string` already shipped once
    /// in this module. Both states are serialized inside a full report.
    #[test]
    fn baseline_report_serializes_in_both_states() {
        let with_baseline = |baseline: BaselineReport| VerifyReport {
            version: VERSION,
            install_dir: "/opt/temper".to_string(),
            verdict: Verdict::Verified,
            checksum: None,
            baseline: Some(baseline),
            note: ONLINE_VERIFY_NOTE,
        };

        let planted = serde_json::to_string(&with_baseline(BaselineReport {
            planted: true,
            path: "/opt/temper/.temper-manifest.json".to_string(),
            reason: None,
            note: BASELINE_PLANTED_NOTE,
        }))
        .expect("planted baseline serializes");
        assert!(planted.contains("\"planted\":true"));
        assert!(
            !planted.contains("\"reason\""),
            "reason must be omitted when the plant succeeded: {planted}"
        );

        let refused = serde_json::to_string(&with_baseline(BaselineReport {
            planted: false,
            path: "/opt/temper/.temper-manifest.json".to_string(),
            reason: Some("could not write the baseline: permission denied".to_string()),
            note: BASELINE_NOT_WRITTEN_NOTE,
        }))
        .expect("refused baseline serializes");
        assert!(refused.contains("\"planted\":false"));
        assert!(refused.contains("permission denied"));
    }

    /// The offline path has nothing to plant from, so it must never report a
    /// baseline block at all — an absent key, not `"baseline":null`.
    #[test]
    fn offline_verify_reports_no_baseline_block() {
        let tmp = tempfile::tempdir().unwrap();
        let report = build_verify_report(tmp.path(), false).unwrap();
        assert!(report.baseline.is_none());
        let json = serde_json::to_string(&report).unwrap();
        assert!(
            !json.contains("baseline\":"),
            "offline report should carry no baseline key: {json}"
        );
    }

    /// F3: the absent-manifest reason must not assert a cause it cannot know.
    /// A pre-manifest binary self-updating to a manifest-bearing release lands
    /// here, and the old wording told that user their install "is not a
    /// manifest-bearing release install" — false, and it named no recovery.
    ///
    /// The bite is the negative assertion: the previous string would fail it.
    #[test]
    fn absent_manifest_reason_claims_no_cause_and_names_the_recovery() {
        assert!(
            !NO_LOCAL_MANIFEST_REASON.contains("this is not a manifest-bearing release install"),
            "the reason must not assert a cause it cannot distinguish"
        );
        assert!(
            NO_LOCAL_MANIFEST_REASON.contains("temper version --verify --online"),
            "the reason must name the recovery"
        );
        // The four shapes that land here, so a future edit that drops one is
        // visible rather than silently narrowing the enumeration back down.
        for shape in [
            "cargo install",
            "Windows install",
            "before per-file manifests shipped",
            "self-update performed by a pre-manifest binary",
        ] {
            assert!(
                NO_LOCAL_MANIFEST_REASON.contains(shape),
                "reason should still enumerate {shape:?}"
            );
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
        let report = finish_verify_report(tmp.path(), verdict, true, ONLINE_VERIFY_NOTE, None)
            .expect("checksum of test binary");
        assert!(report.checksum.is_some());
        assert_eq!(report.note, ONLINE_VERIFY_NOTE);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("binary_sha256"));
    }

    /// `manifest_attestation_identity` is the whole security property Finding
    /// 2 exists to fix: the digest passed to attestation verification MUST be
    /// the digest of the manifest bytes actually used for the comparison —
    /// never the sibling archive's digest, and never a re-fetch. This pins
    /// that it (a) is a real sha256 of the exact bytes given, computed the
    /// same way `Sha256::digest` is used everywhere else in this crate, (b)
    /// carries the `v`-prefixed tag, and (c) is sensitive to its input —
    /// different bytes must never collapse to the same digest, which would
    /// be the signature of a "always hash something else" bug.
    #[test]
    fn manifest_attestation_identity_hashes_the_exact_bytes_compared() {
        let bytes = br#"{"version":"0.3.0","target":"x86_64-unknown-linux-gnu","files":[]}"#;
        let (digest, tag) = manifest_attestation_identity(bytes);
        assert_eq!(digest, format!("{:x}", Sha256::digest(bytes)));
        assert_eq!(tag, format!("v{VERSION}"));

        let (other_digest, _) = manifest_attestation_identity(b"a completely different manifest");
        assert_ne!(
            digest, other_digest,
            "different manifest bytes must produce a different digest"
        );
    }

    /// `finish_online_verdict`'s composition rule, part 1: a `Verified`
    /// manifest match plus an `Ok` attestation is the ONLY path to an overall
    /// `Verified` — this is the positive case the security fix must still
    /// allow.
    #[test]
    fn finish_online_verdict_verified_manifest_and_ok_attestation_is_verified() {
        let verdict = finish_online_verdict(Verdict::Verified, Ok(()));
        assert!(matches!(verdict, Verdict::Verified));
    }

    /// `finish_online_verdict`'s composition rule, part 2 — THE LOAD-BEARING
    /// CASE for Finding 2: a `Verified` manifest match whose attestation
    /// check FAILS must render `Unverifiable`, carrying the attestation
    /// error's own text, never a silent `Verified`. Without this, a matching
    /// manifest alone (as an attacker who tampered the manifest to describe
    /// their own tampered install would produce) could pass as `Verified`
    /// regardless of what the attestation lookup found.
    #[test]
    fn finish_online_verdict_verified_manifest_but_failed_attestation_is_unverifiable() {
        let verdict = finish_online_verdict(
            Verdict::Verified,
            Err(AttestationVerifyError::NotOurs(
                "wrong identity".to_string(),
            )),
        );
        match verdict {
            Verdict::Unverifiable { reason } => assert!(reason.contains("not ours")),
            other => panic!("expected Unverifiable, got {other:?}"),
        }
    }

    /// `finish_online_verdict`'s composition rule, part 3: a manifest
    /// `Mismatch` passes straight through regardless of the attestation
    /// result — attestation can only ever downgrade a `Verified` manifest
    /// verdict, never rescue or override one the manifest comparison already
    /// settled. Deliberately passes `Ok(())` here to prove a "successful"
    /// attestation result cannot paper over a manifest mismatch.
    #[test]
    fn finish_online_verdict_manifest_mismatch_passes_through_regardless_of_attestation() {
        let mismatch = Verdict::Mismatch {
            mismatches: vec![manifest::Mismatch {
                path: "temper".into(),
                expected: "aa".into(),
                actual: Some("bb".into()),
            }],
        };
        let verdict = finish_online_verdict(mismatch, Ok(()));
        assert!(matches!(verdict, Verdict::Mismatch { .. }));
    }

    /// `online_verdict`'s composition rule: attestation is only consulted
    /// once the manifest itself agrees with the install. A manifest
    /// `Mismatch` must stand on its own — it is exercised here via
    /// `verdict_from_manifest_result` directly (no network call), and
    /// `online_verdict` never runs the attestation check on top of a verdict
    /// that already isn't `Verified` (see its `if !matches!` short-circuit,
    /// which passes non-`Verified` manifest verdicts straight through without
    /// a network round-trip).
    #[test]
    fn manifest_mismatch_stands_on_its_own_without_needing_attestation() {
        let tmp = tempfile::tempdir().unwrap();
        // A manifest mismatch must short-circuit — no attestation check is
        // even meaningful once the files themselves disagree.
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
        let manifest_verdict = verdict_from_manifest_result(
            "0.3.0",
            "x86_64-unknown-linux-gnu",
            Ok(published),
            tmp.path(),
        );
        assert!(matches!(manifest_verdict, Verdict::Mismatch { .. }));
    }

    /// `parse_manifest_bytes` round-trips the same JSON shape
    /// `fetch_published_manifest_bytes` would hand it — pins that hashing and
    /// parsing operate on the same raw representation (no re-encoding step
    /// that could make the hashed bytes differ from the parsed ones).
    #[test]
    fn parse_manifest_bytes_parses_the_same_bytes_that_get_hashed() {
        let manifest = manifest::ReleaseManifest {
            version: "0.3.0".to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            files: vec![manifest::ManifestEntry {
                path: "temper".to_string(),
                sha256: "a".repeat(64),
                size: 12,
            }],
        };
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let parsed = parse_manifest_bytes(&bytes).expect("valid manifest JSON parses");
        assert_eq!(parsed.version, manifest.version);
        assert_eq!(parsed.files.len(), 1);
        // The digest bound to this exact byte buffer, for the record — this
        // is what `online_verdict` passes to the attestation lookup.
        let (digest, _) = manifest_attestation_identity(&bytes);
        assert_eq!(digest, format!("{:x}", Sha256::digest(&bytes)));
    }
}
