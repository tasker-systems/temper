//! `temper update` — in-place self-update for curl-script installs.
//!
//! # The load-bearing fact: an install is a directory, not a bare binary
//!
//! A release archive (`build-cli-binaries.yml`) ships more than the binary —
//! on unix it also carries `lib/libonnxruntime.*`, the version-matched ONNX
//! Runtime native lib the embed feature links against. The correct unit of
//! replacement is therefore the **whole install directory**, swapped
//! atomically, not "the binary swapped in place".
//!
//! # One installer, one truth
//!
//! Rather than reimplement target-triple detection, archive naming, the
//! dual-tool checksum verify, and the ORT-aware layout in Rust — a second copy
//! that would drift from a fresh install the instant either changed — this
//! command shells out to the *canonical* `scripts/install/install.sh`, embedded
//! at build time via [`include_str!`]. The binary owns only the *policy*
//! (resolve latest, compare, refuse cargo installs); the script owns the
//! *mechanism* (download, verify, extract, atomic swap, re-point symlink).
//!
//! # Provenance: refuse `cargo install` builds
//!
//! A `cargo install` build is a lone binary in `~/.cargo/bin` with no `lib/`
//! sibling and no archive provenance — there is nothing safe to swap. Such a
//! binary is detected (no `lib/libonnxruntime.*` beside it) and `update`
//! refuses with an actionable hint rather than attempting a swap it can't do.
//!
//! # Scope
//!
//! Unix-first (macOS arm64, Linux x86_64), matching the self-update surface. A
//! running `.exe` is locked on Windows, so Windows self-update is a follow-up —
//! but Windows *is* a shipped release target, so `update` there must still give
//! an honest answer. A Windows script install is detected on its own terms (an
//! `onnxruntime.dll` beside the binary) and refused with the re-run-the-installer
//! hint, never the `cargo install` hint that would be false for a script install.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::commands::version::VERSION;
use crate::error::{CliError, CliResult, Result, TemperError};
use crate::format::OutputFormat;

/// The canonical installer, embedded at build time. `temper update` pipes
/// *this exact script* to `sh`, so its download → verify → atomic-swap →
/// symlink logic can never fork from what a fresh `curl … | sh` install runs.
const INSTALL_SH: &str = include_str!("../../../../scripts/install/install.sh");

/// GitHub `releases/latest` endpoint — the same source `install.sh` reads to
/// resolve "latest".
const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/tasker-systems/temper/releases/latest";

/// Shown when the running binary is a `cargo install` build we can't swap.
const CARGO_REFUSAL: &str = "`temper update` manages curl-script installs only. This binary looks \
like a `cargo install` build (no bundled lib/libonnxruntime.* sibling). Update it with:\n  \
cargo install --path crates/temper-cli --locked --features embed,extract\n\
(or `cargo install temper-cli` once published).";

/// Shown on Windows when a genuine `install.ps1` install is detected. Windows is
/// a shipped release target, but self-update there is deferred — a running
/// `.exe` is file-locked, so the install dir can't be swapped out from under it
/// (`install.ps1` does `Remove-Item -Recurse` then re-extract, which can't
/// replace the running image). Points at the one action that *does* update: re-
/// running the PowerShell installer. Deliberately NOT the `cargo install` hint,
/// which is false advice for a script install.
#[cfg(windows)]
const WINDOWS_REFUSAL: &str = "`temper update` does not yet support self-update on Windows \
(a running .exe is file-locked). Re-run the installer to update:\n  \
irm https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.ps1 | iex";

/// GitHub `releases/latest` response — only the field we consume. A typed
/// struct over `serde_json::Value` per the repo's "typed structs at
/// boundaries" rule.
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
}

/// Where the running binary lives and whether it's a curl-script install we
/// can safely replace wholesale.
enum InstallLayout {
    /// A curl-script install: the binary sits in an install dir with a
    /// `lib/libonnxruntime.*` sibling. `dir` is that install dir (the atomic
    /// swap target).
    CurlScript { dir: PathBuf },
    /// A `cargo install` build (or anything else without the bundled lib):
    /// refuse rather than best-effort.
    Cargo,
    /// A Windows `install.ps1` install: `temper.exe` sits beside an
    /// `onnxruntime.dll` at the install-dir *root* (no `lib/` subdir, unlike
    /// unix). Self-update is deferred on Windows, so this refuses with the
    /// re-run-the-installer hint rather than being misfiled as [`Self::Cargo`]
    /// and handed the (false) `cargo install` advice.
    #[cfg(windows)]
    WindowsScript,
}

