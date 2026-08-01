//! `temper memory harvest` — move each curated title out of the hand-written index and into the
//! memory file it names.
//!
//! **Why this command exists at all.** A memory's human-readable title lives in exactly one place:
//! the link text in the hand-written `MEMORY.md`. Measured on this machine's corpus on 2026-08-01,
//! deriving it from the filename instead destroys the hook on **51 of 110** un-migrated files —
//! the filename names the *instance* (`project_cargo_make_requires_dotenv.md`) where the curated
//! title names the *lesson* ("EVERY task fails on a fresh clone") — and the files' own `name:` key
//! is the bare filename stem on **91 of 114**. So when `emit` takes the index over, the write
//! destroys the only copy of those titles, and with them `migrate`'s ability to move the tail at
//! all: every remaining file would skip with [`SkipReason::NoHarvestedTitle`].
//!
//! This command is the one-shot that runs first. It reads the index, and writes each curated title
//! into the frontmatter of the file it names — after which the title survives the takeover, the
//! union render reads it from the file rather than from the index it is about to overwrite, and
//! the drift gate stays able to see a hand edit (see [`mod@super::check`]).
//!
//! **Pinning `modified` is not a bonus; it is what makes the write safe.** `migrate` dates a
//! memory `metadata.modified`, falling back to the file's mtime — and **84 of the 114** files
//! carry no `modified`. Stamping a title touches the file, which advances its mtime, which would
//! silently re-date every one of those claims to today: a claim reading as freshly checked when
//! nobody checked it. So the same edit pins `modified` to the mtime read *before* the write,
//! turning a hazard into a repair.
//!
//! Every decision is a pure function over its inputs ([`plan_harvest`], [`apply_harvest`]), so the
//! judgment is testable with no filesystem, no server and no clock.
//!
//! [`SkipReason::NoHarvestedTitle`]: super::migrate::SkipReason::NoHarvestedTitle

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use serde::Serialize;

use temper_core::types::config::TemperConfig;

use super::fetch::fetch_context_rows;
use super::migrate::{
    harvest_titles, resolve_run_mode, scan_memory_dir, FileDefect, RunMode, ScannedFile,
};
use crate::actions::runtime::build_config_store_and_client;
use crate::error::{Result, TemperError};

/// One file this run would stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarvestAction {
    pub filename: String,
    /// The curated title, exactly as the index's link text carries it.
    pub title: String,
    /// The date to pin into `metadata.modified`, or `None` when the file already carries one.
    /// Always the mtime read **before** this run's write — never today.
    pub pin_modified: Option<NaiveDate>,
}

/// Why a scanned file is not stamped. Every skip is reported: a file that silently vanishes from
/// a harvest is indistinguishable from one that was stamped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarvestSkip {
    /// The file already carries a root `title:`. This is what makes re-running safe, and it is
    /// checked before anything else so a second run is a pure no-op rather than a rewrite.
    AlreadyTitled,
    /// No `MEMORY.md` link names this file, so it has no curated title. A title is never invented
    /// from the filename — that is the whole finding this command exists because of.
    NoHarvestedTitle,
    /// The file is already a Temper memory; its title lives in the store, which is authoritative.
    AlreadyMigrated,
    /// The file could not be parsed.
    Unparseable(FileDefect),
}

impl std::fmt::Display for HarvestSkip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyTitled => write!(f, "already carries a `title:`"),
            Self::NoHarvestedTitle => {
                write!(f, "no MEMORY.md link names it, so it has no curated title")
            }
            Self::AlreadyMigrated => write!(f, "already in Temper (matched by source_file)"),
            Self::Unparseable(d) => write!(f, "unparseable: {d}"),
        }
    }
}

/// What a run would do, decided purely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarvestPlan {
    pub actions: Vec<HarvestAction>,
    pub skipped: Vec<(String, HarvestSkip)>,
}

