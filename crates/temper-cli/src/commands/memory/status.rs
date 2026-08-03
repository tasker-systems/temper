//! `temper memory status` — the discovery surface.
//!
//! The whole point of this command is that it works on a machine that has
//! never opted into the `[memory]` config section: an absent section is not
//! an error, it is the primary case this command exists to report on.
//!
//! [`status_report`] is pure over its three inputs (config, fetched rows,
//! local files) — no I/O, no clock — so the matching and defect-collection
//! logic is unit-testable without a server or a filesystem. [`status`] is the
//! thin I/O shell: it resolves what to fetch/scan from `config.memory`, calls
//! the server (when opted in), reads local files (when opted in), and prints.

use std::collections::HashSet;

use serde::Serialize;

use temper_core::types::config::{expand_tilde, MemoryConfig, TemperConfig};
use temper_workflow::types::resource::ResourceDetail;

use super::fetch::fetch_context_rows;
use super::render::{parse_entry, reinforcement_of};
use crate::actions::runtime::build_config_store_and_client;
use crate::error::Result;
use crate::format::{self, OutputFormat};
use crate::output;

/// One `.md` file found in the directory that holds the rendered index (`index_path`'s parent).
/// Carries only the filename — matching against Temper is against `open_meta.source_file`
/// (an exact filename, never a full path), so a report stays legible across machines whose vault
/// lives at different absolute paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalMemoryFile {
    pub filename: String,
}

/// This machine's memory state — the discovery report.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryStatus {
    /// Whether `[memory]` is configured at all. `false` is not an error state.
    pub opted_in: bool,
    /// Every context this machine would render for, shared first (empty when not opted in).
    pub contexts: Vec<String>,
    /// Count of fetched rows that parsed cleanly into a [`super::render::MemoryEntry`].
    pub in_temper: usize,
    /// One rendered [`super::render::MemoryDefect`] message per row that failed to parse —
    /// reported, never fatal. Only `emit` refuses on a defect.
    pub defects: Vec<String>,
    /// Count of `.md` files found alongside the index (excluding the index file itself).
    pub local_files: usize,
    /// Filenames present locally that no fetched row's `open_meta.source_file` names. An
    /// unadopted machine's local files are all reported here — that is the point: discovery must
    /// say what this machine is carrying even with nothing to compare it against. A memory whose
    /// `source_file` is absent (authored natively in Temper — from a session, from Desktop) is
    /// ordinary and simply does not contribute a match; it is never itself reported as orphaned,
    /// since orphaning is a property of local files, not of Temper resources.
    pub local_without_counterpart: Vec<String>,
    /// How much of the corpus has been reinforced, and how recently. This is the evidence a
    /// threshold would eventually be set from — see [`ReinforcementDistribution`].
    pub reinforcement: ReinforcementDistribution,
}

/// The reinforcement distribution across every fetched memory.
///
/// **This exists to be read before a threshold is chosen, not after.** `reinforced_min` is
/// deliberately absent from every config in existence, and the only honest way to pick one is to
/// look at what the corpus actually does over months. So this is the instrument, and today it
/// reports `reinforced: 0` on every machine — which is information, not a bug.
///
/// **The population is every fetched `memory` row — the same one [`MemoryStatus::in_temper`]
/// counts, which is NOT the population the index tail collapses.** `fetch_context_rows` returns
/// superseded rows too, and the render drops those before it ever reaches the threshold
/// (`render::render_migrated` filters `status == "active"`). So `never_reinforced` is a strict
/// over-count of what a tail line would say, by the number of superseded memories, and the two
/// numbers are expected to differ.
///
/// That is the right population for this field's actual job — *is the convention being used at
/// all, and by how much of what I am carrying* — but it is emphatically not "what the tail will
/// hide", and reading it as that will mislead. Stated here rather than reconciled, because making
/// the distribution tail-shaped would break its agreement with `in_temper` and buy one consistency
/// with another.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReinforcementDistribution {
    /// Memories carrying at least one well-formed reinforcement date.
    pub reinforced: usize,
    /// Memories carrying none. **A malformed record counts here**, matching how the render treats
    /// it — on that one axis the two agree. See the type doc for the axis on which they do not.
    pub never_reinforced: usize,
    /// The most recent reinforcement anywhere in the corpus, or `None` if nothing has been
    /// reinforced. Not a per-memory field: this answers "is the convention in use at all?".
    pub last_reinforced: Option<chrono::NaiveDate>,
    /// One line per memory whose `open_meta.reinforced` could not be read — **reported, never
    /// fatal, and not a [`super::render::MemoryDefect`]** (`[decided — 2026-08-02, Pete]`). Unlike
    /// `defects`, nothing anywhere refuses on these: `emit` renders straight through and treats
    /// the memory as unreinforced.
    pub malformed: Vec<String>,
}