/// `--check`-mode report: current vs latest, no mutation.
#[derive(Debug, Serialize)]
struct UpdateCheckReport<'a> {
    current: &'a str,
    latest: &'a str,
    up_to_date: bool,
    install_dir: String,
}

/// Post-update report emitted after a successful install.
#[derive(Debug, Serialize)]
struct UpdateReport<'a> {
    previous_version: &'a str,
    target: &'a str,
    /// Version read back from the freshly-installed binary (best-effort — a
    /// read-back failure doesn't fail the update, since the swap already
    /// succeeded).
    installed_version: Option<String>,
    install_dir: String,
    forced: bool,
}

/// `temper update [--check] [--version vX.Y.Z] [--force]`.
///
/// Returns [`CliResult`]: install-specific failures surface as
/// [`CliError::Install`] (kept out of the shared `TemperError` so server crates
/// never carry install semantics); core failures — version resolution, render —
/// propagate through `?` as [`CliError::Temper`].
pub fn run(check: bool, version: Option<String>, force: bool, fmt: OutputFormat) -> CliResult<()> {
    // 1. Provenance first: a cargo build has no install dir to swap.
    let install_dir = match detect_install_layout()? {
        InstallLayout::CurlScript { dir } => dir,
        InstallLayout::Cargo => return Err(CliError::Install(CARGO_REFUSAL.to_string())),
        #[cfg(windows)]
        InstallLayout::WindowsScript => return Err(CliError::Install(WINDOWS_REFUSAL.to_string())),
    };

    // 2. Resolve the target tag. An explicit --version pin is a pass-through
    //    (the user asked for that exact release); otherwise resolve the latest
    //    published tag and normalize it for comparison.
    let pinned = version.map(|v| ensure_v_prefix(&v));
    let target_tag = match &pinned {
        Some(tag) => tag.clone(),
        None => resolve_latest_tag()?,
    };
    let target_version = normalize_version(&target_tag);

    // "No update needed" (unpinned path only): the latest release is not
    // strictly newer than what's running — i.e. we're current *or ahead*. An
    // explicit --version pin always proceeds, so it doubles as the deliberate
    // downgrade/repair lever; the unpinned path must never silently downgrade a
    // newer running build back to an older "latest".
    let up_to_date = pinned.is_none() && !is_strictly_newer(target_version, VERSION);

    // 3. --check: report and exit, mutating nothing.
    if check {
        let report = UpdateCheckReport {
            current: VERSION,
            latest: target_version,
            up_to_date,
            install_dir: install_dir.display().to_string(),
        };
        crate::output::plain(crate::format::render(&report, fmt)?);
        return Ok(());
    }

    // 4. No-op when there's nothing newer and no --force.
    if up_to_date && !force {
        crate::output::success(format!(
            "already up to date (running v{VERSION}; latest release v{target_version})"
        ));
        return Ok(());
    }

    // 5. Hand off to the embedded installer for download → checksum-verify →
    //    run-verify → atomic swap. The installer refuses to finalize unless the
    //    new binary actually runs, so a failure here leaves the prior install
    //    in place (it prints the exact recovery state to stderr).
    run_installer(&install_dir, &target_tag)?;

    // 6. Confirm the new version landed by running the installed binary. The
    //    installer already gated on runnability, so this is a belt-and-braces
    //    confirmation — but a mismatch or an unrunnable read-back is surfaced
    //    loudly rather than swallowed.
    let installed_version = read_installed_version(&install_dir);
    match &installed_version {
        Some(v) if v.contains(target_version) => {}
        Some(v) => crate::output::warning(format!(
            "installed binary reports \"{v}\", but {target_tag} was requested — \
             the updated binary may not be the one first on your PATH. \
             Check `which temper` and `temper --version`."
        )),
        None => crate::output::warning(
            "could not confirm the installed version by running the new binary; \
             run `temper --version` to verify.",
        ),
    }

    let report = UpdateReport {
        previous_version: VERSION,
        target: &target_tag,
        installed_version,
        install_dir: install_dir.display().to_string(),
        forced: force,
    };
    crate::output::plain(crate::format::render(&report, fmt)?);
    Ok(())
}

