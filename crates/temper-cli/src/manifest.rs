//! The release manifest: per-file sha256 for every artifact shipped in a
//! release archive, and the verdict of checking an install directory against it.
//!
//! # Why per-file, and not just the archive
//!
//! The release's `.sha256` sidecar is computed over the whole archive
//! (`build-cli-binaries.yml:177,189`), so it measures a different object than
//! the installed binary and cannot answer "is my temper the one you shipped?".
//! This manifest closes that gap.
//!
//! # What a verdict does and does not mean
//!
//! Checking an install dir against a manifest *in that same dir* detects
//! corruption and drift — NOT an active attacker, who could replace both. Only
//! attestation-backed verification (`temper update`, `--verify --online`)
//! carries provenance weight. Callers must not render `Verified` as more than
//! it earned.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The manifest as written into an install directory by `install.sh`.
pub const MANIFEST_FILENAME: &str = ".temper-manifest.json";

/// Per-file hashes for one release archive. Produced by the release workflow
/// and consumed here; `manifest-golden.json` pins the wire shape so the bash
/// producer and this consumer cannot drift.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub version: String,
    pub target: String,
    pub files: Vec<ManifestEntry>,
}

/// One shipped file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

/// A file that failed verification. `actual` is `None` when the file is absent
/// entirely, which is a different failure from present-but-wrong.
#[derive(Debug, Clone, Serialize)]
pub struct Mismatch {
    pub path: String,
    pub expected: String,
    pub actual: Option<String>,
}

/// The outcome of checking an install dir. `Unverifiable` and `Mismatch` are
/// deliberately distinct: "we cannot tell" is not "it is wrong".
///
/// # Why struct variants, not newtype variants
///
/// Internal tagging (`tag = "verdict"`) can only serialize a variant whose body
/// is itself map-shaped. A newtype variant wrapping a `Vec` or a `String` —
/// `Mismatch(Vec<Mismatch>)` — compiles fine and then fails at **runtime** in
/// `TaggedSerializer::bad_type`, so the failure surfaces only when a caller
/// actually renders a non-`Verified` verdict: exactly the path that matters.
/// Struct variants keep the flat `{"verdict": "...", ...}` shape and serialize.
#[derive(Debug, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    Verified,
    Mismatch { mismatches: Vec<Mismatch> },
    Unverifiable { reason: String },
}