/// Decide the batch. Pure over its three inputs: the scanned files, the titles harvested from the
/// index, and the `source_file` values already present in Temper.
///
/// The skip order is deliberate. `AlreadyTitled` is tested **first**, so a second run is a pure
/// no-op rather than a rewrite — which matters more here than anywhere else, because a rewrite
/// advances the mtime that [`HarvestAction::pin_modified`] exists to preserve.
pub fn plan_harvest(
    scanned: &[ScannedFile],
    titles: &HashMap<String, String>,
    already_migrated: &HashSet<String>,
) -> HarvestPlan {
    let mut ordered: Vec<&ScannedFile> = scanned.iter().collect();
    ordered.sort_by(|a, b| a.filename.cmp(&b.filename));

    let mut actions = Vec::new();
    let mut skipped = Vec::new();

    for file in ordered {
        let parsed = match super::migrate::parse_memory_file(&file.content) {
            Ok(p) => p,
            Err(defect) => {
                skipped.push((file.filename.clone(), HarvestSkip::Unparseable(defect)));
                continue;
            }
        };
        if parsed.title.is_some() {
            skipped.push((file.filename.clone(), HarvestSkip::AlreadyTitled));
            continue;
        }
        if already_migrated.contains(&file.filename) {
            skipped.push((file.filename.clone(), HarvestSkip::AlreadyMigrated));
            continue;
        }
        let Some(title) = titles.get(&file.filename) else {
            skipped.push((file.filename.clone(), HarvestSkip::NoHarvestedTitle));
            continue;
        };

        actions.push(HarvestAction {
            filename: file.filename.clone(),
            title: title.clone(),
            // Fill only the absence, never overwrite. `modified` is NOT a hand-written date —
            // measured 2026-08-01, the Claude Code memory subsystem stamps it on any file it
            // writes — so this is not "a stated fact beats an inferred one". It is that
            // `modified` survives a copy, a `touch` and a restore-from-backup where an mtime does
            // not, so the durable record of when the file was last written wins over the fragile
            // one. Neither is evidence anybody re-checked the claim; see the module doc.
            //
            // The same measurement sharpens a known hazard: because the harness re-stamps
            // `modified` in the file's own bytes, a formatting-only edit advances the date
            // `migrate` will use as `verified` even on a file that carries one — so the exposure
            // is not limited to the mtime-fallback files.
            pin_modified: match parsed.modified {
                Some(_) => None,
                None => Some(file.mtime),
            },
        });
    }

    HarvestPlan { actions, skipped }
}

/// Emit `key: value` as YAML, letting the emitter decide the quoting.
///
/// Serializing the **mapping** rather than the bare scalar is what makes this safe for prose: the
/// curated titles carry colons, quotes, backticks and leading `#`/`-`, and if the emitter decides
/// to fold a long value across lines it is the mapping form that indents the continuation
/// correctly. Hand-writing `format!("title: {t}")` and picking quotes by inspection would be a
/// second, worse YAML emitter living next to the real one.
fn yaml_entry(key: &str, value: &str) -> String {
    let mut m = serde_yaml::Mapping::new();
    m.insert(key.into(), value.into());
    serde_yaml::to_string(&m).expect("a string→string mapping always serializes")
}

/// Split `content` into (frontmatter body, everything from the closing fence onward).
fn split_fence(content: &str) -> std::result::Result<(&str, &str), FileDefect> {
    const FENCE: &str = "---\n";
    let rest = content.strip_prefix(FENCE).ok_or(FileDefect::NoFence)?;
    let end = rest.find("\n---\n").ok_or(FileDefect::NoFence)?;
    Ok((&rest[..end], &rest[end..]))
}