/// Classify the running binary's install layout. Resolves `current_exe`,
/// follows the on-PATH symlink to the real binary, and checks for the
/// `lib/libonnxruntime.*` sibling that only a curl-script install ships.
fn detect_install_layout() -> Result<InstallLayout> {
    let exe = std::env::current_exe()
        .map_err(|e| TemperError::Config(format!("cannot resolve current executable: {e}")))?;
    // The on-PATH `temper` is a symlink into the install dir; canonicalize to
    // the real binary so `.parent()` is the install dir, not `~/.local/bin`.
    let real = std::fs::canonicalize(&exe).unwrap_or(exe);
    let dir = real
        .parent()
        .ok_or_else(|| TemperError::Config("running binary has no parent directory".into()))?;
    // Windows script installs lay `onnxruntime.dll` beside `temper.exe` at the
    // install-dir root — there is no `lib/` subdir, so the unix discriminator
    // below misses them and would misfile a legitimate `install.ps1` install as
    // a `cargo install` build. Catch that layout first, on Windows only.
    #[cfg(windows)]
    if has_windows_ort_sibling(dir) {
        return Ok(InstallLayout::WindowsScript);
    }
    if has_ort_lib_sibling(dir) {
        Ok(InstallLayout::CurlScript {
            dir: dir.to_path_buf(),
        })
    } else {
        Ok(InstallLayout::Cargo)
    }
}

/// True when `dir` contains a `lib/libonnxruntime.*` file — the bundled ONNX
/// Runtime native lib every unix release archive ships beside the binary (see
/// `build-cli-binaries.yml`). Its presence is the signal that we own the whole
/// install dir; a `cargo install` build has no such sibling.
fn has_ort_lib_sibling(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir.join("lib")) else {
        return false;
    };
    entries.flatten().any(|e| {
        e.file_name()
            .to_string_lossy()
            .starts_with("libonnxruntime")
    })
}

/// True when `dir` contains an `onnxruntime.dll` at its root — the ONNX Runtime
/// native lib the *Windows* release archive lays beside `temper.exe` (at the
/// archive root, not in a `lib/` subdir; see `build-cli-binaries.yml`'s
/// `lib_dest_dir: .` for the windows target). This is the Windows analogue of
/// [`has_ort_lib_sibling`]: its presence marks a genuine `install.ps1` install,
/// distinguishing it from a bare `cargo install` build (which `cargo install`
/// copies without any sibling DLL). Compiled on all platforms so its behavior is
/// unit-tested everywhere, but only *consulted* under `#[cfg(windows)]`.
#[cfg_attr(not(windows), allow(dead_code))]
fn has_windows_ort_sibling(dir: &Path) -> bool {
    dir.join("onnxruntime.dll").is_file()
}

/// Resolve the latest published release tag from the GitHub API — the same
/// call `install.sh` makes when no `--version` is given. Runs a short-lived
/// tokio runtime for the one async request.
fn resolve_latest_tag() -> Result<String> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            // GitHub's API rejects requests without a User-Agent.
            .user_agent(format!("temper-cli/{VERSION}"))
            // Bounded timeouts so a black-hole network fails fast instead of
            // hanging forever (reqwest has no default timeout). Nothing has been
            // touched at this point, so a timeout is a clean, non-destructive error.
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| TemperError::Network(format!("building HTTP client: {e}")))?;
        let resp = client
            .get(LATEST_RELEASE_URL)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| TemperError::Network(format!("querying GitHub releases: {e}")))?;
        // A 403 is almost always the unauthenticated rate limit (shared NAT/CI
        // IPs hit it) — flag it as transient and reassure nothing was changed,
        // rather than leaving a bare "403 Forbidden" that reads like an auth wall.
        if resp.status() == reqwest::StatusCode::FORBIDDEN {
            return Err(TemperError::Network(
                "GitHub API rate-limited or forbidden (HTTP 403). Your install is \
                 unchanged — retry in a few minutes."
                    .to_string(),
            ));
        }
        if !resp.status().is_success() {
            return Err(TemperError::Api(format!(
                "GitHub releases API returned {}",
                resp.status()
            )));
        }
        let release: GithubRelease = resp
            .json()
            .await
            .map_err(|e| TemperError::Api(format!("parsing GitHub release JSON: {e}")))?;
        Ok(release.tag_name)
    })
}