/// Load the manifest an install dir carries, if any. `None` means this is not
/// a manifest-bearing install (e.g. a `cargo install` build) — callers render
/// that as [`Verdict::Unverifiable`], never as a mismatch.
pub fn load_from_dir(dir: &Path) -> Option<ReleaseManifest> {
    let raw = std::fs::read_to_string(dir.join(MANIFEST_FILENAME)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Hash every file the manifest lists and compare. Files present in `dir` but
/// absent from the manifest are ignored — the manifest states what we shipped,
/// not what the user may not add beside it.
pub fn verify_dir(manifest: &ReleaseManifest, dir: &Path) -> Verdict {
    let mut mismatches = Vec::new();
    for entry in &manifest.files {
        match std::fs::read(dir.join(&entry.path)) {
            Ok(bytes) => {
                let actual = format!("{:x}", Sha256::digest(&bytes));
                if actual != entry.sha256 {
                    mismatches.push(Mismatch {
                        path: entry.path.clone(),
                        expected: entry.sha256.clone(),
                        actual: Some(actual),
                    });
                }
            }
            Err(_) => mismatches.push(Mismatch {
                path: entry.path.clone(),
                expected: entry.sha256.clone(),
                actual: None,
            }),
        }
    }
    if mismatches.is_empty() {
        Verdict::Verified
    } else {
        Verdict::Mismatch { mismatches }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &std::path::Path, rel: &str, bytes: &[u8]) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    fn sha_of(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(bytes))
    }

    /// A directory whose files match every manifest entry verifies clean.
    #[test]
    fn matching_dir_verifies() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "temper", b"binary-bytes");
        write(tmp.path(), "lib/libonnxruntime.so", b"ort-bytes");

        let m = ReleaseManifest {
            version: "0.3.0".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            files: vec![
                ManifestEntry {
                    path: "temper".into(),
                    sha256: sha_of(b"binary-bytes"),
                    size: 12,
                },
                ManifestEntry {
                    path: "lib/libonnxruntime.so".into(),
                    sha256: sha_of(b"ort-bytes"),
                    size: 9,
                },
            ],
        };
        assert!(matches!(verify_dir(&m, tmp.path()), Verdict::Verified));
    }

    /// A single altered byte is reported as a Mismatch naming that file — this
    /// is the bite: it must fail against the state it claims to detect.
    #[test]
    fn altered_file_is_a_mismatch_naming_the_path() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "temper", b"TAMPERED");

        let m = ReleaseManifest {
            version: "0.3.0".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            files: vec![ManifestEntry {
                path: "temper".into(),
                sha256: sha_of(b"binary-bytes"),
                size: 12,
            }],
        };
        match verify_dir(&m, tmp.path()) {
            Verdict::Mismatch { mismatches: ms } => {
                assert_eq!(ms.len(), 1);
                assert_eq!(ms[0].path, "temper");
                assert!(
                    ms[0].actual.is_some(),
                    "a present-but-wrong file has an actual hash"
                );
            }
            other => panic!("expected Mismatch, got {other:?}"),
        }
    }

    /// A file the manifest lists but the directory lacks is a Mismatch with no
    /// actual hash — distinct from a present-but-wrong file.
    #[test]
    fn missing_file_is_a_mismatch_with_no_actual() {
        let tmp = tempfile::tempdir().unwrap();
        let m = ReleaseManifest {
            version: "0.3.0".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            files: vec![ManifestEntry {
                path: "temper".into(),
                sha256: sha_of(b"binary-bytes"),
                size: 12,
            }],
        };
        match verify_dir(&m, tmp.path()) {
            Verdict::Mismatch { mismatches: ms } => assert!(ms[0].actual.is_none()),
            other => panic!("expected Mismatch, got {other:?}"),
        }
    }

    /// No manifest in the directory (the `cargo install` shape) yields None,
    /// which callers must render as Unverifiable — never Mismatch.
    #[test]
    fn absent_manifest_loads_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load_from_dir(tmp.path()).is_none());
    }

    /// The golden fixture is the wire contract shared with the workflow bash
    /// that produces it (Task 3). If this fails, producer and consumer drifted.
    #[test]
    fn golden_fixture_round_trips() {
        let raw = include_str!("../tests/fixtures/manifest-golden.json");
        let m: ReleaseManifest = serde_json::from_str(raw).expect("golden fixture parses");
        assert_eq!(m.target, "x86_64-unknown-linux-gnu");
        assert!(m.files.iter().any(|f| f.path == "temper"));
        assert!(m.files.iter().all(|f| f.sha256.len() == 64));
    }

    /// EVERY verdict variant must actually serialize.
    ///
    /// This is the test whose absence let a real defect through: the original
    /// shape used newtype variants (`Mismatch(Vec<Mismatch>)`) under internal
    /// tagging, which compiles and then fails at RUNTIME in serde's
    /// `TaggedSerializer::bad_type`. Tests that only pattern-match the verdict
    /// cannot see that — the failure lives on the render path. Both non-clean
    /// verdicts are exactly the ones a user sees when something is wrong, so a
    /// serialization failure there would break the surface precisely when it
    /// matters most.
    #[test]
    fn every_verdict_variant_serializes() {
        let verified = serde_json::to_string(&Verdict::Verified).expect("Verified serializes");
        assert!(verified.contains("\"verdict\":\"verified\""), "{verified}");

        let mismatch = serde_json::to_string(&Verdict::Mismatch {
            mismatches: vec![Mismatch {
                path: "temper".into(),
                expected: "aa".into(),
                actual: Some("bb".into()),
            }],
        })
        .expect("Mismatch serializes");
        assert!(mismatch.contains("\"verdict\":\"mismatch\""), "{mismatch}");
        assert!(mismatch.contains("\"path\":\"temper\""), "{mismatch}");

        let unverifiable = serde_json::to_string(&Verdict::Unverifiable {
            reason: "not a release install".into(),
        })
        .expect("Unverifiable serializes");
        assert!(
            unverifiable.contains("\"verdict\":\"unverifiable\""),
            "{unverifiable}"
        );
        assert!(
            unverifiable.contains("not a release install"),
            "{unverifiable}"
        );
    }

    /// The two failure verdicts are distinguishable in the rendered output, not
    /// just in the Rust type. "We cannot tell" must never render as "it is
    /// wrong" — that distinction is the whole point of the trichotomy, and it
    /// only reaches a caller through the serialized tag.
    #[test]
    fn unverifiable_never_renders_as_mismatch() {
        let json = serde_json::to_string(&Verdict::Unverifiable {
            reason: "no release manifest beside this binary".into(),
        })
        .expect("serializes");
        assert!(!json.contains("mismatch"), "{json}");
    }
}
