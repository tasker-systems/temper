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
//!
//! The trichotomy also has a floor: a manifest listing **zero** files asserts
//! nothing about an install, so it can never be `Verified` — [`verify_dir`]
//! returns [`Verdict::Unverifiable`] for it. Not `Mismatch`, because nothing
//! disagreed; there was simply nothing to check.

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

/// Whether a manifest entry's `path` names something strictly inside the
/// install dir. Only ordinary path components are allowed: `RootDir` means an
/// absolute path (which [`Path::join`] honors by *discarding the base*),
/// `ParentDir` escapes upward, `Prefix` is a Windows drive or UNC root, and
/// `CurDir` is noise no producer emits. `emit-manifest.sh:38` only ever writes
/// paths relative to `STAGING` (`REL="${f#"$STAGING"/}"`), so nothing
/// legitimate is excluded.
///
/// This is a check on *components*, not on substrings: `foo..bar` is an
/// ordinary filename that merely contains dots and is accepted.
fn is_contained_relative(rel: &str) -> bool {
    // `Path::new("").components()` yields nothing, and `all` over an empty
    // iterator is `true` — so the emptiness check cannot be folded away.
    !rel.is_empty()
        && Path::new(rel)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// Hash every file the manifest lists and compare. Files present in `dir` but
/// absent from the manifest are ignored — the manifest states what we shipped,
/// not what the user may not add beside it.
///
/// A manifest listing no files is [`Verdict::Unverifiable`], never
/// [`Verdict::Verified`]: it asserts nothing, so there is nothing it could
/// have verified.
///
/// Every entry must also name a path *contained* by `dir` — no absolute path,
/// no `..` component, not empty; see `is_contained_relative`. An entry that
/// escapes is a [`Verdict::Mismatch`] with no `actual` hash, reported on its
/// path alone and never read.
pub fn verify_dir(manifest: &ReleaseManifest, dir: &Path) -> Verdict {
    // A manifest with no entries asserts nothing about this install, so the
    // loop below cannot fail and the function would return `Verified` over
    // zero checked files. That is a fail-open, and no attacker is needed to
    // reach it: a build-side staging drift emits `files: []`, CI faithfully
    // attests it, and `--verify --online` then returns a genuine,
    // signature-backed `verified` covering nothing. Not a `Mismatch` — nothing
    // disagreed; we simply cannot tell.
    if manifest.files.is_empty() {
        return Verdict::Unverifiable {
            reason: "manifest lists no files — it asserts nothing about this install".to_string(),
        };
    }

    let mut mismatches = Vec::new();
    for entry in &manifest.files {
        // Refuse the entry on its path alone, before anything is read. An
        // absolute path makes `dir.join` discard `dir` entirely and an
        // ancestor-relative one walks out of it, so an attacker who can edit
        // only `path` strings — never a hash — can point an entry at a real
        // file whose real hash the manifest already states. That entry then
        // verifies clean while the install's own `temper` is never hashed at
        // all, and the whole dir reports `Verified`.
        //
        // Reported through the existing `Mismatch` channel with `actual:
        // None`, the same shape an absent file already uses, so no caller's
        // rendering changes. It is a `Mismatch` and not `Unverifiable`
        // because this is not "we cannot tell": the manifest is making a claim
        // it is not permitted to make, which is a definite disagreement with
        // the shape a published manifest is allowed to have.
        if !is_contained_relative(&entry.path) {
            mismatches.push(Mismatch {
                path: entry.path.clone(),
                expected: entry.sha256.clone(),
                actual: None,
            });
            continue;
        }
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

    /// A manifest listing zero files asserts nothing, so it cannot verify
    /// anything. Returning `Verified` here was a reproduced fail-open: an
    /// archive containing `# EVIL PAYLOAD` installed with exit 0. It is not a
    /// `Mismatch` either — nothing disagreed; there was simply nothing to check.
    #[test]
    fn empty_manifest_is_unverifiable_not_verified() {
        let tmp = tempfile::tempdir().unwrap();
        // A real file sits in the dir, so the verdict cannot be blamed on an
        // empty directory: it is the manifest that asserts nothing.
        write(tmp.path(), "temper", b"binary-bytes");

        let m = ReleaseManifest {
            version: "0.3.0".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            files: vec![],
        };
        match verify_dir(&m, tmp.path()) {
            Verdict::Unverifiable { reason } => assert!(
                reason.contains("no files"),
                "reason should name the vacuity, got: {reason}"
            ),
            other => panic!("empty manifest must be unverifiable, got {other:?}"),
        }
    }

    /// The bite for path containment. Each entry below names a REAL file whose
    /// REAL hash the manifest states, so without a containment check the entry
    /// verifies clean and the whole dir reports `Verified` — while the
    /// install's own `temper` is never hashed at all. Forging the offline
    /// verdict then costs only an edit to a `path` string, never to a hash.
    ///
    /// Deliberately NOT written against non-existent targets (`../escape`):
    /// those already fail as "missing file" today, so a test built on them
    /// would pass before the fix and prove nothing.
    #[test]
    fn an_entry_pointing_outside_the_dir_is_a_mismatch_not_a_verification() {
        let root = tempfile::tempdir().unwrap();
        let install = root.path().join("install");
        write(&install, "temper", b"real-binary-bytes");
        write(root.path(), "outside/decoy", b"decoy-bytes");
        let decoy_sha = sha_of(b"decoy-bytes");

        // Absolute: `Path::join` discards `install` outright.
        // Ancestor-relative: it walks out of `install` a level at a time.
        let absolute = root
            .path()
            .join("outside/decoy")
            .to_string_lossy()
            .into_owned();
        for bad in [absolute.as_str(), "../outside/decoy"] {
            let m = ReleaseManifest {
                version: "0.3.0".into(),
                target: "x86_64-unknown-linux-gnu".into(),
                files: vec![ManifestEntry {
                    path: bad.to_string(),
                    sha256: decoy_sha.clone(),
                    size: 11,
                }],
            };
            match verify_dir(&m, &install) {
                Verdict::Mismatch { mismatches: ms } => {
                    assert_eq!(ms.len(), 1, "path {bad:?}");
                    assert_eq!(ms[0].path, bad, "the offending path is named");
                    assert!(
                        ms[0].actual.is_none(),
                        "path {bad:?} is refused unread, so there is no actual hash"
                    );
                }
                other => panic!("path {bad:?} must not verify, got {other:?}"),
            }
        }
    }

    /// `is_contained_relative` rejects every shape that can leave the install
    /// dir, plus the empty path — which `Path::components()` reports as *no*
    /// components, so an `all(...)` over it alone would vacuously succeed.
    #[test]
    fn contained_relative_rejects_escaping_and_empty_paths() {
        for bad in [
            "",
            "/etc/passwd",
            "/",
            "..",
            "../outside/decoy",
            "a/../../escape",
            "lib/../../../etc/passwd",
            // Rejected even though it would resolve back inside: containment is
            // decided on components without resolving, so there is no
            // resolution step for a symlink to disagree with afterwards.
            "foo/../bar",
        ] {
            assert!(!is_contained_relative(bad), "{bad:?} must not be contained");
        }
    }

    /// Dots inside a *filename* are ordinary. `foo..bar` is a legal file to
    /// ship, and a containment check written as a substring test would reject
    /// it — refusing a legitimate release rather than an escape. The check is
    /// on path COMPONENTS, so every name below is accepted.
    #[test]
    fn contained_relative_accepts_ordinary_paths_including_dotted_filenames() {
        for good in [
            "temper",
            "lib/libonnxruntime.so",
            "foo..bar",
            "..foo",
            "foo..",
            "a/foo..bar/b",
            "model/bge-base-en-v1.5/model.onnx",
        ] {
            assert!(is_contained_relative(good), "{good:?} must be contained");
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