/// Pipe the embedded `install.sh` to `sh -s -- --version <tag>`, aiming it at
/// the detected `install_dir` (via `TEMPER_INSTALL_DIR`) so the swap targets
/// exactly where the running binary lives. The installer's own progress
/// chatter is redirected to stderr so this command's stdout carries only the
/// final machine-readable report.
fn run_installer(install_dir: &Path, tag: &str) -> CliResult<()> {
    crate::output::progress(format!("Updating to {tag}…\n"));

    let mut child = Command::new("sh")
        .arg("-s")
        .arg("--")
        .arg("--version")
        .arg(tag)
        // Detection is authoritative: install into the dir the running binary
        // actually lives in, not whatever the XDG default recomputes to.
        .env("TEMPER_INSTALL_DIR", install_dir)
        .stdin(Stdio::piped())
        .stdout(installer_stdout())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| CliError::Install(format!("spawning installer shell: {e}")))?;

    child
        .stdin
        .take()
        .ok_or_else(|| CliError::Install("installer stdin unavailable".into()))?
        .write_all(INSTALL_SH.as_bytes())
        .map_err(|e| CliError::Install(format!("writing installer script: {e}")))?;

    let status = child
        .wait()
        .map_err(|e| CliError::Install(format!("waiting on installer: {e}")))?;

    if !status.success() {
        // The installer prints the true post-failure state to stderr (untouched,
        // restored, or — worst case — where the backup survives), so relay
        // rather than assert an intactness we can't verify from here.
        return Err(CliError::Install(format!(
            "installer exited with {status}; see the installer output above for \
             the state of your install"
        )));
    }
    Ok(())
}

/// The installer's stdout target: a dup of *our* stderr on unix, so its
/// human-facing progress lines don't pollute this command's machine-readable
/// stdout. On non-unix, inherit (Windows self-update is a follow-up anyway).
#[cfg(unix)]
fn installer_stdout() -> Stdio {
    use std::os::fd::FromRawFd;
    // SAFETY: `dup` returns a fresh owned fd or -1; on -1 we fall back to
    // inherit rather than construct from an invalid fd. `Stdio` takes
    // ownership and closes the fd on drop.
    let fd = unsafe { libc::dup(libc::STDERR_FILENO) };
    if fd < 0 {
        return Stdio::inherit();
    }
    unsafe { Stdio::from_raw_fd(fd) }
}

#[cfg(not(unix))]
fn installer_stdout() -> Stdio {
    Stdio::inherit()
}

/// Best-effort read-back: run the freshly-installed binary to confirm the new
/// version landed. Returns `None` on any failure — the swap already succeeded,
/// so this is confirmation, not a gate.
fn read_installed_version(install_dir: &Path) -> Option<String> {
    let out = Command::new(install_dir.join("temper"))
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Strip a single leading `v` so a `v0.3.0` git tag compares cleanly against
/// the bare `CARGO_PKG_VERSION` (`0.3.0`).
fn normalize_version(s: &str) -> &str {
    s.strip_prefix('v').unwrap_or(s)
}

/// Parse a `MAJOR.MINOR.PATCH` core into a comparable tuple, ignoring any
/// `-prerelease` / `+build` suffix. Returns `None` when the core isn't exactly
/// three numeric components, so callers can fall back to a safe default rather
/// than mis-order an exotic version string.
fn parse_core(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.split(['-', '+']).next().unwrap_or(v);
    let mut it = core.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None; // more than three components — not a plain core
    }
    Some((major, minor, patch))
}

