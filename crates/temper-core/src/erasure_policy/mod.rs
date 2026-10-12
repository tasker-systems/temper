//! The erasure-policy manifest: what an erasure reaches, as one typed model read by one parser.
//!
//! The manifest is several TOML files parsed as one unit (`policy/erasure/`): `manifest.toml` (the
//! root, carrying `format`), `estate.toml` (scope rules and one grain per table),
//! `personal-data.toml` and `sensitivity-scan.toml` (the column classes), and
//! `resource-erasure.toml` (what resource erasure does with every carrier column and ledger path).
//!
//! The parser lives here so that admin tooling and the CLI can express the shape. The text in force
//! is not compiled into this crate: the policy that governs an erasure is the deployed server's.
//!
//! Every column entry sits under a quoted `"table.column"` key, so a column declared twice fails the
//! TOML parse itself. Claims and dispositions keep their compact string forms (`"erased:7"`,
//! `"handled:D2.1"`, `"redact:title"`) and are read through `FromStr` into the enums below; an
//! unknown word is a parse error that names its file, line and column.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;
use serde::de::DeserializeOwned;

/// `Deserialize` for a type with a `FromStr` grammar: read a string, parse it, and report a grammar
/// failure as a deserialize error, so the TOML error carries the offending value's position.
macro_rules! deserialize_from_str {
    ($($ty:ty),+ $(,)?) => {$(
        impl<'de> ::serde::Deserialize<'de> for $ty {
            fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = <String as ::serde::Deserialize>::deserialize(deserializer)?;
                raw.parse().map_err(::serde::de::Error::custom)
            }
        }
    )+};
}

mod columns;
mod estate;
mod resource_erasure;

pub use columns::{
    Achieved, Ceiling, Claim, Class, ColumnKey, PersonalColumn, ScanColumn, ScanDisposition,
    SensitivityScan,
};
pub use estate::{Estate, Grain, Home, Link, Mechanism, ParentPath, Reach, ScopeRule, TableGrain};
pub use resource_erasure::{
    Disposition, PayloadDisposition, PayloadLine, Qualifier, Refinement, ResourceErasure, TableLine,
};

/// The manifest format this parser reads. `manifest.toml`'s `format` must equal it: a file written
/// for another format is refused rather than read under this one's meaning.
pub const FORMAT: u32 = 1;

/// The five manifest files' text, as one unit.
#[derive(Debug, Clone, Copy)]
pub struct ManifestFiles<'a> {
    /// `manifest.toml`: the root, carrying `format`.
    pub root: &'a str,
    /// `estate.toml`: scope rules and table grains.
    pub estate: &'a str,
    /// `personal-data.toml`: the personal-data column classes.
    pub personal_data: &'a str,
    /// `sensitivity-scan.toml`: the sensitivity-scan column dispositions.
    pub sensitivity_scan: &'a str,
    /// `resource-erasure.toml`: resource erasure's `[tables]` and `[payload]`.
    pub resource_erasure: &'a str,
}

/// Which manifest file a [`ParseError`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestFile {
    Root,
    Estate,
    PersonalData,
    SensitivityScan,
    ResourceErasure,
}

impl ManifestFile {
    /// The file's name under `policy/erasure/`.
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Root => "manifest.toml",
            Self::Estate => "estate.toml",
            Self::PersonalData => "personal-data.toml",
            Self::SensitivityScan => "sensitivity-scan.toml",
            Self::ResourceErasure => "resource-erasure.toml",
        }
    }
}

impl fmt::Display for ManifestFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.file_name())
    }
}

/// A manifest file failed to parse. `source` is the TOML error, which carries the line and column.
#[derive(Debug, thiserror::Error)]
#[error("erasure-policy manifest {file}: {source}")]
pub struct ParseError {
    /// The file that failed.
    pub file: ManifestFile,
    /// The TOML error, with its position in `file`.
    pub source: toml::de::Error,
}

/// A compact claim or disposition string that is not in its grammar or vocabulary.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct GrammarError(String);

