//! The Homebrew boundary (`tasker-systems/homebrew-tap`; spec of record:
//! `temper-artifacts:specs/2026-09-16-homebrew-tap-design.md`, D-H2 and the
//! 2026-09-16 verify re-rule).
//!
//! A brew-managed install carries the formula's `BREW-MANAGED` marker beside
//! the binary. Two commands honor it: `temper update` refuses (the binary is
//! not authoritative for its own update), and `temper version --verify
//! --online` reports the boundary instead of comparing brew-transformed bytes
//! against published ones — Homebrew finalizes Mach-O files at keg
//! finalization (dynamic-linkage fixups + ad-hoc re-signing with per-install
//! random identifiers) and relocates metadata, so published-manifest byte
//! comparison cannot apply. Offline `--verify` is unaffected: the formula
//! plants a manifest computed from the actual installed tree, so it proves
//! this-install self-consistency.

use std::path::Path;

/// The marker file a `tasker-systems/tap` formula plants beside the binary.
pub(crate) const MARKER_FILE: &str = "BREW-MANAGED";

/// True when the install dir carries the brew marker — the authority signal
/// that this install belongs to Homebrew.
pub(crate) fn is_managed(install_dir: &Path) -> bool {
    install_dir.join(MARKER_FILE).is_file()
}
