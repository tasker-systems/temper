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
//! `"handled"`, `"redact:title"`) and are read through `FromStr` into the enums below; an unknown
//! word is a parse error that names its file, line and column. Every `note` is trimmed as it is
//! read, so a note of only whitespace reads as empty.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};

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
pub use estate::{
    CitedBy, Estate, Gap, Grain, Home, Link, Mechanism, ParentPath, Reach, ScopeRule, TableGrain,
};
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
    pub sensitivity_scan: SensitivityScan,
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

/// A `note`, trimmed, so a note of only whitespace reads as empty.
fn trimmed<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(String::deserialize(deserializer)?.trim().to_owned())
}

fn read<T: DeserializeOwned>(file: ManifestFile, text: &str) -> Result<T, ParseError> {
    toml::from_str(text).map_err(|source| ParseError { file, source })
}

/// Parse the five manifest files as one erasure policy.
pub fn parse(files: &ManifestFiles<'_>) -> Result<ErasurePolicy, ParseError> {
    let root: RootFile = read(ManifestFile::Root, files.root)?;
    let estate: Estate = read(ManifestFile::Estate, files.estate)?;
    let personal: PersonalDataFile = read(ManifestFile::PersonalData, files.personal_data)?;
    let sensitivity_scan: SensitivityScan =
        read(ManifestFile::SensitivityScan, files.sensitivity_scan)?;
    let resource_erasure: ResourceErasure =
        read(ManifestFile::ResourceErasure, files.resource_erasure)?;
    Ok(ErasurePolicy {
        format: root.format,
        estate,
        personal_data: personal.columns,
        sensitivity_scan,
        resource_erasure,
    })
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    const ROOT: &str = "format = 1\n";

    /// The task the fixture's gaps name.
    const TASK: &str = "01a12dcf-2ef5-729d-b6ef-051fdee3ef8c";

    const ESTATE: &str = r#"
scope = [
  { rule = "the subject's @me contexts", mechanism = { bound = "_erasure_estate_contexts" } },
  { rule = "cogmaps whose only team join is the personal team", mechanism = { gap = { task = "01a12dcf-2ef5-729d-b6ef-051fdee3ef8c", what = "cogmaps whose only kb_team_cogmaps row is the personal team" } } },
]

[tables.kb_resources]
grain = { kind = "per-item", container = false, fence = { bound = "is_active" }, unit = { bound = "resource_erasure_execute" }, home = { table = "kb_resource_homes", table_column = "anchor_table", id_column = "anchor_id" } }

[tables.kb_contexts]
grain = { kind = "per-item", container = true, fence = { bound = "is_active" }, unit = { gap = { task = "01a12dcf-2ef5-729d-b6ef-051fdee3ef8c", what = "the unit that erases one context" } } }

[tables.kb_edges]
grain = { kind = "per-item", container = false, fence = { bound = "is_folded" }, unit = { gap = { task = "01a12dcf-2ef5-729d-b6ef-051fdee3ef8c", what = "label redaction" } } }
also_reached = [
  { path = { table = "kb_resources", link = { fk = "source_id" } }, mechanism = { bound = "resource_erasure_execute" } },
]

[tables.kb_properties]
grain = { kind = "covered", parent = { table = "kb_resources", link = { pair = { table_column = "owner_table", id_column = "owner_id" } } } }

[tables.kb_remote_sources]
grain = { kind = "covered", parent = { table = "kb_block_provenance", link = { ref = "source_id" }, cited_by = { column = "source_kind", value = "remote" } } }
note = "re-pointed, not erased"

[tables.kb_profiles]
grain = { kind = "identity", mechanism = { bound = "_erasure_apply_redaction" } }

[tables.kb_oauth_refresh_tokens]
grain = { kind = "identity", mechanism = { bound = "_erasure_apply_redaction" }, retention = "credential-replay-evidence" }

[tables.kb_events]
grain = { kind = "declared-out", reason = "the ledger is append-only" }

[tables.kb_join_requests]
grain = { kind = "declared-out", reason = "kept under the deployment's policy", retention = "moderation-record" }
"#;

    const PERSONAL: &str = r#"
[columns]
"kb_profiles.id" = { class = "reference", ceiling = "pseudonym", achieved = "-", note = "THE pseudonym" }
"kb_profiles.email" = { class = "identifier", ceiling = "full", achieved = "erased:7" }
"kb_chunks.embedding" = { class = "derived", ceiling = "full", achieved = "resource-erasure + erased:4", note = "estate resources" }
"kb_blob_upload_segments.bytes" = { class = "content", ceiling = "full", achieved = "cascaded:6a:kb_blob_uploads" }
"kb_slack_link_intents.slack_principal_id" = { class = "identifier", ceiling = "full", achieved = "deleted:8" }
"kb_invocations.outcome" = { class = "incidental", ceiling = "full", achieved = "remainder:  no arm reaches it " }
"kb_access_grants.principal_table" = { class = "discriminator", ceiling = "n-a", achieved = "-" }
"#;

    const SENSITIVITY_SCAN: &str = r#"
count_at = "20261023100000"
declared_count = 3

[columns]
"kb_resources.title" = { disposition = "scan", note = "the commonest carrier of personal data" }
"kb_blobs.content_hash" = { disposition = "structural", note = "sha256" }
"kb_team_invitations.token" = { disposition = "credential" }
"#;

    const RESOURCE: &str = r#"
[tables]
"kb_chunks.embedding" = { disposition = "handled", note = "nulled with embedded_with" }
"kb_ingestion_records.source_uri" = { disposition = "handled" }
"kb_remote_sources.uri" = { disposition = "repointed:kb_block_provenance.source_id", note = "outside the walk" }
"kb_events.payload" = { disposition = "out-of-scope", note = "the ledger" }

[payload.resource_updated]
"*" = { disposition = "permissive", note = "fields from payloads.rs ResourceUpdated" }
"/title" = { disposition = "redact:title", note = "a title" }

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
"/reasoning" = { disposition = "redact:authorship", note = "the sentinel \"erased\"" }
"#;

    fn files() -> ManifestFiles<'static> {
        ManifestFiles {
            root: ROOT,
            estate: ESTATE,
            personal_data: PERSONAL,
            sensitivity_scan: SENSITIVITY_SCAN,
            resource_erasure: RESOURCE,
        }
    }

    fn key(raw: &str) -> ColumnKey {
        raw.parse().expect("a well-formed column key")
    }

    fn gap(what: &str) -> Mechanism {
        Mechanism::Gap(Gap {
            task: Uuid::parse_str(TASK).expect("the fixture's task is a uuid"),
            what: what.into(),
        })
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

    /// `ESTATE` with `from` replaced by `to`, asserting the replacement changed it.
    fn estate_with(from: &str, to: &str) -> String {
        let estate = ESTATE.replace(from, to);
        assert_ne!(estate, ESTATE, "the probe must change the fixture");
        estate
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
                Claim::ResourceErasure,
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
            [
                "kb_profiles.email",
                "kb_slack_link_intents.slack_principal_id"
            ]
        );

        let scan = &policy.sensitivity_scan;
        assert_eq!(scan.count_at, "20261023100000");
        assert_eq!(scan.declared_count, 3);
        let scanned: Vec<&str> = scan
            .keys(ScanDisposition::Scan)
            .map(ColumnKey::as_str)
            .collect();
        assert_eq!(scanned, ["kb_resources.title"]);

        let tables = &policy.resource_erasure.tables;
        assert_eq!(
            tables[&key("kb_remote_sources.uri")].disposition,
            Disposition::Repointed {
                via: key("kb_block_provenance.source_id"),
            }
        );
        assert_eq!(
            tables[&key("kb_events.payload")].disposition,
            Disposition::OutOfScope
        );
        let handled: Vec<&str> = policy
            .resource_erasure
            .handled()
            .map(ColumnKey::as_str)
            .collect();
        assert_eq!(
            handled,
            ["kb_chunks.embedding", "kb_ingestion_records.source_uri"]
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
            gap("cogmaps whose only kb_team_cogmaps row is the personal team")
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
                    cited_by: None,
                },
                mechanism: Mechanism::Bound("resource_erasure_execute".into()),
                column: None,
            }]
        );
        assert_eq!(
            estate.tables["kb_remote_sources"].grain,
            Grain::Covered {
                parent: ParentPath {
                    table: "kb_block_provenance".into(),
                    link: Link::Ref("source_id".into()),
                    cited_by: Some(CitedBy {
                        column: "source_kind".into(),
                        value: "remote".into(),
                    }),
                },
                unit: None,
            }
        );
        assert_eq!(
            estate.tables["kb_profiles"].grain,
            Grain::Identity {
                mechanism: Mechanism::Bound("_erasure_apply_redaction".into()),
                retention: None,
            }
        );
        assert_eq!(
            estate.tables["kb_oauth_refresh_tokens"].grain,
            Grain::Identity {
                mechanism: Mechanism::Bound("_erasure_apply_redaction".into()),
                retention: Some("credential-replay-evidence".into()),
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
        let scan = SENSITIVITY_SCAN.replace(r#""scan""#, r#""unadjudicated""#);
        fails_in(
            ManifestFiles {
                sensitivity_scan: &scan,
                ..files()
            },
            ManifestFile::SensitivityScan,
        );
        for bad in ["handled:3", "repointed", "repointed:source_id", "erased"] {
            let resource = RESOURCE.replace(
                r#"{ disposition = "handled", note = "nulled"#,
                &format!(r#"{{ disposition = "{bad}", note = "nulled"#),
            );
            assert_ne!(resource, RESOURCE, "the probe must change the fixture");
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
        let estate = estate_with(
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
        let personal = PERSONAL.replace(
            r#"achieved = "erased:7""#,
            r#"achieved = "erased:7", arm = "7""#,
        );
        fails_in(
            ManifestFiles {
                personal_data: &personal,
                ..files()
            },
            ManifestFile::PersonalData,
        );
        let scan = SENSITIVITY_SCAN.replace(r#"note = "sha256""#, r#"notes = "sha256""#);
        fails_in(
            ManifestFiles {
                sensitivity_scan: &scan,
                ..files()
            },
            ManifestFile::SensitivityScan,
        );
        let resource = RESOURCE.replace(
            r#"note = "a title""#,
            r#"note = "a title", class = "title""#,
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
            "resource-erasure:3",
            "scrubbed:7",
            "erased:7 + ",
        ] {
            let personal =
                PERSONAL.replace(r#"achieved = "erased:7""#, &format!("achieved = \"{bad}\""));
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
        let empty = RESOURCE.replace(
            r#"where = { key = "facet" }, disposition = "structural""#,
            r#"where = {}, disposition = "structural""#,
        );
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

    /// FAILS IF: a `*` line carries a refinement. A permissive refinement of `*` never becomes an
    /// allowlist row, so it would read as a declaration that does nothing.
    #[test]
    fn a_star_line_takes_no_refinement() {
        let resource = RESOURCE.replace(
            r#""*" = { disposition = "permissive", note = "fields from payloads.rs ResourceUpdated" }"#,
            r#""*" = { disposition = "permissive", note = "fields from payloads.rs ResourceUpdated", refine = [ { where = { key = "x" }, disposition = "permissive" } ] }"#,
        );
        assert_ne!(resource, RESOURCE, "the probe must change the fixture");
        let text = fails_in(
            ManifestFiles {
                resource_erasure: &resource,
                ..files()
            },
            ManifestFile::ResourceErasure,
        );
        assert!(text.contains("`*` takes no refinement"), "{text}");
    }

    #[test]
    fn a_mechanism_names_exactly_one_of_bound_or_gap() {
        let estate = estate_with(
            r#"unit = { gap = { task = "01a12dcf-2ef5-729d-b6ef-051fdee3ef8c", what = "label redaction" } }"#,
            r#"unit = { gap = { task = "01a12dcf-2ef5-729d-b6ef-051fdee3ef8c", what = "label redaction" }, bound = "edge_erasure_execute" }"#,
        );
        fails_in(
            ManifestFiles {
                estate: &estate,
                ..files()
            },
            ManifestFile::Estate,
        );
    }

    /// FAILS IF: a gap parses without naming its task as a whole UUID, or without saying what is
    /// uncovered. A gap is the record of uncovered work, so both halves are required.
    #[test]
    fn a_gap_names_a_well_formed_task_and_what_is_uncovered() {
        let well_formed = r#"{ gap = { task = "01a12dcf-2ef5-729d-b6ef-051fdee3ef8c", what = "label redaction" } }"#;
        for (bad, named) in [
            (r#"{ gap = { what = "label redaction" } }"#, "task"),
            (
                r#"{ gap = { task = "01a12dcf", what = "label redaction" } }"#,
                "01a12dcf",
            ),
            (
                r#"{ gap = { task = "01a12dcf2ef5729db6ef051fdee3ef8c", what = "label redaction" } }"#,
                "01a12dcf2ef5729db6ef051fdee3ef8c",
            ),
            (
                r#"{ gap = { task = "01A12DCF-2EF5-729D-B6EF-051FDEE3EF8C", what = "label redaction" } }"#,
                "01A12DCF",
            ),
            (
                r#"{ gap = { task = "01a12dcf-2ef5-729d-b6ef-051fdee3ef8c" } }"#,
                "what",
            ),
            (
                r#"{ gap = { task = "01a12dcf-2ef5-729d-b6ef-051fdee3ef8c", what = "   " } }"#,
                "blank",
            ),
            (
                r#"{ gap = { task = "01a12dcf-2ef5-729d-b6ef-051fdee3ef8c", what = "label redaction", due = "soon" } }"#,
                "due",
            ),
            (r#"{ gap = "label redaction" }"#, "struct Gap"),
        ] {
            let estate = estate_with(&format!("unit = {well_formed}"), &format!("unit = {bad}"));
            let text = fails_in(
                ManifestFiles {
                    estate: &estate,
                    ..files()
                },
                ManifestFile::Estate,
            );
            assert!(text.contains(named), "{bad}: {text}");
        }
    }

    /// FAILS IF: a value that states something parses blank: a scope rule, a declared-out reason,
    /// or a retention category on an identity or declared-out grain.
    #[test]
    fn a_rule_reason_or_retention_is_never_blank() {
        for (from, to) in [
            (
                r#"rule = "the subject's @me contexts""#,
                r#"rule = "  ""#,
            ),
            (
                r#"reason = "the ledger is append-only""#,
                r#"reason = " ""#,
            ),
            (
                r#"retention = "credential-replay-evidence""#,
                r#"retention = "  ""#,
            ),
            (
                r#"retention = "moderation-record""#,
                r#"retention = """#,
            ),
        ] {
            let estate = estate_with(from, to);
            let text = fails_in(
                ManifestFiles {
                    estate: &estate,
                    ..files()
                },
                ManifestFile::Estate,
            );
            assert!(text.contains("blank"), "{to}: {text}");
        }
    }

    #[test]
    fn a_rule_reason_or_retention_is_trimmed() {
        let estate = estate_with(
            r#"reason = "the ledger is append-only""#,
            r#"reason = " the ledger is append-only ""#,
        );
        let policy = parse(&ManifestFiles {
            estate: &estate,
            ..files()
        })
        .expect("a padded reason parses");
        assert_eq!(
            policy.estate.tables["kb_events"].grain,
            Grain::DeclaredOut {
                reason: "the ledger is append-only".into(),
                retention: None,
            }
        );
    }

    #[test]
    fn a_gap_what_is_trimmed() {
        let estate = estate_with(
            r#"what = "label redaction""#,
            r#"what = "  label redaction ""#,
        );
        let policy = parse(&ManifestFiles {
            estate: &estate,
            ..files()
        })
        .expect("a padded what parses");
        let Grain::PerItem { unit, .. } = &policy.estate.tables["kb_edges"].grain else {
            panic!("kb_edges is per-item in the fixture");
        };
        assert_eq!(*unit, gap("label redaction"));
    }

    #[test]
    fn a_covered_grain_may_name_its_own_unit() {
        let estate = format!(
            "{ESTATE}\n[tables.kb_invocations]\n\
             grain = {{ kind = \"covered\", parent = {{ table = \"kb_cogmaps\", link = {{ fk = \"originating_cogmap_id\" }} }}, unit = {{ gap = {{ task = \"{TASK}\", what = \"invocation outcomes\" }} }} }}\n"
        );
        let policy = parse(&ManifestFiles {
            estate: &estate,
            ..files()
        })
        .expect("a covered grain with a unit parses");
        assert_eq!(
            policy.estate.tables["kb_invocations"].grain,
            Grain::Covered {
                parent: ParentPath {
                    table: "kb_cogmaps".into(),
                    link: Link::Fk("originating_cogmap_id".into()),
                    cited_by: None,
                },
                unit: Some(gap("invocation outcomes")),
            }
        );
        let Grain::Covered { unit, .. } = &policy.estate.tables["kb_properties"].grain else {
            panic!("kb_properties is covered in the fixture");
        };
        assert_eq!(*unit, None, "an absent unit is None");

        let estate = estate.replace(r#"unit = { gap = { task"#, r#"unit = { gone = { task"#);
        fails_in(
            ManifestFiles {
                estate: &estate,
                ..files()
            },
            ManifestFile::Estate,
        );
    }

    #[test]
    fn a_reach_may_name_the_one_column_it_is_for() {
        let estate = format!(
            "{ESTATE}\n[tables.kb_teams]\n\
             grain = {{ kind = \"identity\", mechanism = {{ bound = \"_erasure_apply_redaction\" }} }}\n\
             [[tables.kb_teams.also_reached]]\n\
             path = {{ table = \"kb_profiles\", link = {{ ref = \"personal_of\" }} }}\n\
             mechanism = {{ gap = {{ task = \"{TASK}\", what = \"the personal team's description\" }} }}\n\
             column = \"description\"\n"
        );
        let policy = parse(&ManifestFiles {
            estate: &estate,
            ..files()
        })
        .expect("a reach with a column parses");
        assert_eq!(
            policy.estate.tables["kb_teams"].also_reached,
            [Reach {
                path: ParentPath {
                    table: "kb_profiles".into(),
                    link: Link::Ref("personal_of".into()),
                    cited_by: None,
                },
                mechanism: gap("the personal team's description"),
                column: Some("description".into()),
            }]
        );
        assert_eq!(
            policy.estate.tables["kb_edges"].also_reached[0].column, None,
            "an absent column is None"
        );
    }

    #[test]
    fn a_reverse_citation_names_its_column_and_value() {
        let estate = estate_with(
            r#"cited_by = { column = "source_kind", value = "remote" }"#,
            r#"cited_by = { column = "source_kind" }"#,
        );
        fails_in(
            ManifestFiles {
                estate: &estate,
                ..files()
            },
            ManifestFile::Estate,
        );
        let estate = estate_with(
            r#"cited_by = { column = "source_kind", value = "remote" }"#,
            r#"via = "source_kind = 'remote'""#,
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
            policy.sensitivity_scan.columns[&key("kb_team_invitations.token")].note,
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

    /// FAILS IF: a note of only whitespace reads as a note. The fences that require a reason test
    /// `note.is_empty()`, so an untrimmed blank note would pass them.
    #[test]
    fn a_blank_note_parses_as_empty() {
        let personal = PERSONAL.replace(r#"note = "THE pseudonym""#, r#"note = "   ""#);
        let scan = SENSITIVITY_SCAN.replace(r#"note = "sha256""#, r#"note = " ""#);
        let resource = RESOURCE
            .replace(r#"note = "the ledger""#, r#"note = "  ""#)
            .replace(r#"note = "the literal key stays""#, r#"note = "\t""#);
        let estate = estate_with(r#"note = "re-pointed, not erased""#, r#"note = "  ""#);
        for changed in [&personal, &scan, &resource] {
            assert!(
                ![PERSONAL, SENSITIVITY_SCAN, RESOURCE].contains(&changed.as_str()),
                "the probe must change the fixture"
            );
        }
        let policy = parse(&ManifestFiles {
            personal_data: &personal,
            sensitivity_scan: &scan,
            resource_erasure: &resource,
            estate: &estate,
            ..files()
        })
        .expect("blank notes parse");
        assert_eq!(policy.personal_data[&key("kb_profiles.id")].note, "");
        assert_eq!(
            policy.sensitivity_scan.columns[&key("kb_blobs.content_hash")].note,
            ""
        );
        assert_eq!(
            policy.resource_erasure.tables[&key("kb_events.payload")].note,
            ""
        );
        assert_eq!(
            policy.resource_erasure.payload["property_set"]["/property_key"].refine[0].note,
            ""
        );
        assert_eq!(policy.estate.tables["kb_remote_sources"].note, "");

        let padded = parse(&ManifestFiles {
            resource_erasure: &RESOURCE
                .replace(r#"note = "the ledger""#, r#"note = " the ledger ""#),
            ..files()
        })
        .expect("a padded note parses");
        assert_eq!(
            padded.resource_erasure.tables[&key("kb_events.payload")].note,
            "the ledger"
        );
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