/// The erasure policy: the parsed manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErasurePolicy {
    /// The manifest format (`manifest.toml`), always [`FORMAT`] once parsed.
    pub format: u32,
    /// Scope rules and one grain per table (`estate.toml`).
    pub estate: Estate,
    /// `table.column` → its personal-data class, ceiling and achieved reach (`personal-data.toml`).
    pub personal_data: BTreeMap<ColumnKey, PersonalColumn>,
    /// The sensitivity-scan columns and their stated count (`sensitivity-scan.toml`).
    pub sensitivity: SensitivityScan,
    /// What resource erasure does with each carrier column and ledger path (`resource-erasure.toml`).
    pub resource_erasure: ResourceErasure,
}

impl ErasurePolicy {
    /// The personal-data columns declared with `class`.
    pub fn personal_data_keys(&self, class: Class) -> impl Iterator<Item = &ColumnKey> {
        self.personal_data
            .iter()
            .filter(move |(_, column)| column.class == class)
            .map(|(key, _)| key)
    }
}

/// `manifest.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootFile {
    #[serde(deserialize_with = "known_format")]
    format: u32,
}

fn known_format<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let format = u32::deserialize(deserializer)?;
    if format == FORMAT {
        Ok(format)
    } else {
        Err(serde::de::Error::custom(format!(
            "unknown manifest format {format} (this parser reads format {FORMAT})"
        )))
    }
}

/// `personal-data.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersonalDataFile {
    columns: BTreeMap<ColumnKey, PersonalColumn>,
}

fn read<T: DeserializeOwned>(file: ManifestFile, text: &str) -> Result<T, ParseError> {
    toml::from_str(text).map_err(|source| ParseError { file, source })
}