/// Is `candidate` a strictly newer release than `base`? Compares the numeric
/// `MAJOR.MINOR.PATCH` cores. If *either* side can't be parsed as a plain core
/// (e.g. a prerelease-only or non-semver tag), fall back to string inequality —
/// so an odd version never wedges `update` into refusing to act, while the
/// common case still refuses to silently downgrade a newer running build.
fn is_strictly_newer(candidate: &str, base: &str) -> bool {
    match (parse_core(candidate), parse_core(base)) {
        (Some(c), Some(b)) => c > b,
        _ => candidate != base,
    }
}

/// Ensure a user-supplied `--version` carries the leading `v` the release tags
/// (and `install.sh`'s archive naming) use, so `--version 0.3.0` and
/// `--version v0.3.0` both resolve the same archive.
fn ensure_v_prefix(v: &str) -> String {
    if v.starts_with('v') {
        v.to_string()
    } else {
        format!("v{v}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `normalize_version` strips exactly one leading `v` and leaves an
    /// already-bare version untouched, so tag-vs-`CARGO_PKG_VERSION` equality
    /// compares like-for-like.
    #[test]
    fn normalize_version_strips_leading_v() {
        assert_eq!(normalize_version("v0.3.0"), "0.3.0");
        assert_eq!(normalize_version("0.3.0"), "0.3.0");
        // Only the first char, and only if it's `v`.
        assert_eq!(normalize_version("version"), "ersion");
    }

    /// `ensure_v_prefix` is idempotent and forgiving of a `--version` value
    /// given with or without the leading `v`.
    #[test]
    fn ensure_v_prefix_adds_v_once() {
        assert_eq!(ensure_v_prefix("0.3.0"), "v0.3.0");
        assert_eq!(ensure_v_prefix("v0.3.0"), "v0.3.0");
    }

    /// The comparison the unpinned no-op branch relies on: the same release is
    /// not "strictly newer" than itself, so an up-to-date install is a clean
    /// no-op; a higher latest is newer.
    #[test]
    fn up_to_date_comparison_matches_compiled_version() {
        let same = normalize_version(&format!("v{VERSION}")).to_string();
        assert!(
            !is_strictly_newer(&same, VERSION),
            "same version isn't newer"
        );
        assert!(
            is_strictly_newer("99.99.99", VERSION),
            "a higher latest is newer"
        );
    }

    /// `parse_core` accepts a plain `X.Y.Z`, drops a prerelease/build suffix,
    /// and rejects non-cores (too few/many components, non-numeric).
    #[test]
    fn parse_core_handles_plain_and_prerelease() {
        assert_eq!(parse_core("0.3.0"), Some((0, 3, 0)));
        assert_eq!(parse_core("1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(parse_core("1.2.3+build.7"), Some((1, 2, 3)));
        assert_eq!(parse_core("0.3"), None);
        assert_eq!(parse_core("0.3.0.1"), None);
        assert_eq!(parse_core("nightly"), None);
    }

    /// The anti-downgrade guard: a newer running build is NOT "up to date"-
    /// eligible for a downgrade — a lower latest is not strictly newer, so the
    /// unpinned path no-ops instead of rolling the user back.
    #[test]
    fn strictly_newer_refuses_silent_downgrade() {
        assert!(is_strictly_newer("0.4.0", "0.3.0"));
        // would-be downgrade → not newer
        assert!(!is_strictly_newer("0.3.0", "0.4.0"));
        // equal → not newer
        assert!(!is_strictly_newer("0.3.0", "0.3.0"));
        // A newer running prerelease vs an older stable latest: the core compare
        // says the stable isn't newer, so no downgrade.
        assert!(!is_strictly_newer("0.3.0", "0.4.0-rc1"));
        // Unparseable on either side → string-inequality fallback (still acts
        // when the tags genuinely differ, never wedges).
        assert!(is_strictly_newer("nightly", "0.3.0"));
        assert!(!is_strictly_newer("nightly", "nightly"));
    }

    /// `has_ort_lib_sibling` is the curl-vs-cargo discriminator. A dir with
    /// `lib/libonnxruntime.*` reads as a curl install; a bare dir (the shape
    /// of a `cargo install` bin dir) does not.
    #[test]
    fn ort_lib_sibling_detects_curl_layout() {
        let tmp = tempfile::tempdir().unwrap();
        // Bare dir: no lib/ → not a curl install.
        assert!(!has_ort_lib_sibling(tmp.path()));

        // With lib/libonnxruntime.so → curl install.
        let lib = tmp.path().join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("libonnxruntime.so"), b"").unwrap();
        assert!(has_ort_lib_sibling(tmp.path()));
    }

    /// An empty `lib/` (no `libonnxruntime.*`) does not count as a curl
    /// install — the discriminator is the ORT lib specifically, not any `lib`
    /// directory.
    #[test]
    fn ort_lib_sibling_requires_the_ort_lib() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("lib")).unwrap();
        assert!(!has_ort_lib_sibling(tmp.path()));
    }

    /// `has_windows_ort_sibling` is the Windows curl-vs-cargo discriminator,
    /// mirroring `ort_lib_sibling_detects_curl_layout`. A dir with
    /// `onnxruntime.dll` at its root reads as an `install.ps1` install; a bare
    /// dir (the shape of a `cargo install` bin dir) does not.
    #[test]
    fn windows_ort_sibling_detects_script_layout() {
        let tmp = tempfile::tempdir().unwrap();
        // Bare dir: no onnxruntime.dll → not a Windows script install.
        assert!(!has_windows_ort_sibling(tmp.path()));

        // With onnxruntime.dll beside the binary → Windows script install.
        std::fs::write(tmp.path().join("onnxruntime.dll"), b"").unwrap();
        assert!(has_windows_ort_sibling(tmp.path()));
    }

    /// The two discriminators do not cross-fire: a Windows script layout (dll at
    /// root, no `lib/`) is never read as a unix curl install, and a unix curl
    /// layout (`lib/libonnxruntime.*`, no root dll) is never read as a Windows
    /// script install. This is what keeps a real Windows install from being
    /// misfiled as `Cargo` and handed the false `cargo install` hint.
    #[test]
    fn windows_and_unix_discriminators_are_disjoint() {
        // Windows script layout: onnxruntime.dll at root, no lib/.
        let win = tempfile::tempdir().unwrap();
        std::fs::write(win.path().join("onnxruntime.dll"), b"").unwrap();
        assert!(has_windows_ort_sibling(win.path()));
        assert!(
            !has_ort_lib_sibling(win.path()),
            "windows script layout must not read as a unix curl install"
        );

        // Unix curl layout: lib/libonnxruntime.*, no root dll.
        let unix = tempfile::tempdir().unwrap();
        let lib = unix.path().join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("libonnxruntime.so"), b"").unwrap();
        assert!(has_ort_lib_sibling(unix.path()));
        assert!(
            !has_windows_ort_sibling(unix.path()),
            "unix curl layout must not read as a windows script install"
        );
    }

    /// The Windows refusal points at the installer, not at `cargo install` —
    /// the whole point of the fix is that a script install stops being told to
    /// rebuild with cargo. Windows-only, since the constant is `#[cfg(windows)]`.
    #[cfg(windows)]
    #[test]
    fn windows_refusal_is_actionable_and_not_the_cargo_hint() {
        assert!(WINDOWS_REFUSAL.contains("install.ps1"));
        assert!(!WINDOWS_REFUSAL.contains("cargo install"));
    }

    /// The cargo-refusal hint names the exact recovery command an operator
    /// needs, so the error is actionable rather than a dead end.
    #[test]
    fn cargo_refusal_is_actionable() {
        assert!(CARGO_REFUSAL.contains("cargo install"));
        assert!(CARGO_REFUSAL.contains("--features embed,extract"));
    }

    /// The embedded installer is the real script, not a stub — guard against an
    /// `include_str!` path drift silently shipping an empty/foreign file.
    #[test]
    fn embedded_installer_is_the_real_script() {
        assert!(INSTALL_SH.contains("REPO=\"tasker-systems/temper\""));
        assert!(INSTALL_SH.contains("Verifying checksum"));
    }
}