/// Rewrite one file's frontmatter, inserting `title:` and — when `pin_modified` is `Some` —
/// `metadata.modified`. Everything else in the file is preserved byte-for-byte.
///
/// **Textual insertion, never a YAML round-trip.** Parsing the frontmatter and re-emitting it
/// would reorder keys, requote every value and normalize whitespace across all 114 files, turning
/// a two-line addition into a diff nobody can review — and the body is the memory itself, so a
/// re-serialization is a rewrite of the claim.
pub fn apply_harvest(
    content: &str,
    title: &str,
    pin_modified: Option<NaiveDate>,
) -> std::result::Result<String, FileDefect> {
    let (fm, after) = split_fence(content)?;

    // First line of the frontmatter: independent of every other key's shape, so the insertion
    // cannot be broken by a multi-line `description:` or a missing `name:`.
    let mut new_fm = format!("{}{fm}", yaml_entry("title", title));

    if let Some(date) = pin_modified {
        let entry = format!("modified: {}", date.format("%Y-%m-%d"));
        new_fm = match find_metadata_line(&new_fm) {
            // The nested shape (104 of 114): insert as the block's first child, matching whatever
            // indent its existing children use.
            Some((line_end, indent)) => {
                format!(
                    "{}\n{indent}{entry}{}",
                    &new_fm[..line_end],
                    &new_fm[line_end..]
                )
            }
            // The flat shape (10 of 114) has no `metadata:` block, and `parse_memory_file` reads
            // `modified` only from under one — so pinning has to create it. Additive: the root
            // `type:` these files carry is still found beside it.
            None => format!("{new_fm}\nmetadata:\n  {entry}"),
        };
    }

    Ok(format!("---\n{new_fm}{after}"))
}

/// Locate the root-level `metadata:` block header, returning the byte offset of its line's end
/// and the indent its children use.
///
/// Matches only a line that starts at column 0 and carries nothing after the colon — a `metadata:`
/// appearing as a nested key or with an inline value is not the block being extended. The indent
/// is read off the existing first child rather than assumed, so a file indenting with four spaces
/// is extended in its own style instead of being given a mixed block.
fn find_metadata_line(fm: &str) -> Option<(usize, String)> {
    let mut offset = 0usize;
    let mut lines = fm.split('\n');
    while let Some(line) = lines.next() {
        let line_end = offset + line.len();
        if line
            .strip_prefix("metadata:")
            .is_some_and(|r| r.trim().is_empty())
        {
            let indent = lines
                .next()
                .map(|next| next.chars().take_while(|c| *c == ' ').collect::<String>())
                .filter(|i| !i.is_empty())
                .unwrap_or_else(|| "  ".to_string());
            return Some((line_end, indent));
        }
        offset = line_end + 1;
    }
    None
}