/// Extract `open_meta.source_file` from a row, if present. Independent of whether the row parses
/// cleanly via [`parse_entry`] — a memory with a malformed `status`/`verified` is still a real
/// migrated file and should still vouch for its local counterpart; that a memory contributes a
/// defect and that it contributes provenance are orthogonal facts about it.
pub(super) fn source_file_of(row: &ResourceDetail) -> Option<String> {
    row.open_meta
        .as_ref()?
        .get("source_file")?
        .as_str()
        .map(str::to_owned)
}

/// Pure core of `status`. Takes the memory config (`None` = feature off), every fetched
/// `memory`-typed row across all configured contexts, and every local `.md` file found next to
/// the index — and reports, without failing on a malformed row.
///
/// Matching a local file to a Temper memory is by exact filename against `open_meta.source_file`
/// — never a title or a sluggified/derived form. A migrated memory's human-readable title
/// ("a clause cannot retire its own goal") is never its filename
/// ("feedback_a_clause_cannot_retire_its_own_goal.md"), so any comparison through title would
/// misreport every migrated memory's local file as orphaned. `source_file` is optional (a memory
/// authored natively in Temper never had one), so its absence on a given row simply means that
/// row contributes no match — never a defect, and never a false orphan for some unrelated file.
pub fn status_report(
    cfg: Option<&MemoryConfig>,
    rows: &[ResourceDetail],
    local: &[LocalMemoryFile],
) -> MemoryStatus {
    let contexts = cfg
        .map(|c| c.all_contexts().into_iter().map(String::from).collect())
        .unwrap_or_default();

    let mut in_temper = 0usize;
    let mut defects = Vec::new();
    let mut known_source_files: HashSet<String> = HashSet::new();
    let mut reinforcement = ReinforcementDistribution::default();

    for row in rows {
        if let Some(source_file) = source_file_of(row) {
            known_source_files.insert(source_file);
        }

        // Read UNCONDITIONALLY, before the parse match, so a row that is *both* a hard defect and
        // malformed on `reinforced` contributes both reports. Reading it inside the `Ok` arm would
        // silently drop the soft report for exactly the rows most likely to have one — a memory
        // whose open tier is wrong in one place is the memory most likely to be wrong in another.
        let r = reinforcement_of(row);
        if let Some(msg) = &r.malformed {
            reinforcement.malformed.push(format!(
                "{} \"{}\": {msg}",
                row.row.id.uuid(),
                row.row.title
            ));
        }
        if r.dates.is_empty() {
            reinforcement.never_reinforced += 1;
        } else {
            reinforcement.reinforced += 1;
            reinforcement.last_reinforced = reinforcement.last_reinforced.max(r.last());
        }

        match parse_entry(row) {
            Ok(_) => in_temper += 1,
            Err(defect) => defects.push(defect.to_string()),
        }
    }

    let local_without_counterpart = local
        .iter()
        .filter(|f| !known_source_files.contains(&f.filename))
        .map(|f| f.filename.clone())
        .collect();

    MemoryStatus {
        opted_in: cfg.is_some(),
        contexts,
        in_temper,
        defects,
        local_files: local.len(),
        local_without_counterpart,
        reinforcement,
    }
}