/// Parse the five manifest files as one erasure policy.
pub fn parse(files: &ManifestFiles<'_>) -> Result<ErasurePolicy, ParseError> {
    let root: RootFile = read(ManifestFile::Root, files.root)?;
    let estate: Estate = read(ManifestFile::Estate, files.estate)?;
    let personal: PersonalDataFile = read(ManifestFile::PersonalData, files.personal_data)?;
    let sensitivity: SensitivityScan = read(ManifestFile::SensitivityScan, files.sensitivity_scan)?;
    let resource_erasure: ResourceErasure =
        read(ManifestFile::ResourceErasure, files.resource_erasure)?;
    Ok(ErasurePolicy {
        format: root.format,
        estate,
        personal_data: personal.columns,
        sensitivity,
        resource_erasure,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "format = 1\n";

    const ESTATE: &str = r#"
scope = [
  { rule = "the subject's @me contexts", mechanism = { bound = "_erasure_estate_contexts" } },
  { rule = "cogmaps whose only team join is the personal team", mechanism = { gap = "2a: R15 cogmap scope" } },
]

[tables.kb_resources]
grain = { kind = "per-item", container = false, fence = { bound = "is_active" }, unit = { bound = "resource_erasure_execute" }, home = { table = "kb_resource_homes", table_column = "anchor_table", id_column = "anchor_id" } }

[tables.kb_contexts]
grain = { kind = "per-item", container = true, fence = { bound = "is_active" }, unit = { gap = "2a: context unit" } }

[tables.kb_edges]
grain = { kind = "per-item", container = false, fence = { bound = "is_folded" }, unit = { gap = "2a: label redaction" } }
also_reached = [
  { path = { table = "kb_resources", link = { fk = "source_id" } }, mechanism = { bound = "resource_erasure_execute" } },
]

[tables.kb_properties]
grain = { kind = "covered", parent = { table = "kb_resources", link = { pair = { table_column = "owner_table", id_column = "owner_id" } } } }

[tables.kb_remote_sources]
grain = { kind = "covered", parent = { table = "kb_block_provenance", link = { pair = { table_column = "source_kind", id_column = "source_id" } }, via = "source_kind = 'remote_source'; no foreign key" } }
note = "re-pointed, not erased"

[tables.kb_profiles]
grain = { kind = "identity", mechanism = { bound = "_erasure_apply_redaction" } }

[tables.kb_events]
grain = { kind = "declared-out", reason = "the ledger is append-only" }

[tables.kb_join_requests]
grain = { kind = "declared-out", reason = "kept under the deployment's policy", retention = "moderation-record" }
"#;

    const PERSONAL: &str = r#"
[columns]
"kb_profiles.id" = { class = "reference", ceiling = "pseudonym", achieved = "-", note = "THE pseudonym" }
"kb_profiles.email" = { class = "identifier", ceiling = "full", achieved = "erased:7" }
"kb_chunks.embedding" = { class = "derived", ceiling = "full", achieved = "resource-erasure:D2.3 + erased:4", note = "estate resources" }
"kb_blob_upload_segments.bytes" = { class = "content", ceiling = "full", achieved = "cascaded:6a:kb_blob_uploads" }
"kb_slack_link_intents.slack_principal_id" = { class = "identifier", ceiling = "full", achieved = "deleted:8" }
"kb_invocations.outcome" = { class = "incidental", ceiling = "full", achieved = "remainder:  no arm reaches it " }
"kb_access_grants.principal_table" = { class = "discriminator", ceiling = "n-a", achieved = "-" }
"#;

    const SENSITIVITY: &str = r#"
count_at = "20261023100000"
declared_count = 3

[columns]
"kb_resources.title" = { disposition = "scan", note = "the commonest PII carrier" }
"kb_blobs.content_hash" = { disposition = "structural", note = "sha256" }
"kb_team_invitations.token" = { disposition = "credential" }
"#;

    const RESOURCE: &str = r#"
[tables]
"kb_chunks.embedding" = { disposition = "handled:D2.3", note = "nulled with embedded_with" }
"kb_ingestion_records.source_uri" = { disposition = "handled:D2.7a" }
"kb_remote_sources.uri" = { disposition = "repointed:D2.9:kb_block_provenance.source_id", note = "outside the walk" }
"kb_events.payload" = { disposition = "out-of-scope", note = "the ledger" }

[payload.resource_updated]
"*" = { disposition = "permissive", note = "fields from payloads.rs ResourceUpdated" }
"/title" = { disposition = "redact:title", note = "F3" }

[payload.property_set]
"/value" = { disposition = "redact:property-value", note = "erased:<event_id>", refine = [ { where = { key = "facet" }, disposition = "redact:facet-value" } ] }

[payload.property_set."/property_key"]
disposition = "redact:property-key"
note = "erased-key-<n>"
refine = [
  { where = { key = "doc_type", owner = "kb_resources" }, disposition = "structural", note = "the literal key stays" },
  { where = { key = "facet" }, disposition = "structural" },
]

[payload.metadata]
"/reasoning" = { disposition = "redact:authorship", note = "D4: \"erased\"" }
"#;

    fn files() -> ManifestFiles<'static> {
        ManifestFiles {
            root: ROOT,
            estate: ESTATE,
            personal_data: PERSONAL,
            sensitivity_scan: SENSITIVITY,
            resource_erasure: RESOURCE,
        }
    }

    fn key(raw: &str) -> ColumnKey {
        raw.parse().expect("a well-formed column key")
    }

    /// Parse `files`, expecting a failure in `file`; returns the error's text.
    fn fails_in(files: ManifestFiles<'_>, file: ManifestFile) -> String {
        let err = parse(&files).expect_err("the manifest must not parse");
        assert_eq!(err.file, file, "the error names the wrong file: {err}");
        let text = err.to_string();
        assert!(
            text.contains(file.file_name()),
            "the message names no file: {text}"
        );
        text
    }

    #[test]
    fn a_well_formed_manifest_parses_into_its_values() {
        let policy = parse(&files()).expect("the fixture parses");
        assert_eq!(policy.format, 1);

        let email = &policy.personal_data[&key("kb_profiles.email")];
        assert_eq!(email.class, Class::Identifier);
        assert_eq!(email.ceiling, Ceiling::Full);
        assert_eq!(
            email.achieved,
            Achieved::Claims(vec![Claim::Erased { arm: "7".into() }])
        );
        assert_eq!(
            policy.personal_data[&key("kb_chunks.embedding")].achieved,
            Achieved::Claims(vec![
                Claim::ResourceErasure { step: "3".into() },
                Claim::Erased { arm: "4".into() },
            ])
        );
        assert_eq!(
            policy.personal_data[&key("kb_blob_upload_segments.bytes")].achieved,
            Achieved::Claims(vec![Claim::Cascaded {
                arm: "6a".into(),
                parent: "kb_blob_uploads".into(),
            }])
        );
        assert_eq!(
            policy.personal_data[&key("kb_slack_link_intents.slack_principal_id")].achieved,
            Achieved::Claims(vec![Claim::Deleted { arm: "8".into() }])
        );
        assert_eq!(
            policy.personal_data[&key("kb_invocations.outcome")].achieved,
            Achieved::Remainder("no arm reaches it".into())
        );
        let id = &policy.personal_data[&key("kb_profiles.id")];
        assert_eq!(
            (id.ceiling, &id.achieved, id.note.as_str()),
            (Ceiling::Pseudonym, &Achieved::Unstated, "THE pseudonym")
        );
        assert_eq!(
            policy.personal_data[&key("kb_access_grants.principal_table")].ceiling,
            Ceiling::NotApplicable
        );
        let identifiers: Vec<&str> = policy
            .personal_data_keys(Class::Identifier)
            .map(ColumnKey::as_str)
            .collect();
        assert_eq!(
            identifiers,
            ["kb_profiles.email", "kb_slack_link_intents.slack_principal_id"]
        );

        assert_eq!(policy.sensitivity.count_at, "20261023100000");
        assert_eq!(policy.sensitivity.declared_count, 3);
        let scanned: Vec<&str> = policy
            .sensitivity
            .keys(ScanDisposition::Scan)
            .map(ColumnKey::as_str)
            .collect();
        assert_eq!(scanned, ["kb_resources.title"]);

        let tables = &policy.resource_erasure.tables;
        assert_eq!(
            tables[&key("kb_remote_sources.uri")].disposition,
            Disposition::Repointed {
                step: "9".into(),
                via: key("kb_block_provenance.source_id"),
            }
        );
        assert_eq!(
            tables[&key("kb_events.payload")].disposition,
            Disposition::OutOfScope
        );
        let handled: Vec<(&str, &str)> = policy
            .resource_erasure
            .handled()
            .map(|(k, step)| (k.as_str(), step))
            .collect();
        assert_eq!(
            handled,
            [
                ("kb_chunks.embedding", "3"),
                ("kb_ingestion_records.source_uri", "7a")
            ]
        );

        let payload = &policy.resource_erasure.payload;
        assert_eq!(
            payload["resource_updated"]["*"].disposition,
            PayloadDisposition::Permissive
        );
        let key_line = &payload["property_set"]["/property_key"];
        assert_eq!(
            key_line.disposition,
            PayloadDisposition::Redact("property-key".into())
        );
        assert_eq!(
            key_line.refine[0],
            Refinement {
                r#where: Qualifier {
                    key: Some("doc_type".into()),
                    owner: Some("kb_resources".into()),
                },
                disposition: PayloadDisposition::Structural,
                note: "the literal key stays".into(),
            }
        );
        assert_eq!(
            payload["property_set"]["/value"].refine[0].disposition,
            PayloadDisposition::Redact("facet-value".into())
        );
        assert_eq!(
            payload["metadata"]["/reasoning"].disposition,
            PayloadDisposition::Redact("authorship".into())
        );

        let estate = &policy.estate;
        assert_eq!(
            estate.scope[1].mechanism,
            Mechanism::Gap("2a: R15 cogmap scope".into())
        );
        assert_eq!(
            estate.tables["kb_resources"].grain,
            Grain::PerItem {
                container: false,
                fence: Mechanism::Bound("is_active".into()),
                unit: Mechanism::Bound("resource_erasure_execute".into()),
                home: Some(Home {
                    table: "kb_resource_homes".into(),
                    table_column: "anchor_table".into(),
                    id_column: "anchor_id".into(),
                }),
            }
        );
        assert_eq!(
            estate.tables["kb_edges"].also_reached,
            [Reach {
                path: ParentPath {
                    table: "kb_resources".into(),
                    link: Link::Fk("source_id".into()),
                    via: None,
                },
                mechanism: Mechanism::Bound("resource_erasure_execute".into()),
            }]
        );
        assert_eq!(
            estate.tables["kb_remote_sources"].grain,
            Grain::Covered {
                parent: ParentPath {
                    table: "kb_block_provenance".into(),
                    link: Link::Pair {
                        table_column: "source_kind".into(),
                        id_column: "source_id".into(),
                    },
                    via: Some("source_kind = 'remote_source'; no foreign key".into()),
                },
            }
        );
        assert_eq!(
            estate.tables["kb_profiles"].grain,
            Grain::Identity {
                mechanism: Mechanism::Bound("_erasure_apply_redaction".into()),
            }
        );
        assert_eq!(
            estate.tables["kb_events"].grain,
            Grain::DeclaredOut {
                reason: "the ledger is append-only".into(),
                retention: None,
            }
        );
        assert_eq!(
            estate.tables["kb_join_requests"].grain,
            Grain::DeclaredOut {
                reason: "kept under the deployment's policy".into(),
                retention: Some("moderation-record".into()),
            }
        );
    }

    #[test]
    fn an_unknown_class_fails_and_names_its_file() {
        let personal = PERSONAL.replace(r#"class = "identifier""#, r#"class = "secret""#);
        let text = fails_in(
            ManifestFiles {
                personal_data: &personal,
                ..files()
            },
            ManifestFile::PersonalData,
        );
        assert!(text.contains("secret"), "{text}");
    }

    #[test]
    fn an_unknown_ceiling_scan_or_resource_disposition_fails_and_names_its_file() {
        let personal = PERSONAL.replace(r#"ceiling = "full""#, r#"ceiling = "partial""#);
        fails_in(
            ManifestFiles {
                personal_data: &personal,
                ..files()
            },
            ManifestFile::PersonalData,
        );
        let scan = SENSITIVITY.replace(r#""scan""#, r#""unadjudicated""#);
        fails_in(
            ManifestFiles {
                sensitivity_scan: &scan,
                ..files()
            },
            ManifestFile::SensitivityScan,
        );
        for bad in ["handled:D2.10", "repointed:D2.9", "repointed:D2.9:source_id", "erased"] {
            let resource = RESOURCE.replace("handled:D2.3", bad);
            fails_in(
                ManifestFiles {
                    resource_erasure: &resource,
                    ..files()
                },
                ManifestFile::ResourceErasure,
            );
        }
        let resource = RESOURCE.replace("redact:title", "redact:name");
        fails_in(
            ManifestFiles {
                resource_erasure: &resource,
                ..files()
            },
            ManifestFile::ResourceErasure,
        );
    }

    #[test]
    fn an_unknown_field_fails_in_every_file() {
        let root = "format = 1\nversion = 2\n";
        fails_in(ManifestFiles { root, ..files() }, ManifestFile::Root);
        let estate = ESTATE.replace(
            r#"reason = "the ledger is append-only""#,
            r#"reason = "the ledger is append-only", why = "x""#,
        );
        fails_in(
            ManifestFiles {
                estate: &estate,
                ..files()
            },
            ManifestFile::Estate,
        );
        let personal = PERSONAL.replace(r#"achieved = "erased:7""#, r#"achieved = "erased:7", arm = "7""#);
        fails_in(
            ManifestFiles {
                personal_data: &personal,
                ..files()
            },
            ManifestFile::PersonalData,
        );
        let scan = SENSITIVITY.replace(r#"note = "sha256""#, r#"notes = "sha256""#);
        fails_in(
            ManifestFiles {
                sensitivity_scan: &scan,
                ..files()
            },
            ManifestFile::SensitivityScan,
        );
        let resource = RESOURCE.replace(r#"note = "F3""#, r#"note = "F3", class = "title""#);
        fails_in(
            ManifestFiles {
                resource_erasure: &resource,
                ..files()
            },
            ManifestFile::ResourceErasure,
        );
    }

    #[test]
    fn a_column_declared_twice_fails() {
        let personal = format!(
            "{PERSONAL}\"kb_profiles.email\" = {{ class = \"identifier\", ceiling = \"full\", achieved = \"erased:7\" }}\n"
        );
        fails_in(
            ManifestFiles {
                personal_data: &personal,
                ..files()
            },
            ManifestFile::PersonalData,
        );
    }

    #[test]
    fn a_malformed_claim_fails() {
        for bad in [
            "erased:99",
            "cascaded:6a",
            "resource-erasure:D2.10",
            "scrubbed:7",
            "erased:7 + ",
        ] {
            let personal = PERSONAL.replace(
                r#"achieved = "erased:7""#,
                &format!("achieved = \"{bad}\""),
            );
            fails_in(
                ManifestFiles {
                    personal_data: &personal,
                    ..files()
                },
                ManifestFile::PersonalData,
            );
        }
    }

    #[test]
    fn a_qualifier_with_an_unknown_key_fails() {
        let resource = RESOURCE.replace(
            r#"where = { key = "facet" }, disposition = "structural""#,
            r#"where = { key = "facet", table = "kb_edges" }, disposition = "structural""#,
        );
        fails_in(
            ManifestFiles {
                resource_erasure: &resource,
                ..files()
            },
            ManifestFile::ResourceErasure,
        );
    }

    #[test]
    fn an_empty_or_repeated_qualifier_fails() {
        let empty = RESOURCE.replace(r#"where = { key = "facet" }, disposition = "structural""#, r#"where = {}, disposition = "structural""#);
        fails_in(
            ManifestFiles {
                resource_erasure: &empty,
                ..files()
            },
            ManifestFile::ResourceErasure,
        );
        let repeated = RESOURCE.replace(
            r#"where = { key = "facet" }, disposition = "structural""#,
            r#"where = { key = "doc_type", owner = "kb_resources" }, disposition = "structural""#,
        );
        fails_in(
            ManifestFiles {
                resource_erasure: &repeated,
                ..files()
            },
            ManifestFile::ResourceErasure,
        );
    }

    #[test]
    fn a_payload_path_is_a_pointer_or_a_permissive_star() {
        for (from, to) in [
            (r#""/title" = { disposition"#, r#""title" = { disposition"#),
            (
                r#""*" = { disposition = "permissive""#,
                r#""*" = { disposition = "structural""#,
            ),
            (
                r#""/title" = { disposition = "redact:title""#,
                r#""/title" = { disposition = "permissive""#,
            ),
        ] {
            let resource = RESOURCE.replace(from, to);
            assert_ne!(resource, RESOURCE, "the probe must change the fixture");
            fails_in(
                ManifestFiles {
                    resource_erasure: &resource,
                    ..files()
                },
                ManifestFile::ResourceErasure,
            );
        }
    }

    #[test]
    fn a_mechanism_names_exactly_one_of_bound_or_gap() {
        let estate = ESTATE.replace(
            r#"unit = { gap = "2a: context unit" }"#,
            r#"unit = { gap = "2a: context unit", bound = "context_erasure_execute" }"#,
        );
        fails_in(
            ManifestFiles {
                estate: &estate,
                ..files()
            },
            ManifestFile::Estate,
        );
    }

    #[test]
    fn an_unknown_format_fails() {
        fails_in(
            ManifestFiles {
                root: "format = 2\n",
                ..files()
            },
            ManifestFile::Root,
        );
    }

    #[test]
    fn an_absent_note_parses_as_empty() {
        let policy = parse(&files()).expect("the fixture parses");
        assert_eq!(policy.personal_data[&key("kb_profiles.email")].note, "");
        assert_eq!(
            policy.sensitivity.columns[&key("kb_team_invitations.token")].note,
            ""
        );
        assert_eq!(
            policy.resource_erasure.tables[&key("kb_ingestion_records.source_uri")].note,
            ""
        );
        assert_eq!(
            policy.resource_erasure.payload["property_set"]["/property_key"].refine[1].note,
            ""
        );
        assert_eq!(policy.estate.tables["kb_resources"].note, "");
    }

    #[test]
    fn a_column_key_needs_both_halves() {
        for bad in ["kb_profiles", ".email", "kb_profiles.", ""] {
            assert!(bad.parse::<ColumnKey>().is_err(), "{bad:?} parsed");
        }
        let k = key("kb_profiles.email");
        assert_eq!((k.table(), k.column()), ("kb_profiles", "email"));
    }
}