/// One file this run stamped, or would have.
#[derive(Debug, Clone, Serialize)]
pub struct StampedReport {
    pub source_file: String,
    pub title: String,
    /// The date pinned into `metadata.modified`, or `null` when the file already stated one.
    pub pinned_modified: Option<String>,
    /// `false` on a dry run, and on a file whose write failed — the write error is reported
    /// beside it rather than ending the run.
    pub written: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedReport {
    pub source_file: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HarvestReport {
    pub dry_run: bool,
    pub scanned: usize,
    pub titles_harvested: usize,
    pub stamped: Vec<StampedReport>,
    pub skipped: Vec<SkippedReport>,
}

/// The I/O shell: scan, harvest, plan, write.
///
/// Runs under the same [`RunMode`] gate as `migrate`, for the same reason rather than for
/// symmetry's sake: this rewrites files, and a write run with no human attached and no explicit
/// authorization is exactly the case that should not proceed.
pub fn harvest(global: &TemperConfig, dry_run: bool, unattended: bool) -> Result<HarvestReport> {
    use std::io::IsTerminal;

    let mem = global.memory.as_ref().ok_or_else(|| {
        TemperError::Config(
            "no [memory] section in config.toml — the memory projection is off; see \
             `temper memory status`"
                .to_string(),
        )
    })?;

    let mode = resolve_run_mode(dry_run, unattended, std::io::stdin().is_terminal())
        .map_err(TemperError::BadRequest)?;

    let index_path = super::resolve_index_path(mem, None);
    let dir = index_path
        .parent()
        .ok_or_else(|| TemperError::Config(format!("{} has no parent", index_path.display())))?
        .to_path_buf();
    let scanned = scan_memory_dir(&index_path);
    let titles = harvest_titles(&std::fs::read_to_string(&index_path).unwrap_or_default());

    // A file already in Temper has its title in the store, which is authoritative — stamping a
    // second copy into the local file would create two that nothing keeps in step.
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
    let (_cfg, _store, client) = build_config_store_and_client()?;
    let mut already_migrated: HashSet<String> = HashSet::new();
    for ctx in mem.all_contexts() {
        for row in runtime.block_on(fetch_context_rows(&client, ctx))? {
            if let Some(sf) = super::status::source_file_of(&row) {
                already_migrated.insert(sf);
            }
        }
    }
    drop(runtime);

    let plan = plan_harvest(&scanned, &titles, &already_migrated);

    let mut stamped = Vec::with_capacity(plan.actions.len());
    for action in &plan.actions {
        let (written, error) = match mode {
            RunMode::DryRun => (false, None),
            RunMode::Interactive | RunMode::Unattended => {
                match stamp_file(&dir.join(&action.filename), action) {
                    Ok(()) => (true, None),
                    // One unwritable file must not strand the rest, exactly as one unparseable
                    // file does not strand the scan.
                    Err(e) => (false, Some(e.to_string())),
                }
            }
        };
        stamped.push(StampedReport {
            source_file: action.filename.clone(),
            title: action.title.clone(),
            pinned_modified: action
                .pin_modified
                .map(|d| d.format("%Y-%m-%d").to_string()),
            written,
            error,
        });
    }

    Ok(HarvestReport {
        dry_run: matches!(mode, RunMode::DryRun),
        scanned: scanned.len(),
        titles_harvested: titles.len(),
        stamped,
        skipped: plan
            .skipped
            .iter()
            .map(|(source_file, reason)| SkippedReport {
                source_file: source_file.clone(),
                reason: reason.to_string(),
            })
            .collect(),
    })
}

/// Re-read, transform, write. The file is re-read here rather than reusing the content the scan
/// held, so a file edited between the scan and the write is stamped as it now is — or fails to
/// parse and is reported, instead of being silently reverted to the scanned copy.
fn stamp_file(path: &std::path::Path, action: &HarvestAction) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let updated = apply_harvest(&content, &action.title, action.pin_modified)
        .map_err(|d| TemperError::BadRequest(format!("{}: {d}", path.display())))?;
    std::fs::write(path, updated)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("test date")
    }

    /// The dominant shape: 104 of the 114 un-migrated files nest `type` under `metadata:`.
    const NESTED: &str = "\
---
name: project_ci_flake_signatures
description: CI reds that are genuine flakes
metadata:
  node_type: memory
  type: project
---

# Body stays untouched.
";

    /// The older shape: 10 of the 114 carry `type` at the root and have no `metadata:` block at
    /// all, so pinning `modified` has to create one.
    const FLAT: &str = "\
---
name: project_hybrid_execution_skill_idea
description: The hybrid-execution skill is installed
type: project
originSessionId: b38b9798-e75a-4fd8-aae0-b8726e4db2aa
---

# Body.
";

    fn scanned(filename: &str, content: &str, mtime: &str) -> ScannedFile {
        ScannedFile {
            filename: filename.to_string(),
            content: content.to_string(),
            mtime: d(mtime),
        }
    }