/// The I/O shell. Reads `config.memory`: `None` means the feature is off, and this still
/// returns `Ok` with a report saying so — an unadopted machine is the primary case, not an
/// error path. When opted in, reads local `.md` files from the directory containing
/// `index_path`, fetches every `memory`-typed resource in each configured context, and prints
/// the resulting [`MemoryStatus`] via [`format::render`] (JSON/TOON per `output_format`).
///
/// Client construction and each per-context fetch degrade gracefully (a warning on stderr, not
/// a hard failure) rather than refusing the whole report — an opted-in-but-unauthenticated or
/// momentarily-unreachable machine still gets a local-files report instead of nothing, matching
/// this command's status as the discovery surface.
pub async fn status(config: &TemperConfig, output_format: OutputFormat) -> Result<()> {
    let mem = config.memory.as_ref();

    let local = mem
        .map(|m| read_local_files(&m.index_path))
        .unwrap_or_default();

    let mut rows: Vec<ResourceDetail> = Vec::new();
    if let Some(mem) = mem {
        match build_config_store_and_client() {
            Ok((_cfg, _store, client)) => {
                for ctx in mem.all_contexts() {
                    match fetch_context_rows(&client, ctx).await {
                        Ok(mut ctx_rows) => rows.append(&mut ctx_rows),
                        Err(e) => {
                            output::warning(format!("could not list memories in {ctx}: {e}"));
                        }
                    }
                }
            }
            Err(e) => {
                output::warning(format!(
                    "cloud unreachable — reporting local files only ({e})"
                ));
            }
        }
    }

    let report = status_report(mem, &rows, &local);
    let rendered = format::render(&report, output_format)?;
    println!("{rendered}");
    Ok(())
}

