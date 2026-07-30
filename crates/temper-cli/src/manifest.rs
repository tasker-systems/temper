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
/// and consumed here.
///
/// Two things hold the bash producer and this consumer to one wire format, and
/// they are not equivalent. `manifest-golden.json` is a checked-in *snapshot* —
/// it proves this type still parses the bytes the producer emitted on the day
/// the fixture was regenerated, and nothing more. The live proof is
/// `producer_bytes_deserialize_and_verify_through_this_module`, which runs
/// `.github/scripts/release/emit-manifest.sh` and feeds its actual output to
/// this deserializer: a coordinated field rename on both bash sides is caught
/// there, and only there.
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

    /// The golden fixture is a *snapshot* of the wire contract shared with the
    /// workflow bash that produces it. It was hand-authored once and never
    /// regenerated, which is precisely how a fixture stops describing the
    /// producer; it is now generated by running the real producer:
    ///
    /// ```text
    /// T=$(mktemp -d)
    /// mkdir -p "$T/staging/lib" "$T/staging/models"
    /// printf 'binary' > "$T/staging/temper"
    /// printf 'ort'    > "$T/staging/lib/libonnxruntime.so"
    /// printf 'model'  > "$T/staging/models/model_quantized.onnx"
    /// printf 'lic'    > "$T/staging/LICENSE"
    /// LC_ALL=C VERSION=0.3.0 TARGET=x86_64-unknown-linux-gnu \
    ///   STAGING="$T/staging" OUTPUT=crates/temper-cli/tests/fixtures/manifest-golden.json \
    ///   bash .github/scripts/release/emit-manifest.sh
    /// rm -rf "$T"
    /// ```
    ///
    /// `LC_ALL=C` is not decoration: `emit-manifest.sh:44` orders entries with
    /// `sort -z`, whose collation is locale-dependent — the same staging tree
    /// yields `lib/…` before `LICENSE` under a macOS default locale and the
    /// reverse under `C`. Without it the fixture is not reproducible across
    /// the machines that might regenerate it. The staging shape deliberately
    /// matches `.github/scripts/test-emit-manifest.sh:13-18`, so the fixture
    /// and the producer's own harness describe one archive rather than two.
    ///
    /// A snapshot can only ever prove what the producer emitted *then*. The
    /// drift guard that runs against the producer as it stands today is
    /// `producer_bytes_deserialize_and_verify_through_this_module` below.
    #[test]
    fn golden_fixture_round_trips() {
        let raw = include_str!("../tests/fixtures/manifest-golden.json");
        let m: ReleaseManifest = serde_json::from_str(raw).expect("golden fixture parses");
        assert_eq!(m.target, "x86_64-unknown-linux-gnu");
        assert!(m.files.iter().any(|f| f.path == "temper"));
        assert!(m.files.iter().all(|f| f.sha256.len() == 64));
    }

    /// The repo root, reached from this crate's manifest dir. The release bash
    /// lives under `.github/scripts/`, which belongs to no crate, so there is
    /// no `include_str!`-shaped way to get at it.
    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .canonicalize()
            .expect("repo root resolves from CARGO_MANIFEST_DIR")
    }

    /// Run the REAL `.github/scripts/release/emit-manifest.sh` over a staging
    /// dir shaped like a unix release archive, and hand back the staging dir
    /// plus the exact bytes it wrote.
    ///
    /// `OUTPUT` is placed OUTSIDE `STAGING` on purpose: the producer recounts
    /// `find "$STAGING" -type f` after writing (`emit-manifest.sh:64-69`) as a
    /// second opinion on the entry count, so a manifest written into its own
    /// staging tree would be counted as a file it does not list and the
    /// producer would refuse it.
    ///
    /// The `TempDir` is returned rather than dropped here because dropping it
    /// deletes the tree the caller is about to verify.
    ///
    /// `jq` is required — `emit-manifest.sh:41` builds every entry with it — so
    /// a machine without it cannot run the producer at all. That is a hard
    /// failure and deliberately not a skip: these two tests exist because a
    /// check that cannot go red is worth nothing, and a test that quietly opts
    /// out on some machines is the same vacuity one level up. CI has it: the
    /// `guard-tests` job already runs `test-emit-manifest.sh` and
    /// `scripts/install/test-install.sh` on `ubuntu-latest` with no install
    /// step for `jq`, and the unit job this test runs in is the same image.
    fn run_real_producer() -> (tempfile::TempDir, std::path::PathBuf, String) {
        let work = tempfile::tempdir().unwrap();
        let staging = work.path().join("staging");
        write(&staging, "temper", b"binary");
        write(&staging, "lib/libonnxruntime.so", b"ort");
        write(&staging, "models/model_quantized.onnx", b"model");
        write(&staging, "LICENSE", b"lic");

        let output = work.path().join("out.json");
        let script = repo_root().join(".github/scripts/release/emit-manifest.sh");
        let run = std::process::Command::new("bash")
            .arg(&script)
            .env("VERSION", "0.3.0")
            .env("TARGET", "x86_64-unknown-linux-gnu")
            .env("STAGING", &staging)
            .env("OUTPUT", &output)
            .output()
            .unwrap_or_else(|e| panic!("could not run {}: {e}", script.display()));
        assert!(
            run.status.success(),
            "emit-manifest.sh failed (is `jq` installed?): {}",
            String::from_utf8_lossy(&run.stderr)
        );

        let raw = std::fs::read_to_string(&output).expect("the producer wrote a manifest");
        (work, staging, raw)
    }

    /// The cross-language round trip: REAL producer bytes into the REAL
    /// deserializer.
    ///
    /// Until this existed the wire format was proven twice and never against
    /// itself — `golden_fixture_round_trips` parses a hand-authored snapshot,
    /// and `test-emit-manifest.sh` checks the producer against its own `jq`
    /// queries. Neither can see a rename applied consistently across both bash
    /// sides, so `emit-manifest.sh` could start emitting `hash` while every
    /// check stayed green and this deserializer silently stopped parsing. The
    /// first symptom would be a user's install failing.
    ///
    /// Parsing is only half of it. The `verify_dir` call is the stronger half:
    /// it re-hashes the producer's own staging tree with `sha2` and demands
    /// `Verified`, so a producer that emitted sha512, a constant, the wrong
    /// file's digest, or an absolute path fails here even though all of those
    /// deserialize perfectly.
    #[test]
    fn producer_bytes_deserialize_and_verify_through_this_module() {
        let (_work, staging, raw) = run_real_producer();

        let m: ReleaseManifest = serde_json::from_str(&raw).unwrap_or_else(|e| {
            panic!("real producer bytes must parse as ReleaseManifest: {e}\n{raw}")
        });

        assert_eq!(m.version, "0.3.0", "{raw}");
        assert_eq!(m.target, "x86_64-unknown-linux-gnu", "{raw}");
        assert_eq!(m.files.len(), 4, "one entry per staged file: {raw}");

        match verify_dir(&m, &staging) {
            Verdict::Verified => {}
            other => panic!("the producer's own staging tree must verify clean, got {other:?}"),
        }
    }

    /// The bite for the round trip above. A guard that cannot go red proves
    /// nothing, so this renames the producer's hash field on the way to the
    /// deserializer — exactly the drift the pair exists to catch — and asserts
    /// the parse fails.
    ///
    /// The rename is applied to the producer's ACTUAL output rather than to a
    /// literal, so the test also witnesses that `sha256` is the key the
    /// producer emits today; if it ever stopped, the pre-assertion below goes
    /// red rather than the rename silently becoming a no-op.
    #[test]
    fn a_renamed_producer_field_breaks_the_round_trip() {
        let (_work, _staging, raw) = run_real_producer();

        assert!(
            raw.contains("\"sha256\""),
            "the producer must emit a `sha256` key for this bite to mean anything:\n{raw}"
        );
        let renamed = raw.replace("\"sha256\"", "\"hash\"");
        assert_ne!(renamed, raw, "the rename must actually change the bytes");

        let err = serde_json::from_str::<ReleaseManifest>(&renamed)
            .expect_err("a manifest whose hash field was renamed must not deserialize");
        assert!(
            err.to_string().contains("sha256"),
            "the error must name the field that went missing, got: {err}"
        );
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