    fn titles(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    // ---- apply_harvest: the write itself -------------------------------------------------

    #[test]
    fn the_title_is_inserted_and_reads_back_identically() {
        let out = apply_harvest(NESTED, "genuine flake signatures", None).expect("must apply");
        let parsed = super::super::migrate::parse_memory_file(&out).expect("must still parse");
        assert_eq!(
            parsed.title.as_deref(),
            Some("genuine flake signatures"),
            "the stamped title must survive a round-trip through the YAML parser"
        );
    }

    /// The curated titles are prose, not identifiers: they carry colons, quotes, backticks,
    /// em-dashes, commas and parentheses, every one of which is YAML-significant. Asserting a
    /// hand-picked format string would test the emitter against itself; this asserts the only
    /// property that matters — what goes in comes back out — across the shapes that actually
    /// appear in this machine's index.
    #[test]
    fn every_yaml_hostile_title_survives_the_round_trip() {
        let hostile = [
            "`list --status` and `--tags` are FIXED — re-probe before trusting a \"known bug\"",
            "STOP building instrument — Pete, 2026-07-27",
            "uuid_generate_v7(), not native",
            "a header announcing a gap ≠ an open gap",
            "the rwx capability synthesis — don't re-derive",
            "bge-base-en-v1.5 / 768",
            "FIELD GUIDE: read research 019f6280 first",
            "#not a comment",
            "- not a list item",
            "yes",
            "null",
            "123",
            "  leading and trailing spaces  ",
            "a { brace } and a [ bracket ]",
            "an @at and a *star and an &amp",
        ];
        for title in hostile {
            let out = apply_harvest(NESTED, title, None)
                .unwrap_or_else(|e| panic!("must apply for {title:?}: {e}"));
            let parsed = super::super::migrate::parse_memory_file(&out)
                .unwrap_or_else(|e| panic!("must still parse for {title:?}: {e}"));
            assert_eq!(
                parsed.title.as_deref(),
                Some(title),
                "title must round-trip byte-for-byte"
            );
        }
    }

    /// The body is the memory. A frontmatter stamp that reflows or re-emits it has rewritten the
    /// claim it was supposed to be preserving.
    #[test]
    fn the_body_is_preserved_byte_for_byte() {
        let out = apply_harvest(NESTED, "a title", Some(d("2026-07-14"))).expect("must apply");
        assert!(
            out.ends_with("\n# Body stays untouched.\n"),
            "the body must survive untouched: {out}"
        );
    }

    /// Every key the file already carried must still be there — a stamp is an insertion, never a
    /// re-serialization of the frontmatter through a YAML emitter (which would reorder keys, drop
    /// the trailing space after `metadata:`, and requote every value).
    #[test]
    fn existing_frontmatter_keys_survive_untouched() {
        let out = apply_harvest(NESTED, "a title", None).expect("must apply");
        for line in [
            "name: project_ci_flake_signatures",
            "description: CI reds that are genuine flakes",
            "  node_type: memory",
            "  type: project",
        ] {
            assert!(out.contains(line), "must preserve {line:?}, got:\n{out}");
        }
    }

    /// 84 of the 114 files carry no `metadata.modified`, so `migrate` dates them from their mtime
    /// — and stamping a title advances that mtime. Pinning the pre-write mtime is what stops this
    /// command from re-dating 84 claims to today, which is the goal's
    /// `a-claim-must-never-read-as-checked-when-nobody-checked-it` in its most literal form.
    #[test]
    fn pinning_modified_preserves_the_date_migrate_would_have_used() {
        let out = apply_harvest(NESTED, "a title", Some(d("2026-07-14"))).expect("must apply");
        let parsed = super::super::migrate::parse_memory_file(&out).expect("must parse");
        assert_eq!(
            parsed.modified,
            Some(d("2026-07-14")),
            "the pre-write mtime must be pinned, so a later mtime bump cannot re-date the claim"
        );
    }

    /// The flat shape has no `metadata:` block, and `parse_memory_file` reads `modified` only from
    /// under one — so pinning has to create it. Without this the 10 flat files are exactly the
    /// ones this command would silently re-date.
    #[test]
    fn pinning_modified_creates_the_metadata_block_on_a_flat_file() {
        let out = apply_harvest(FLAT, "a flat title", Some(d("2026-06-02"))).expect("must apply");
        let parsed = super::super::migrate::parse_memory_file(&out).expect("must parse");
        assert_eq!(parsed.modified, Some(d("2026-06-02")));
        assert_eq!(
            parsed.cohort, "project",
            "the root `type:` must still be found after a metadata block is added beside it"
        );
        assert_eq!(parsed.title.as_deref(), Some("a flat title"));
    }

    #[test]
    fn a_file_with_no_frontmatter_fence_is_a_defect_not_a_silent_pass() {
        let err = apply_harvest("# just a body\n", "t", None).expect_err("must refuse");
        assert_eq!(err, FileDefect::NoFence);
    }

    // ---- plan_harvest: the judgment ------------------------------------------------------

    #[test]
    fn a_file_with_a_curated_title_is_stamped_with_its_pre_write_mtime() {
        let plan = plan_harvest(
            &[scanned(
                "project_ci_flake_signatures.md",
                NESTED,
                "2026-07-14",
            )],
            &titles(&[("project_ci_flake_signatures.md", "genuine flake signatures")]),
            &HashSet::new(),
        );
        assert_eq!(
            plan.actions,
            vec![HarvestAction {
                filename: "project_ci_flake_signatures.md".to_string(),
                title: "genuine flake signatures".to_string(),
                pin_modified: Some(d("2026-07-14")),
            }]
        );
    }

    /// A file that already declares `metadata.modified` needs no pin — and must not be given one,
    /// since overwriting a hand-written date with an mtime would replace a stated fact with an
    /// inferred one.
    #[test]
    fn a_file_that_already_declares_modified_is_not_re_pinned() {
        let with_modified = "\
---
name: x
description: d
metadata:
  type: project
  modified: 2026-05-01T10:00:00Z
---
body
";
        let plan = plan_harvest(
            &[scanned("x.md", with_modified, "2026-07-14")],
            &titles(&[("x.md", "a title")]),
            &HashSet::new(),
        );
        assert_eq!(plan.actions.len(), 1);
        assert_eq!(
            plan.actions[0].pin_modified, None,
            "a stated `modified` must never be overwritten by an inferred mtime"
        );
    }

    /// Re-running must be a no-op, not a rewrite — otherwise every run bumps every mtime, which is
    /// the exact hazard `pin_modified` exists to close.
    #[test]
    fn a_file_already_carrying_a_title_is_skipped() {
        let stamped = apply_harvest(NESTED, "already here", None).expect("must apply");
        let plan = plan_harvest(
            &[scanned("x.md", &stamped, "2026-07-14")],
            &titles(&[("x.md", "a different title")]),
            &HashSet::new(),
        );
        assert!(plan.actions.is_empty(), "a second run must change nothing");
        assert_eq!(
            plan.skipped,
            vec![("x.md".to_string(), HarvestSkip::AlreadyTitled)]
        );
    }

    /// The store is authoritative for a migrated memory's title. Stamping the local file would
    /// mint a second copy that nothing keeps in step with the first.
    #[test]
    fn a_file_already_in_temper_is_skipped() {
        let already: HashSet<String> = ["x.md".to_string()].into_iter().collect();
        let plan = plan_harvest(
            &[scanned("x.md", NESTED, "2026-07-14")],
            &titles(&[("x.md", "a title")]),
            &already,
        );
        assert!(plan.actions.is_empty());
        assert_eq!(
            plan.skipped,
            vec![("x.md".to_string(), HarvestSkip::AlreadyMigrated)]
        );
    }

    /// The 4 files no index link names. They are not given a title derived from their filename —
    /// that derivation is precisely what this command exists to avoid.
    #[test]
    fn a_file_no_link_names_is_skipped_never_given_a_derived_title() {
        let plan = plan_harvest(
            &[scanned("project_orphan.md", NESTED, "2026-07-14")],
            &titles(&[]),
            &HashSet::new(),
        );
        assert!(plan.actions.is_empty());
        assert_eq!(
            plan.skipped,
            vec![(
                "project_orphan.md".to_string(),
                HarvestSkip::NoHarvestedTitle
            )]
        );
    }

    #[test]
    fn an_unparseable_file_is_reported_not_fatal() {
        let plan = plan_harvest(
            &[
                scanned("broken.md", "# no fence\n", "2026-07-14"),
                scanned("good.md", NESTED, "2026-07-14"),
            ],
            &titles(&[("broken.md", "t1"), ("good.md", "t2")]),
            &HashSet::new(),
        );
        assert_eq!(
            plan.actions.len(),
            1,
            "one unparseable file must not strand the others"
        );
        assert_eq!(plan.actions[0].filename, "good.md");
        assert!(matches!(
            plan.skipped.as_slice(),
            [(f, HarvestSkip::Unparseable(_))] if f == "broken.md"
        ));
    }
}