/// List the memory files in the directory containing `index_path`.
///
/// **Delegates to `migrate::scan_memory_dir` rather than enumerating the directory itself.** These
/// were two independent scans of one concept, and on 2026-08-03 they drifted exactly as two copies
/// do: the index moved to a sibling filename, the frontmatter rule that keeps Claude Code's own
/// `MEMORY.md` from being read as an un-migrated memory was added here, and `emit` — which scans
/// through `scan_memory_dir` — went on counting it. `status` said one un-migrated file while the
/// index it validates said two. One scanner is the fix; agreeing today would not have been.
///
/// Returns an empty list (never an error) when the directory or its parent is unreachable — a
/// machine mid-adoption whose configured directory does not exist yet still gets a report.
fn read_local_files(index_path: &str) -> Vec<LocalMemoryFile> {
    super::migrate::scan_memory_dir(&expand_tilde(index_path))
        .into_iter()
        .map(|f| LocalMemoryFile {
            filename: f.filename,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use serde_json::json;
    use temper_core::types::ids::{ProfileId, ResourceId};
    use temper_workflow::types::resource::{BodyStorage, IngestState, ResourceRow};
    use uuid::Uuid;

    fn cfg() -> MemoryConfig {
        MemoryConfig {
            shared_contexts: vec![],
            project_contexts: vec!["@me/temper".to_string()],
            index_path: "~/.claude/projects/p/memory/MEMORY.md".to_string(),
            stale_after_days: 90,
            reinforced_min: None,
        }
    }

    fn local(filename: &str) -> LocalMemoryFile {
        LocalMemoryFile {
            filename: filename.to_string(),
        }
    }

    /// Build a `ResourceDetail` titled `title`, homed in `context_ref` (`@owner/slug`), with
    /// `open_meta` carrying `status`/`verified` only when the corresponding argument is `Some` —
    /// mirrors `render::tests::row_with`, parameterized by title and context since status must
    /// match across multiple distinct rows.
    fn build_row(
        title: &str,
        context_ref: &str,
        status: Option<&str>,
        verified: Option<&str>,
        source_file: Option<&str>,
    ) -> ResourceDetail {
        let (owner, slug) = context_ref
            .split_once('/')
            .expect("context_ref must be @owner/slug");

        let mut open = serde_json::Map::new();
        if let Some(s) = status {
            open.insert("status".to_string(), json!(s));
        }
        if let Some(v) = verified {
            open.insert("verified".to_string(), json!(v));
        }
        if let Some(sf) = source_file {
            open.insert("source_file".to_string(), json!(sf));
        }

        let row = ResourceRow {
            id: ResourceId::from(Uuid::now_v7()),
            kb_context_id: None,
            origin_uri: String::new(),
            title: title.to_string(),
            originator_profile_id: ProfileId::from(Uuid::now_v7()),
            owner_profile_id: ProfileId::from(Uuid::now_v7()),
            is_active: true,
            created: DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"),
            updated: DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"),
            context_name: None,
            doc_type_name: "memory".to_string(),
            owner_handle: "someone".to_string(),
            context_slug: Some(slug.to_string()),
            context_owner_ref: Some(owner.to_string()),
            cogmap_id: None,
            cogmap_name: None,
            stage: None,
            seq: None,
            mode: None,
            effort: None,
            body_hash: None,
            ingest_state: Some(IngestState::Complete),
            body_storage: Some(BodyStorage::Derived),
        };

        ResourceDetail {
            row,
            managed_meta: None,
            open_meta: Some(serde_json::Value::Object(open)),
        }
    }

    fn meta_row_titled(title: &str, context_ref: &str) -> ResourceDetail {
        build_row(title, context_ref, Some("active"), Some("2026-08-01"), None)
    }

    fn meta_row_missing_status(title: &str, context_ref: &str) -> ResourceDetail {
        build_row(title, context_ref, None, Some("2026-08-01"), None)
    }

    /// A memory migrated from a local `.md` file — `title` is the human-readable hook, distinct
    /// from `source_file`, the filename it was migrated from. This is the real-world shape: see
    /// `status_matches_local_files_by_source_file_not_title` for why a title/filename comparison
    /// cannot stand in for this.
    fn meta_row_migrated(title: &str, context_ref: &str, source_file: &str) -> ResourceDetail {
        build_row(
            title,
            context_ref,
            Some("active"),
            Some("2026-08-01"),
            Some(source_file),
        )
    }

    #[test]
    fn status_works_when_not_opted_in() {
        let r = status_report(None, &[], &[local("feedback_x.md")]);
        assert!(!r.opted_in);
        assert_eq!(r.local_files, 1);
        assert_eq!(
            r.local_without_counterpart,
            vec!["feedback_x.md"],
            "an unadopted machine must still be told what it is carrying"
        );
    }

    /// The real-world shape: a migrated memory's title is a human-readable hook
    /// ("a clause cannot retire its own goal"), never the filename stem
    /// ("feedback_a_clause_cannot_retire_its_own_goal"). Matching must go through
    /// `open_meta.source_file` — a title/filename-stem comparison would report BOTH local files
    /// as orphaned here, not just the genuinely unmatched one.
    #[test]
    fn status_matches_local_files_by_source_file_not_title() {
        let rows = vec![meta_row_migrated(
            "a clause cannot retire its own goal",
            "@me/temper",
            "feedback_a_clause_cannot_retire_its_own_goal.md",
        )];
        let r = status_report(
            Some(&cfg()),
            &rows,
            &[
                local("feedback_a_clause_cannot_retire_its_own_goal.md"),
                local("feedback_y.md"),
            ],
        );
        assert_eq!(r.in_temper, 1);
        assert_eq!(
            r.local_without_counterpart,
            vec!["feedback_y.md"],
            "matching is by source_file; a migrated memory's title never equals its filename"
        );
    }

    /// A memory authored natively in Temper (from a session, from Desktop) never had a source
    /// file. That absence is ordinary — never a defect, and never grounds to treat some unrelated
    /// local file as matched or orphaned by accident.
    #[test]
    fn a_memory_without_a_source_file_is_ordinary_not_a_defect() {
        let native = meta_row_titled("a native memory", "@me/temper");
        let r = status_report(Some(&cfg()), &[native], &[local("feedback_z.md")]);
        assert_eq!(r.in_temper, 1);
        assert!(
            r.defects.is_empty(),
            "a memory with no source_file must not become a defect"
        );
        assert_eq!(
            r.local_without_counterpart,
            vec!["feedback_z.md"],
            "an unrelated local file stays orphaned; a source-file-less memory matches nothing"
        );
    }

    /// As [`build_row`], plus a raw `open_meta.reinforced`.
    fn meta_row_reinforced(
        title: &str,
        context_ref: &str,
        reinforced: serde_json::Value,
    ) -> ResourceDetail {
        let mut r = build_row(title, context_ref, Some("active"), Some("2026-08-01"), None);
        let Some(serde_json::Value::Object(open)) = r.open_meta.as_mut() else {
            unreachable!("build_row always builds an object")
        };
        open.insert("reinforced".to_string(), reinforced);
        r
    }

    /// **The instrument the threshold will eventually be chosen from.** It has to report the
    /// corpus as it actually is today — everything unreinforced — as a number rather than as
    /// silence, because "no data yet" and "the feature is not wired up" are indistinguishable
    /// otherwise.
    #[test]
    fn status_reports_the_reinforcement_distribution() {
        let rows = vec![
            meta_row_reinforced(
                "worked twice",
                "@me/temper",
                json!(["2026-05-14", "2026-07-02"]),
            ),
            meta_row_reinforced("worked once", "@me/temper", json!(["2026-08-01"])),
            meta_row_titled("never used", "@me/temper"),
        ];
        let r = status_report(Some(&cfg()), &rows, &[]);

        assert_eq!(r.reinforcement.reinforced, 2);
        assert_eq!(r.reinforcement.never_reinforced, 1);
        assert_eq!(
            r.reinforcement.last_reinforced,
            Some(chrono::NaiveDate::from_ymd_opt(2026, 8, 1).expect("date")),
            "last-reinforced is the max across the corpus, not the last row's max"
        );
        assert!(r.reinforcement.malformed.is_empty());
    }

    /// **The distribution and the tail count different populations, and that is pinned here so
    /// nobody has to rediscover it from a mismatch in the wild.** `fetch_context_rows` returns
    /// superseded memories; the render drops them before the threshold ever applies. So
    /// `never_reinforced` is over-counted relative to any tail line by exactly the superseded
    /// count — which is why the distribution agrees with `in_temper` and not with the index.
    ///
    /// This is a real observed gap: on the live corpus `never_reinforced` read 303 while the tail
    /// lines summed to 298.
    #[test]
    fn the_distribution_counts_superseded_memories_which_the_tail_never_collapses() {
        // The superseded memory is REINFORCED, deliberately. Two unreinforced rows cannot
        // discriminate here: skipping one still lands it in `never_reinforced`, so the assertion
        // would hold whether or not superseded rows were counted. Only a superseded row that must
        // appear in the `reinforced` bucket proves the population includes it.
        let mut retired = meta_row_reinforced("retired", "@me/temper", json!(["2026-07-01"]));
        let Some(serde_json::Value::Object(open)) = retired.open_meta.as_mut() else {
            unreachable!()
        };
        open.insert("status".to_string(), json!("superseded"));

        let rows = vec![
            meta_row_titled("live and unreinforced", "@me/temper"),
            retired,
        ];
        let r = status_report(Some(&cfg()), &rows, &[]);

        assert_eq!(
            r.reinforcement.reinforced, 1,
            "a superseded memory's reinforcement is counted, though the index never renders it"
        );
        assert_eq!(r.reinforcement.never_reinforced, 1);
        assert_eq!(
            r.reinforcement.last_reinforced,
            Some(chrono::NaiveDate::from_ymd_opt(2026, 7, 1).expect("date")),
            "and it can be the corpus's most recent reinforcement"
        );
        assert_eq!(
            r.reinforcement.reinforced + r.reinforcement.never_reinforced,
            r.in_temper,
            "the distribution's population is `in_temper`'s, which is the agreement that DOES hold"
        );
    }

    /// The acceptance criterion: a malformed record is **reported, not fatal**. `status` already
    /// treats hard defects this way; this asserts the softer class gets at least the same
    /// treatment, and lands in its own list rather than in `defects` — nothing anywhere refuses
    /// on it, so filing it beside the fatal ones would misreport what it costs.
    #[test]
    fn status_reports_a_malformed_reinforced_rather_than_failing() {
        let rows = vec![meta_row_reinforced(
            "typo'd",
            "@me/temper",
            json!(["last tuesday"]),
        )];
        let r = status_report(Some(&cfg()), &rows, &[]);

        assert_eq!(
            r.in_temper, 1,
            "the memory itself is fine and still counted"
        );
        assert!(
            r.defects.is_empty(),
            "a malformed `reinforced` is NOT a MemoryDefect — emit must still render"
        );
        assert_eq!(r.reinforcement.malformed.len(), 1);
        assert!(
            r.reinforcement.malformed[0].contains("typo'd")
                && r.reinforcement.malformed[0].contains("last tuesday"),
            "the report must name the memory and what it could not read: {:?}",
            r.reinforcement.malformed
        );
        assert_eq!(
            r.reinforcement.never_reinforced, 1,
            "and it counts as unreinforced, matching how the render will treat it"
        );
    }

    /// **A row can be wrong in both tiers, and reading the soft one inside the `Ok` arm would have
    /// dropped it for exactly those rows.** A memory whose open tier is malformed in one place is
    /// the memory most likely to be malformed in another, so the report that helps least is the
    /// one that goes quiet there.
    #[test]
    fn a_row_that_is_both_defective_and_malformed_contributes_both_reports() {
        let mut row = meta_row_reinforced("doubly wrong", "@me/temper", json!(["not a date"]));
        let Some(serde_json::Value::Object(open)) = row.open_meta.as_mut() else {
            unreachable!()
        };
        open.remove("status");

        let r = status_report(Some(&cfg()), &[row], &[]);

        assert_eq!(r.defects.len(), 1, "the hard defect is still reported");
        assert_eq!(
            r.reinforcement.malformed.len(),
            1,
            "and so is the soft one — neither report may swallow the other"
        );
    }

    #[test]
    fn status_reports_defects_without_failing() {
        let rows = vec![meta_row_missing_status("feedback_x", "@me/temper")];
        let r = status_report(Some(&cfg()), &rows, &[]);
        assert_eq!(
            r.defects.len(),
            1,
            "status REPORTS defects; only emit refuses on them"
        );
    }

    /// Write `body` to `dir/name` and return the directory, so a test reads like the directory
    /// it is describing. Mirrors `emit::tests`' `tempfile::TempDir` idiom.
    fn write(dir: &tempfile::TempDir, name: &str, body: &str) {
        std::fs::write(dir.path().join(name), body).expect("write fixture");
    }

    /// A memory file, as `harvest` leaves one: frontmatter first, `title:` present.
    const STAMPED: &str = "---\nname: project_x\ntitle: a curated title\n---\n\nbody\n";

    /// **The discriminator is frontmatter, not the filename.** Excluding only `index_path` was
    /// sufficient while temper owned the one index in the directory. It stops being sufficient the
    /// moment the index moves to a sibling name and the harness's own `MEMORY.md` — which carries
    /// no frontmatter — is left in place beside the memory files. Without this, that file is
    /// reported as an un-migrated memory forever: `status` lists it under
    /// `local_without_counterpart` and `emit` renders it under "Not yet migrated" with an
    /// instruction to `migrate` a file that is not a memory.
    #[test]
    fn a_file_without_frontmatter_is_not_a_local_memory_file() {
        let dir = tempfile::TempDir::new().unwrap();
        write(&dir, "MANAGED_MEMORY.md", "<!-- GENERATED -->\n\n# Index\n");
        write(
            &dir,
            "MEMORY.md",
            "# Memory index\n\n- a harness-written pointer\n",
        );
        write(&dir, "project_x.md", STAMPED);

        let index = dir.path().join("MANAGED_MEMORY.md");
        let found = read_local_files(index.to_str().unwrap());

        assert_eq!(
            found
                .iter()
                .map(|f| f.filename.as_str())
                .collect::<Vec<_>>(),
            vec!["project_x.md"],
            "only the frontmatter-bearing file is a memory; neither index is"
        );
    }

    /// The over-filter this must not become. `harvest` stamps `title:` into files that lack one,
    /// and a file it could not title is exactly the file a reader needs `status` to name — the
    /// live instance is `project_temper_services_extraction_idea.md`, which `migrate` skips *and*
    /// `status` still reports. Filtering on `title:` rather than on frontmatter would hide it, so
    /// the presence of a frontmatter block is the whole test.
    #[test]
    fn a_frontmatter_file_with_no_title_is_still_a_local_memory_file() {
        let dir = tempfile::TempDir::new().unwrap();
        write(&dir, "MEMORY.md", "# not a memory\n");
        write(&dir, "untitled.md", "---\nname: untitled\n---\n\nbody\n");

        let index = dir.path().join("MEMORY.md");
        let found = read_local_files(index.to_str().unwrap());

        assert_eq!(
            found
                .iter()
                .map(|f| f.filename.as_str())
                .collect::<Vec<_>>(),
            vec!["untitled.md"],
            "a titleless memory must stay visible — it is the one a reader must be told about"
        );
    }

    /// The incumbent behaviour, pinned so the frontmatter rule cannot quietly replace it: the file
    /// named by `index_path` is excluded even when it *does* carry frontmatter.
    #[test]
    fn the_configured_index_is_excluded_even_if_it_has_frontmatter() {
        let dir = tempfile::TempDir::new().unwrap();
        write(&dir, "MANAGED_MEMORY.md", STAMPED);
        write(&dir, "project_x.md", STAMPED);

        let index = dir.path().join("MANAGED_MEMORY.md");
        let found = read_local_files(index.to_str().unwrap());

        assert_eq!(
            found
                .iter()
                .map(|f| f.filename.as_str())
                .collect::<Vec<_>>(),
            vec!["project_x.md"],
            "index_path exclusion is independent of the frontmatter rule"
        );
    }
}
