//! `temper memory migrate` — move local memory files into Temper, reconciling rather than
//! blind-creating.
//!
//! Every pure decision this command makes is a free function over its inputs, so the judgment is
//! testable without a server, a filesystem, or a clock. The I/O shell only fetches, prompts, and
//! writes.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use serde::Serialize;

use temper_core::types::config::{MemoryConfig, TemperConfig};

use super::fetch::{fetch_context_rows, MEMORY_DOC_TYPE};
use crate::actions::runtime::build_config_store_and_client;
use crate::error::{Result, TemperError};
use crate::format::OutputFormat;
use crate::output;

/// One local memory file, parsed into the parts a Temper memory needs.
///
/// `name` is deliberately absent: the frontmatter key exists on every file but means three
/// different things across the corpus (filename stem, kebab slug, or a human sentence), so nothing
/// downstream may depend on it. Titles come from [`harvest_titles`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMemoryFile {
    /// The curated human-readable title, from a root `title:` key — present only on files
    /// [`super::harvest`] has stamped. `None` on an un-harvested file, whose title still lives
    /// only in the index's link text.
    pub title: Option<String>,
    /// `open_meta.descriptor` — the recall-relevance line, from `description:`.
    pub descriptor: String,
    /// Which cohort this file belongs to (`feedback` / `project` / `reference`), from the
    /// frontmatter `type`, which sits at the root on the older flat shape and under `metadata:`
    /// on the newer one.
    pub cohort: String,
    /// `metadata.modified`, date part only. `None` on every flat-shape file and on most of the
    /// nested ones — the caller supplies the fallback, since a file's mtime is I/O.
    pub modified: Option<NaiveDate>,
    /// Everything after the closing fence, byte-for-byte.
    pub body: String,
}

/// Why a local file cannot become a memory. Reported per file and skipped — never fatal to the
/// run, because one unparseable file must not strand the other 68.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDefect {
    /// No opening `---`, or no closing one.
    NoFence,
    /// The fence is well-formed but the YAML inside it will not parse. Distinct from [`NoFence`]
    /// because the fixes are different and a reader told "no frontmatter" about a file that
    /// visibly has some goes looking for the wrong thing. The live instance is an unquoted
    /// `description:` containing `tests run: N passed` — a colon-space inside a plain scalar.
    ///
    /// [`NoFence`]: FileDefect::NoFence
    InvalidYaml(String),
    /// A key the contract needs is missing or is not a string.
    MissingKey(&'static str),
}

impl std::fmt::Display for FileDefect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoFence => write!(f, "no `---` frontmatter block"),
            Self::InvalidYaml(e) => write!(
                f,
                "the `---` fence is well-formed but its YAML will not parse ({e})"
            ),
            Self::MissingKey(k) => write!(f, "frontmatter has no string `{k}`"),
        }
    }
}

/// Parse one local memory file. Pure over its input — the filename and the mtime fallback are the
/// caller's to supply.
pub fn parse_memory_file(content: &str) -> std::result::Result<ParsedMemoryFile, FileDefect> {
    let (yaml_text, body) = temper_workflow::frontmatter::parse::split_frontmatter_block(content)
        .map_err(|_| FileDefect::NoFence)?;
    let fm: serde_yaml::Value =
        serde_yaml::from_str(&yaml_text).map_err(|e| FileDefect::InvalidYaml(e.to_string()))?;

    // Optional by construction: a file only carries `title:` once `harvest` has stamped it, and
    // an un-stamped file is ordinary rather than defective.
    let title = fm
        .get("title")
        .and_then(serde_yaml::Value::as_str)
        .map(str::to_string);

    let descriptor = fm
        .get("description")
        .and_then(serde_yaml::Value::as_str)
        .ok_or(FileDefect::MissingKey("description"))?
        .to_string();

    // `type` sits at the root on the flat shape and under `metadata:` on the nested one. Both are
    // live on disk, so both are read here — checking only one silently drops a whole shape.
    let metadata = fm.get("metadata");
    let cohort = fm
        .get("type")
        .or_else(|| metadata.and_then(|m| m.get("type")))
        .and_then(serde_yaml::Value::as_str)
        .ok_or(FileDefect::MissingKey("type"))?
        .to_string();

    let modified = metadata
        .and_then(|m| m.get("modified"))
        .and_then(serde_yaml::Value::as_str)
        .and_then(|raw| NaiveDate::parse_from_str(&raw[..raw.len().min(10)], "%Y-%m-%d").ok());

    Ok(ParsedMemoryFile {
        title,
        descriptor,
        cohort,
        modified,
        body,
    })
}

/// What `migrate` was asked to do.
#[derive(Debug, Clone)]
pub struct MigrateArgs<'a> {
    /// The frontmatter `type` to migrate — `feedback` first, by the design's sequencing.
    pub cohort: &'a str,
    /// Target context ref. `None` resolves through [`default_context_for_cohort`].
    pub context: Option<&'a str>,
    pub dry_run: bool,
    pub unattended: bool,
    pub format: OutputFormat,
}

/// What happened to one proposal.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum ProposalOutcome {
    /// Dry run — this is what a real run would write.
    Planned,
    /// Written. Carries no id: `resource::create` prints the created resource and returns `()`,
    /// and inventing a placeholder id here would put a value in the report that names nothing.
    Created,
    /// A human was shown the batch and said no, so nothing was written. The decision is over the
    /// whole batch — every proposal in the run reports this, never a subset.
    Declined,
}

/// One proposal as reported.
#[derive(Debug, Clone, Serialize)]
pub struct ProposalReport {
    pub source_file: String,
    pub title: String,
    pub descriptor: String,
    pub verified: String,
    pub body_bytes: usize,
    #[serde(flatten)]
    pub outcome: ProposalOutcome,
}

/// The whole run, as a reviewable object. A dry run's report is exactly what a real run would do,
/// which is what makes reviewing one worth anything.
#[derive(Debug, Clone, Serialize)]
pub struct MigrationReport {
    pub cohort: String,
    pub context: String,
    pub dry_run: bool,
    /// Every `.md` file found beside the index, in or out of cohort.
    pub scanned: usize,
    /// How many of them a `MEMORY.md` link named. A sharp drop here means the index has been
    /// taken over by `emit` and no longer carries curated titles — see the module doc.
    pub titles_harvested: usize,
    pub proposals: Vec<ProposalReport>,
    pub skipped: Vec<SkippedReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedReport {
    pub source_file: String,
    pub reason: String,
}

/// Run the migration.
///
/// Reads are batched ahead of every write: the whole plan is resolved before the first resource is
/// created. That is deliberate on two counts. The already-migrated set is fetched once for the
/// run, rather than re-read per file against a store this run is itself changing. And the batch
/// confirmation can only name a count that is complete — interleaving would ask the human to
/// authorize a batch whose size was still being discovered.
pub fn migrate(
    global: &TemperConfig,
    config: &crate::config::Config,
    args: MigrateArgs<'_>,
) -> Result<MigrationReport> {
    use std::io::IsTerminal;

    let mem = global.memory.as_ref().ok_or_else(|| {
        TemperError::Config(
            "no [memory] section in config.toml — the memory projection is off; see \
             `temper memory status`"
                .to_string(),
        )
    })?;

    let mode = resolve_run_mode(
        args.dry_run,
        args.unattended,
        std::io::stdin().is_terminal(),
    )
    .map_err(TemperError::BadRequest)?;

    let target_context = match args.context {
        Some(ctx) => ctx.to_string(),
        None => default_context_for_cohort(mem, args.cohort)
            .ok_or_else(|| {
                TemperError::Config(format!(
                    "no default context for cohort `{}`: the matching list in [memory] is empty. \
                     Pass --context <ref>, or add one to {} in config.toml",
                    args.cohort,
                    if args.cohort == CROSS_PROJECT_COHORT {
                        "shared_contexts"
                    } else {
                        "project_contexts"
                    }
                ))
            })?
            .to_string(),
    };

    let index_path = super::resolve_index_path(mem, None);
    let scanned = scan_memory_dir(&index_path);
    let titles = harvest_titles(&std::fs::read_to_string(&index_path).unwrap_or_default());

    // Read phase — one runtime, everything the plan needs, before any write.
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
    let (_cfg, _store, client) = build_config_store_and_client()?;

    let mut already_migrated: HashSet<String> = HashSet::new();
    for ctx in mem.all_contexts() {
        for row in runtime.block_on(fetch_context_rows(&client, ctx))? {
            if let Some(sf) = row
                .open_meta
                .as_ref()
                .and_then(|om| om.get("source_file"))
                .and_then(serde_json::Value::as_str)
            {
                already_migrated.insert(sf.to_string());
            }
        }
    }
    drop(runtime);

    let plan = plan_migration(&scanned, &titles, args.cohort, &already_migrated);

    // One question for the whole run, asked after the plan is complete and before anything is
    // written. Declining is a decision about the batch, so it lands on every proposal in it.
    let declined = batch_confirmation_required(mode, plan.proposals.len())
        && !confirm_batch(plan.proposals.len(), &target_context, args.cohort)?;

    // Write phase. `resource::create` builds its own runtime, so the read runtime is dropped
    // above rather than held across it.
    let mut proposals = Vec::with_capacity(plan.proposals.len());
    for proposal in &plan.proposals {
        let outcome = match (mode, declined) {
            (RunMode::DryRun, _) => ProposalOutcome::Planned,
            (_, true) => ProposalOutcome::Declined,
            (_, false) => {
                write_proposal(config, &target_context, proposal, args.format)?;
                ProposalOutcome::Created
            }
        };

        proposals.push(ProposalReport {
            source_file: proposal.source_file.clone(),
            title: proposal.title.clone(),
            descriptor: proposal.descriptor.clone(),
            verified: proposal.verified.format("%Y-%m-%d").to_string(),
            body_bytes: proposal.body.len(),
            outcome,
        });
    }

    Ok(MigrationReport {
        cohort: args.cohort.to_string(),
        context: target_context,
        dry_run: matches!(mode, RunMode::DryRun),
        scanned: scanned.len(),
        titles_harvested: titles.len(),
        proposals,
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

/// Whether this run must ask a human before it writes.
///
/// Pure, so the judgment is testable without a terminal. `DryRun` writes nothing, so there is
/// nothing to confirm; `--unattended` **is** the authorization, and asking a second time would
/// strand every agent and hook run at a prompt nothing can answer.
///
/// An empty batch never asks. Re-running `migrate` once everything has migrated is the ordinary
/// case, and a confirmation with nothing behind it is exactly the prompt-nobody-reads failure that
/// retired the per-file gate this replaced.
pub fn batch_confirmation_required(mode: RunMode, proposal_count: usize) -> bool {
    match mode {
        RunMode::DryRun | RunMode::Unattended => false,
        RunMode::Interactive => proposal_count > 0,
    }
}

/// Show what the run would write and ask, once. The default is **no** — the expensive mistake is
/// the write, not the delay.
///
/// The near-duplicate caveat is stated here rather than only in the help text because this is
/// where an operator actually meets it: nothing in this command compares a proposal against what
/// the context already holds, so overlap is theirs to adjudicate beforehand.
fn confirm_batch(proposal_count: usize, context_ref: &str, cohort: &str) -> Result<bool> {
    let plural = if proposal_count == 1 { "y" } else { "ies" };
    output::warning(format!(
        "about to create {proposal_count} memor{plural} in {context_ref} from the `{cohort}` cohort"
    ));
    output::plain(
        "  Near-duplicates are NOT detected: nothing here is checked against what the context \
         already holds. Adjudicate overlap before migrating — this command will not.",
    );
    output::plain("  Re-run with --dry-run to see the per-file plan without writing.");
    dialoguer::Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt("Create them?")
        .default(false)
        .interact()
        .map_err(|e| TemperError::Config(format!("prompt error: {e}")))
}

/// Write one memory through the ordinary `resource create` path — the same code a person running
/// `temper resource create --type memory` takes, rather than a second write path that would drift
/// from it.
///
/// The body reaches `create` as `--body @<tempfile>` because the on-disk file still carries its
/// frontmatter and the memory's body must not.
fn write_proposal(
    config: &crate::config::Config,
    context_ref: &str,
    proposal: &MigrationProposal,
    format: OutputFormat,
) -> Result<()> {
    let mut body_file = tempfile::NamedTempFile::new()?;
    std::io::Write::write_all(&mut body_file, proposal.body.as_bytes())?;
    let open_meta = serde_json::to_string(&proposal_open_meta(proposal))
        .map_err(|e| TemperError::BadRequest(format!("open_meta serialization: {e}")))?;

    crate::commands::resource::create(
        config,
        crate::commands::resource::CreateResourceArgs {
            doc_type: MEMORY_DOC_TYPE,
            title: &proposal.title,
            context: Some(context_ref),
            cogmap: None,
            mode: None,
            effort: None,
            open_meta: Some(&open_meta),
            goal: None,
            task: None,
            body_flag: Some(format!("@{}", body_file.path().display())),
            from: None,
            sources: Vec::new(),
            sources_as_edges: false,
            no_source: false,
            format,
            act: temper_core::types::ActInput::default(),
        },
    )
}

/// Read every memory file sitting alongside the index, with its content and its mtime.
///
/// The index file itself is excluded — it is the thing being harvested for titles, not a memory.
/// Any file whose content or mtime cannot be read is dropped rather than failing the scan; a
/// vanished file is not a reason to strand the other 181.
pub fn scan_memory_dir(index_path: &std::path::Path) -> Vec<ScannedFile> {
    let Some(dir) = index_path.parent() else {
        return Vec::new();
    };
    let index_filename = index_path.file_name().and_then(std::ffi::OsStr::to_str);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut files: Vec<ScannedFile> = entries
        .filter_map(std::io::Result::ok)
        .filter_map(|entry| {
            let filename = entry.file_name().to_str()?.to_string();
            if !filename.ends_with(".md") || index_filename == Some(filename.as_str()) {
                return None;
            }
            let content = std::fs::read_to_string(entry.path()).ok()?;
            // **Frontmatter is what separates a memory file from an index, and the filename
            // cannot.** Excluding `index_path` alone was sufficient while temper owned the only
            // index in the directory; it stopped being sufficient when the index moved to a
            // sibling name and Claude Code's own frontmatter-free `MEMORY.md` was left beside the
            // memory files. Without this, that file is scanned as an un-migrated memory forever.
            // Deliberately weaker than "parses as frontmatter" — a malformed block still belongs
            // to a memory the reader must be told about.
            if !content.starts_with("---") {
                return None;
            }
            let mtime = entry.metadata().ok()?.modified().ok()?;
            Some(ScannedFile {
                filename,
                content,
                mtime: chrono::DateTime::<chrono::Utc>::from(mtime).date_naive(),
            })
        })
        .collect();
    files.sort_by(|a, b| a.filename.cmp(&b.filename));
    files
}

/// Where a cohort migrates when `--context` is not given.
///
/// This is the design's homes table, read straight off it: `feedback` is the cross-project cohort
/// and lands in the first `shared_contexts` entry; `project` and `reference` sit beside the
/// resources they discuss, in the first `project_contexts` entry. Reach stays a property of which
/// config list names the context — the memory itself never carries one and so can never claim one
/// it does not have.
///
/// `None` when the relevant list is empty: a cohort with nowhere to go is a refusal, never a
/// silent fallback to the other list.
pub fn default_context_for_cohort<'a>(mem: &'a MemoryConfig, cohort: &str) -> Option<&'a str> {
    let list = if cohort == CROSS_PROJECT_COHORT {
        &mem.shared_contexts
    } else {
        &mem.project_contexts
    };
    list.first().map(String::as_str)
}

/// The one cohort whose memories apply to every project on the machine, and therefore the one
/// that homes in `shared_contexts`. Named rather than inlined at the comparison, because "which
/// cohort is cross-project" is a claim about the corpus, not a string.
pub const CROSS_PROJECT_COHORT: &str = "feedback";

/// Build the `open_meta` a proposal is written with.
///
/// `status` and `verified` live in the OPEN tier, where nothing validates them at write time —
/// `emit` is the only gate, and it refuses the whole index on a malformed one. So this is the
/// exact point where a typo would go unnoticed until the next emit, which is why
/// `migrate_writes_open_meta_that_emit_accepts` runs the output straight through `emit`'s parser
/// rather than asserting the keys by hand.
pub fn proposal_open_meta(p: &MigrationProposal) -> serde_json::Value {
    serde_json::to_value(MemoryOpenMeta {
        descriptor: &p.descriptor,
        status: "active",
        verified: p.verified.format("%Y-%m-%d").to_string(),
        source_file: &p.source_file,
    })
    .expect("MemoryOpenMeta has no non-serializable field")
}

/// The open-tier keys a migrated memory carries, as a type rather than an inline literal — the
/// four keys are a known shape and the compiler should be the one checking them.
///
/// Deliberately NOT the whole open tier: `tags` and `relates_to` are absent because the files
/// carry no tags, and because a `[[wikilink]]` names a memory whose UUID does not exist until it
/// has itself been migrated. Filling either would be invention. See the module doc's remainder.
#[derive(serde::Serialize)]
struct MemoryOpenMeta<'a> {
    descriptor: &'a str,
    /// Always `active` at migration. A memory that turns out wrong is superseded later, by a
    /// decision someone makes — never pre-judged by the migration that moved it.
    status: &'static str,
    verified: String,
    source_file: &'a str,
}

/// How this run may write. Decided purely from the flags and whether a human is attached, before
/// any file is read or any resource created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// Plan and print; write nothing. Always permitted, with or without a terminal — a read-only
    /// run has nothing to be unattended about.
    DryRun,
    /// A human is attached; the whole batch is shown once and confirmed before the first write.
    Interactive,
    /// Explicitly authorized to run with no human. The flag **is** the confirmation, so the batch
    /// is written without asking.
    Unattended,
}

/// Decide the run mode, or refuse.
///
/// Shared with [`super::harvest`], and the refusal is the point for both: each of them writes —
/// one creates resources, the other rewrites files on disk — and a write run with no human
/// attached and no explicit authorization is exactly the case that should not proceed. What a
/// given command asks before writing, and therefore what `--unattended` waives, belongs in that
/// command's own help rather than in this shared message.
pub fn resolve_run_mode(
    dry_run: bool,
    unattended: bool,
    stdin_is_tty: bool,
) -> std::result::Result<RunMode, String> {
    if dry_run {
        return Ok(RunMode::DryRun);
    }
    if unattended {
        return Ok(RunMode::Unattended);
    }
    if stdin_is_tty {
        return Ok(RunMode::Interactive);
    }
    Err(
        "refusing to write with no terminal attached: there is nobody to confirm the run. \
         Re-run with --dry-run to see the plan, or --unattended to authorize writing without \
         a confirmation."
            .to_string(),
    )
}

/// One local `.md` file as the shell found it. `mtime` is the fallback `verified` date for the
/// files carrying no `metadata.modified` — 50 of the 69 in the first cohort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    pub filename: String,
    pub content: String,
    pub mtime: NaiveDate,
}

/// A memory this run would create. Everything here is decided before any network call, so the
/// dry-run shows exactly what a real run would write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationProposal {
    /// `open_meta.source_file` — the only thing tying this memory back to the file it came from,
    /// and therefore the only thing that makes `status`'s orphan report mean anything.
    pub source_file: String,
    pub title: String,
    pub descriptor: String,
    /// `open_meta.verified`. Migration is not a re-check, so this is the date the claim was last
    /// *written*, never the date it was migrated.
    pub verified: NaiveDate,
    pub body: String,
}

/// Why a scanned file is not in this run's proposals. Every skip is reported with its reason —
/// a file that silently vanishes from a migration is indistinguishable from one that migrated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// The file parsed, but its `type` is not the cohort being migrated. Carries what was found,
    /// so a mis-filed file (a `project_`-named file whose type is `reference`) is visible rather
    /// than merely absent.
    NotInCohort { found: String },
    /// A memory in Temper already names this file in `open_meta.source_file`. This is what makes
    /// re-running `migrate` safe.
    AlreadyMigrated,
    /// No `MEMORY.md` link names this file, so it has no curated title. A title is not invented
    /// from the filename — see [`harvest_titles`].
    NoHarvestedTitle,
    /// The file could not be parsed. Reported for every scanned file, in or out of cohort, since
    /// an unparseable file's cohort is exactly what cannot be known.
    Unparseable(FileDefect),
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInCohort { found } => write!(f, "not in cohort (type is `{found}`)"),
            Self::AlreadyMigrated => write!(f, "already in Temper (matched by source_file)"),
            Self::NoHarvestedTitle => {
                write!(f, "no MEMORY.md link names it, so it has no curated title")
            }
            Self::Unparseable(d) => write!(f, "unparseable: {d}"),
        }
    }
}

/// What a run would do, decided purely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlan {
    pub proposals: Vec<MigrationProposal>,
    pub skipped: Vec<(String, SkipReason)>,
}

/// Decide the batch. Pure over its four inputs: the scanned files, the titles harvested from
/// `MEMORY.md`, the cohort to migrate, and the `source_file` values already present in Temper.
pub fn plan_migration(
    scanned: &[ScannedFile],
    titles: &HashMap<String, String>,
    cohort: &str,
    already_migrated: &HashSet<String>,
) -> MigrationPlan {
    let mut ordered: Vec<&ScannedFile> = scanned.iter().collect();
    ordered.sort_by(|a, b| a.filename.cmp(&b.filename));

    let mut proposals = Vec::new();
    let mut skipped = Vec::new();

    for file in ordered {
        let parsed = match parse_memory_file(&file.content) {
            Ok(p) => p,
            Err(defect) => {
                // Reported whatever the cohort filter would have said, because an unparseable
                // file's cohort is exactly the thing that cannot be known.
                skipped.push((file.filename.clone(), SkipReason::Unparseable(defect)));
                continue;
            }
        };
        if parsed.cohort != cohort {
            skipped.push((
                file.filename.clone(),
                SkipReason::NotInCohort {
                    found: parsed.cohort,
                },
            ));
            continue;
        }
        if already_migrated.contains(&file.filename) {
            skipped.push((file.filename.clone(), SkipReason::AlreadyMigrated));
            continue;
        }
        // The file's own stamped title wins over the index's link text. Both are the same curated
        // string — `harvest` put it there — but preferring the file is what lets this command keep
        // working once `emit` has taken the index over, instead of depending on a generated file
        // to carry the titles forward.
        let Some(title) = parsed
            .title
            .clone()
            .or_else(|| titles.get(&file.filename).cloned())
        else {
            skipped.push((file.filename.clone(), SkipReason::NoHarvestedTitle));
            continue;
        };

        proposals.push(MigrationProposal {
            source_file: file.filename.clone(),
            title,
            descriptor: parsed.descriptor,
            verified: parsed.modified.unwrap_or(file.mtime),
            body: parsed.body,
        });
    }

    MigrationPlan { proposals, skipped }
}

/// Map each memory filename linked from a hand-written `MEMORY.md` to that link's text.
///
/// **This is the only place a human-readable title for a memory exists.** The files' own `name:`
/// frontmatter is not a substitute: across the feedback cohort 39 of 69 carry the snake_case
/// filename stem, and snake_case fails `memory.schema.json`'s `temper-slug` pattern
/// (`^[a-z0-9][a-z0-9-]*$`) outright. Harvesting the index before it is retired is the point.
///
/// Only `.md` targets with no path separator are collected — the index also links to specs, PRs
/// and issues, and none of those name a memory file.
pub fn harvest_titles(memory_md: &str) -> HashMap<String, String> {
    let mut titles = HashMap::new();
    for line in memory_md.lines() {
        let mut cursor = 0usize;
        while let Some(rel) = line[cursor..].find("](") {
            let bracket_close = cursor + rel;
            let target_start = bracket_close + 2;
            let Some(target_len) = line[target_start..].find(')') else {
                break;
            };
            let target = &line[target_start..target_start + target_len];
            cursor = target_start + target_len + 1;

            if !names_a_memory_file(target) {
                continue;
            }
            let Some(bracket_open) = line[..bracket_close].rfind('[') else {
                continue;
            };
            titles.insert(
                target.to_string(),
                line[bracket_open + 1..bracket_close].to_string(),
            );
        }
    }
    titles
}

/// A link target names a memory file only when it is a bare `.md` filename. A path
/// (`docs/…/x.md`) is a repo document and a scheme (`https://`, `temper://`) is not a file at
/// all — sweeping either in would mint titles for files that do not exist, and would let a
/// `docs/…/foo.md` shadow a memory legitimately named `foo.md`.
fn names_a_memory_file(target: &str) -> bool {
    target.ends_with(".md")
        && target.len() > ".md".len()
        && !target.contains('/')
        && !target.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use temper_core::types::ids::{ProfileId, ResourceId};
    use temper_core::types::managed_meta::ManagedMeta;
    use temper_core::types::resource::{BodyStorage, IngestState};
    use temper_core::types::resource_view::ResourceView;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("test date")
    }

    /// The newer of the two shapes on disk: 155 of the 182 files nest `type` under `metadata:`
    /// alongside a `modified` timestamp.
    const NESTED_SHAPE: &str = "\
---
name: feedback_a_clause_cannot_retire_its_own_goal
description: \"A goal's criteria say whether it was achieved well\"
metadata:
  node_type: memory
  type: feedback
  originSessionId: c09403de-0eca-4e23-99d3-cc4df40d9973
  modified: 2026-07-27T11:17:19.411Z
---

Pete, 2026-07-27:

The body.
";

    /// The older shape: 27 files carry `type` at the root and no `modified` at all.
    const FLAT_SHAPE: &str = "\
---
name: Never ship \"for now\" workarounds in code
description: Don't leave \"for now\" comments in shipped code
type: feedback
originSessionId: 4be1a0cf-c843-422f-acdb-0adda843de45
---
Never ship code with \"for now\".
";

    /// The smallest thing the scan accepts. **Every fixture below carries frontmatter because
    /// every real memory file does** — `harvest` guarantees it and Claude Code's auto-memory
    /// template emits it. Bodies of `"a"` / `"x"` passed before the frontmatter rule existed and
    /// would have gone on passing after it, which is a fixture describing a file shape the
    /// producer never emits.
    const MINIMAL: &str = "---\nname: feedback_a\n---\nbody\n";

    /// The index is the title source, not a memory. Scanning it in would try to migrate
    /// `MEMORY.md` itself.
    #[test]
    fn the_scan_excludes_the_index_file_and_non_markdown() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = dir.path().join("MEMORY.md");
        std::fs::write(&index, "- [t](feedback_a.md)\n").unwrap();
        std::fs::write(dir.path().join("feedback_a.md"), MINIMAL).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "b").unwrap();

        let found = scan_memory_dir(&index);

        let names: Vec<&str> = found.iter().map(|f| f.filename.as_str()).collect();
        assert_eq!(names, ["feedback_a.md"]);
    }

    /// **A frontmatter-free `.md` beside the memory files is an index, not a memory.** Since
    /// 2026-08-03 temper's index is `MANAGED_MEMORY.md` and Claude Code's auto-memory keeps
    /// `MEMORY.md`, so a sibling the scan must not claim is now the normal state of the directory
    /// rather than a hypothetical. Excluding `index_path` alone cannot do this — only one of the
    /// two is ever the configured index.
    #[test]
    fn the_scan_excludes_a_frontmatter_free_sibling_index() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = dir.path().join("MANAGED_MEMORY.md");
        std::fs::write(&index, "<!-- GENERATED -->\n\n# Memory index\n").unwrap();
        std::fs::write(
            dir.path().join("MEMORY.md"),
            "# Memory index\n\n- [a pointer](feedback_a.md) — hook\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("feedback_a.md"), MINIMAL).unwrap();

        let found = scan_memory_dir(&index);

        let names: Vec<&str> = found.iter().map(|f| f.filename.as_str()).collect();
        assert_eq!(
            names,
            ["feedback_a.md"],
            "the harness's own index must not be scanned as an un-migrated memory"
        );
    }

    #[test]
    fn the_scan_carries_each_files_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = dir.path().join("MEMORY.md");
        std::fs::write(&index, "").unwrap();
        std::fs::write(dir.path().join("feedback_a.md"), MINIMAL).unwrap();

        let found = scan_memory_dir(&index);

        assert_eq!(found[0].content, MINIMAL, "content is carried verbatim");
    }

    /// The mtime is the `verified` fallback for 50 of the 69 files in the first cohort, so it has
    /// to be a real filesystem reading — not today's date wearing a fallback's name.
    #[test]
    fn the_scan_reads_a_real_mtime_not_todays_date() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = dir.path().join("MEMORY.md");
        std::fs::write(&index, "").unwrap();
        let file = dir.path().join("feedback_a.md");
        std::fs::write(&file, MINIMAL).unwrap();
        // 2026-05-14T00:00:00Z, chosen so a today-shaped bug is visibly wrong.
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_778_716_800);
        filetime::set_file_mtime(&file, filetime::FileTime::from_system_time(when)).unwrap();

        let found = scan_memory_dir(&index);

        assert_eq!(found[0].mtime, d("2026-05-14"));
    }

    #[test]
    fn scanning_a_directory_that_does_not_exist_yields_nothing_rather_than_failing() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = dir.path().join("nope/MEMORY.md");
        assert!(scan_memory_dir(&index).is_empty());
    }

    fn mem_cfg() -> MemoryConfig {
        MemoryConfig {
            shared_contexts: vec!["@me/working-agreements".to_string()],
            project_contexts: vec!["@me/temper".to_string()],
            index_path: "~/.claude/projects/p/memory/MEMORY.md".to_string(),
            stale_after_days: 90,
            reinforced_min: None,
        }
    }

    /// The cross-project cohort — the whole reason this migration goes first. It must land where
    /// every project on the machine reads it, not beside one project's goals.
    #[test]
    fn the_feedback_cohort_defaults_to_the_first_shared_context() {
        assert_eq!(
            default_context_for_cohort(&mem_cfg(), "feedback"),
            Some("@me/working-agreements")
        );
    }

    #[test]
    fn the_project_and_reference_cohorts_default_to_the_first_project_context() {
        assert_eq!(
            default_context_for_cohort(&mem_cfg(), "project"),
            Some("@me/temper")
        );
        assert_eq!(
            default_context_for_cohort(&mem_cfg(), "reference"),
            Some("@me/temper")
        );
    }

    /// A cohort with nowhere to go must refuse, not quietly land in the other list. Falling back
    /// would give a memory a reach nobody configured — the one property the homes model exists to
    /// protect.
    #[test]
    fn a_cohort_with_an_empty_list_has_no_default_rather_than_the_other_list() {
        let mut cfg = mem_cfg();
        cfg.shared_contexts.clear();
        assert_eq!(default_context_for_cohort(&cfg, "feedback"), None);
    }

    fn proposal() -> MigrationProposal {
        MigrationProposal {
            source_file: "feedback_a.md".to_string(),
            title: "a curated hook".to_string(),
            descriptor: "the recall-relevance line".to_string(),
            verified: d("2026-07-27"),
            body: "the body\n".to_string(),
        }
    }

    /// Wrap an `open_meta` value in the `ResourceView` shape the server would hand back, so
    /// `emit`'s own parser can be run against what `migrate` writes.
    fn detail_with(open_meta: serde_json::Value) -> ResourceView {
        ResourceView {
            id: ResourceId::from(uuid::Uuid::now_v7()),
            r#ref: String::new(),
            title: "a curated hook".to_string(),
            origin_uri: String::new(),
            kb_context_id: None,
            context_name: None,
            context_slug: Some("working-agreements".to_string()),
            context_owner_ref: Some("@me".to_string()),
            context_ref: None,
            cogmap_id: None,
            cogmap_name: None,
            doc_type_name: "memory".to_string(),
            owner_handle: "someone".to_string(),
            owner_profile_id: ProfileId::from(uuid::Uuid::now_v7()),
            originator_profile_id: ProfileId::from(uuid::Uuid::now_v7()),
            is_active: true,
            created: DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"),
            updated: DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"),
            body_hash: None,
            ingest_state: Some(IngestState::Complete),
            body_storage: Some(BodyStorage::Derived),
            managed_meta: ManagedMeta::default(),
            open_meta: Some(open_meta),
            content: None,
        }
        .with_derived_refs()
    }

    /// The differential guard, and the reason this is not a hand-written key assertion: `status`
    /// and `verified` live in the open tier, where nothing checks them at write time. `emit` is
    /// the only gate. Running `migrate`'s output through `emit`'s own parser is what stops the
    /// writer and the gate from drifting into disagreement about the contract.
    #[test]
    fn migrate_writes_open_meta_that_emit_accepts() {
        let om = proposal_open_meta(&proposal());
        let entry = super::super::render::parse_entry(&detail_with(om))
            .expect("emit's gate must accept what migrate writes");
        assert_eq!(entry.status, "active");
        assert_eq!(entry.verified, d("2026-07-27"));
    }

    /// The descriptor is no longer rendered into the index, so nothing downstream of `emit` would
    /// notice if `migrate` stopped writing it. It still has a job — FTS weight-D searchability on
    /// the resource — and this is now the only thing that holds the writer to it.
    #[test]
    fn the_descriptor_is_still_written_even_though_the_index_no_longer_prints_it() {
        let om = proposal_open_meta(&proposal());
        assert_eq!(
            om.get("descriptor").and_then(serde_json::Value::as_str),
            Some("the recall-relevance line")
        );
    }

    /// Without `source_file`, `status`'s "local files with no counterpart" report is meaningless —
    /// it is the only tie between a Temper memory and the file it came from, and no inference
    /// recovers it (a title is never a filename).
    #[test]
    fn every_migrated_memory_records_the_file_it_came_from() {
        let om = proposal_open_meta(&proposal());
        assert_eq!(
            om.get("source_file").and_then(serde_json::Value::as_str),
            Some("feedback_a.md")
        );
    }

    /// The write must not carry `title` or `slug` in the open tier — canonical identity lives in
    /// the managed fields, and `validate_open_meta_send_side` warns on a bare copy. Sixty-nine
    /// writes each emitting a warning is how a real warning gets trained out of being read.
    #[test]
    fn open_meta_carries_no_discouraged_identity_keys() {
        let om = proposal_open_meta(&proposal());
        let issues = temper_workflow::schema::check_discouraged_open_meta_keys(&om);
        assert!(
            issues.is_empty(),
            "migrate must not write discouraged keys: {issues:?}"
        );
    }

    #[test]
    fn open_meta_passes_the_send_side_shape_validation() {
        let om = proposal_open_meta(&proposal());
        let issues =
            temper_workflow::schema::validate_open_meta(&om).expect("validation must not error");
        assert!(issues.is_empty(), "{issues:?}");
    }

    /// The anti-noise property, and the reason this command asks per batch rather than per file:
    /// re-running `migrate` once everything has migrated plans nothing, and a confirmation with
    /// nothing behind it teaches an operator to answer prompts without reading them — which is
    /// how the per-file gate this replaced ended up being routed around.
    #[test]
    fn an_empty_batch_is_never_confirmed() {
        assert!(!batch_confirmation_required(RunMode::Interactive, 0));
    }

    /// The one prompt the command has left: a human is attached and there is something to write.
    #[test]
    fn an_interactive_run_with_proposals_is_confirmed() {
        assert!(batch_confirmation_required(RunMode::Interactive, 1));
        assert!(batch_confirmation_required(RunMode::Interactive, 69));
    }

    /// `--unattended` **is** the authorization. Asking for a second one would hang every agent
    /// and hook run on a prompt nothing is there to answer.
    #[test]
    fn an_unattended_run_is_not_confirmed_because_the_flag_is_the_authorization() {
        assert!(!batch_confirmation_required(RunMode::Unattended, 69));
    }

    /// A dry run writes nothing, so there is nothing to authorize — and a read-only run that
    /// stopped to ask would be unusable from an agent or a hook, which is where it is run from.
    #[test]
    fn a_dry_run_is_never_confirmed() {
        assert!(!batch_confirmation_required(RunMode::DryRun, 69));
    }

    /// A dry run writes nothing, so it has nothing to be unattended about — an agent or a hook
    /// must be able to see the plan with no terminal and no flag.
    #[test]
    fn a_dry_run_is_permitted_with_no_terminal() {
        assert_eq!(resolve_run_mode(true, false, false), Ok(RunMode::DryRun));
    }

    /// The refusal the design asks for, and it is shared with `harvest`: both commands write, so
    /// a write run with no human attached and no explicit authorization does not proceed. The
    /// message must stay surface-neutral — it is rendered by both — while still naming the two
    /// ways forward, which is what this asserts.
    #[test]
    fn a_write_run_with_no_terminal_and_no_flag_is_refused() {
        let err = resolve_run_mode(false, false, false).expect_err("must refuse");
        assert!(
            err.contains("--dry-run") && err.contains("--unattended"),
            "the refusal must name both ways forward: {err}"
        );
    }

    #[test]
    fn a_terminal_gives_an_interactive_run_without_any_flag() {
        assert_eq!(
            resolve_run_mode(false, false, true),
            Ok(RunMode::Interactive)
        );
    }

    #[test]
    fn the_explicit_flag_authorizes_a_run_with_no_terminal() {
        assert_eq!(
            resolve_run_mode(false, true, false),
            Ok(RunMode::Unattended)
        );
    }

    /// `--dry-run` wins over `--unattended`: the pair reads as "show me what an unattended run
    /// would do", and resolving it to a write would be the most expensive possible reading.
    #[test]
    fn dry_run_wins_when_both_flags_are_given() {
        assert_eq!(resolve_run_mode(true, true, false), Ok(RunMode::DryRun));
    }

    /// A scanned file built from one of the two real shapes, with a chosen cohort and mtime.
    fn scanned(filename: &str, cohort: &str, modified: Option<&str>, mtime: &str) -> ScannedFile {
        let content = match modified {
            Some(m) => format!(
                "---\nname: {filename}\ndescription: desc of {filename}\nmetadata:\n  type: {cohort}\n  modified: {m}T11:17:19.411Z\n---\nbody of {filename}\n"
            ),
            None => format!(
                "---\nname: {filename}\ndescription: desc of {filename}\ntype: {cohort}\n---\nbody of {filename}\n"
            ),
        };
        ScannedFile {
            filename: filename.to_string(),
            content,
            mtime: d(mtime),
        }
    }

    fn titles_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(f, t)| ((*f).to_string(), (*t).to_string()))
            .collect()
    }

    fn skip_reason<'a>(plan: &'a MigrationPlan, filename: &str) -> Option<&'a SkipReason> {
        plan.skipped
            .iter()
            .find(|(f, _)| f == filename)
            .map(|(_, r)| r)
    }

    #[test]
    fn a_cohort_file_with_a_harvested_title_becomes_a_proposal() {
        let files = vec![scanned("feedback_a.md", "feedback", None, "2026-06-01")];
        let plan = plan_migration(
            &files,
            &titles_of(&[("feedback_a.md", "a curated hook")]),
            "feedback",
            &HashSet::new(),
        );
        assert_eq!(plan.proposals.len(), 1);
        let p = &plan.proposals[0];
        assert_eq!(p.source_file, "feedback_a.md");
        assert_eq!(p.title, "a curated hook");
        assert_eq!(p.descriptor, "desc of feedback_a.md");
        assert_eq!(p.body, "body of feedback_a.md\n");
    }

    /// Once `harvest` has stamped a file, its own `title:` is the source — not the index's link
    /// text. This is what lets `migrate` keep working after `emit` takes the index over: the
    /// generated index is no longer the thing carrying these titles.
    ///
    /// The two strings are made to differ only so the test can tell which one was used; in
    /// practice `harvest` puts the same curated string in both places.
    #[test]
    fn a_stamped_title_on_the_file_wins_over_the_indexs_link_text() {
        let mut file = scanned("feedback_a.md", "feedback", None, "2026-06-01");
        file.content = file
            .content
            .replace("---\nname:", "---\ntitle: the file's own title\nname:");

        let plan = plan_migration(
            &[file],
            &titles_of(&[("feedback_a.md", "the index's link text")]),
            "feedback",
            &HashSet::new(),
        );
        assert_eq!(plan.proposals.len(), 1);
        assert_eq!(plan.proposals[0].title, "the file's own title");
    }

    /// The fallback stays live: a file harvest has not reached is still migratable while the
    /// hand-written index survives. Removing the fallback would strand every un-stamped file.
    #[test]
    fn an_unstamped_file_still_falls_back_to_the_indexs_link_text() {
        let plan = plan_migration(
            &[scanned("feedback_a.md", "feedback", None, "2026-06-01")],
            &titles_of(&[("feedback_a.md", "the index's link text")]),
            "feedback",
            &HashSet::new(),
        );
        assert_eq!(plan.proposals[0].title, "the index's link text");
    }

    #[test]
    fn verified_prefers_the_files_modified_date_over_its_mtime() {
        let files = vec![scanned(
            "feedback_a.md",
            "feedback",
            Some("2026-07-27"),
            "2026-08-01",
        )];
        let plan = plan_migration(
            &files,
            &titles_of(&[("feedback_a.md", "t")]),
            "feedback",
            &HashSet::new(),
        );
        assert_eq!(
            plan.proposals[0].verified,
            d("2026-07-27"),
            "the date the claim was written, not the date the file was last touched"
        );
    }

    /// 50 of the 69 feedback files carry no `modified`. The fallback must be the file's mtime and
    /// never today — migrating a memory is not re-checking it, and a `verified` stamped at
    /// migration time asserts attention nobody spent.
    #[test]
    fn verified_falls_back_to_mtime_when_the_file_has_no_modified_date() {
        let files = vec![scanned("feedback_a.md", "feedback", None, "2026-05-14")];
        let plan = plan_migration(
            &files,
            &titles_of(&[("feedback_a.md", "t")]),
            "feedback",
            &HashSet::new(),
        );
        assert_eq!(plan.proposals[0].verified, d("2026-05-14"));
    }

    #[test]
    fn a_file_outside_the_cohort_is_skipped_and_its_actual_type_is_named() {
        let files = vec![scanned("project_x.md", "reference", None, "2026-06-01")];
        let plan = plan_migration(
            &files,
            &titles_of(&[("project_x.md", "t")]),
            "feedback",
            &HashSet::new(),
        );
        assert!(plan.proposals.is_empty());
        assert_eq!(
            skip_reason(&plan, "project_x.md"),
            Some(&SkipReason::NotInCohort {
                found: "reference".to_string()
            }),
            "a mis-filed file must be visible, not merely absent"
        );
    }

    /// The property that makes re-running `migrate` safe: a file whose `source_file` is already
    /// claimed by a memory in Temper is never proposed a second time.
    #[test]
    fn a_file_already_migrated_is_never_proposed_again() {
        let files = vec![scanned("feedback_a.md", "feedback", None, "2026-06-01")];
        let already: HashSet<String> = ["feedback_a.md".to_string()].into_iter().collect();
        let plan = plan_migration(
            &files,
            &titles_of(&[("feedback_a.md", "t")]),
            "feedback",
            &already,
        );
        assert!(plan.proposals.is_empty());
        assert_eq!(
            skip_reason(&plan, "feedback_a.md"),
            Some(&SkipReason::AlreadyMigrated)
        );
    }

    /// Four files on disk are linked from nothing in `MEMORY.md`. Their titles are not invented
    /// from the filename — `feedback_a_clause_cannot_retire_its_own_goal` is not a title, and a
    /// title-cased version of it is worse than none.
    #[test]
    fn a_file_with_no_harvested_title_is_skipped_rather_than_given_an_invented_one() {
        let files = vec![scanned(
            "feedback_orphan.md",
            "feedback",
            None,
            "2026-06-01",
        )];
        let plan = plan_migration(&files, &HashMap::new(), "feedback", &HashSet::new());
        assert!(plan.proposals.is_empty());
        assert_eq!(
            skip_reason(&plan, "feedback_orphan.md"),
            Some(&SkipReason::NoHarvestedTitle)
        );
    }

    /// One unparseable file must not strand the other 68.
    #[test]
    fn an_unparseable_file_is_skipped_and_the_rest_still_plan() {
        let mut broken = scanned("feedback_broken.md", "feedback", None, "2026-06-01");
        broken.content = "# no frontmatter here\n".to_string();
        let files = vec![
            broken,
            scanned("feedback_good.md", "feedback", None, "2026-06-01"),
        ];
        let plan = plan_migration(
            &files,
            &titles_of(&[("feedback_broken.md", "t"), ("feedback_good.md", "t")]),
            "feedback",
            &HashSet::new(),
        );
        assert_eq!(plan.proposals.len(), 1);
        assert_eq!(plan.proposals[0].source_file, "feedback_good.md");
        assert_eq!(
            skip_reason(&plan, "feedback_broken.md"),
            Some(&SkipReason::Unparseable(FileDefect::NoFence))
        );
    }

    /// A dry-run is read to decide whether to run for real, so two runs over the same inputs must
    /// produce the same order — a plan whose order comes from directory iteration is not reviewable.
    #[test]
    fn proposals_are_ordered_by_filename() {
        let files = vec![
            scanned("feedback_c.md", "feedback", None, "2026-06-01"),
            scanned("feedback_a.md", "feedback", None, "2026-06-01"),
            scanned("feedback_b.md", "feedback", None, "2026-06-01"),
        ];
        let plan = plan_migration(
            &files,
            &titles_of(&[
                ("feedback_a.md", "t"),
                ("feedback_b.md", "t"),
                ("feedback_c.md", "t"),
            ]),
            "feedback",
            &HashSet::new(),
        );
        let order: Vec<&str> = plan
            .proposals
            .iter()
            .map(|p| p.source_file.as_str())
            .collect();
        assert_eq!(order, ["feedback_a.md", "feedback_b.md", "feedback_c.md"]);
    }

    #[test]
    fn parse_reads_the_cohort_from_the_nested_metadata_shape() {
        let p = parse_memory_file(NESTED_SHAPE).expect("nested shape must parse");
        assert_eq!(p.cohort, "feedback");
        assert_eq!(
            p.descriptor,
            "A goal's criteria say whether it was achieved well"
        );
    }

    /// Both shapes are live on disk. A parser that handled only the nested one would silently
    /// drop 27 files, 27 of which are in the cohort being migrated first.
    #[test]
    fn parse_reads_the_cohort_from_the_flat_shape_too() {
        let p = parse_memory_file(FLAT_SHAPE).expect("flat shape must parse");
        assert_eq!(p.cohort, "feedback");
        assert_eq!(
            p.descriptor,
            "Don't leave \"for now\" comments in shipped code"
        );
    }

    #[test]
    fn parse_takes_the_date_part_of_a_modified_timestamp() {
        let p = parse_memory_file(NESTED_SHAPE).expect("must parse");
        assert_eq!(
            p.modified,
            Some(d("2026-07-27")),
            "`verified` is a date; the time of day is not a claim about attention"
        );
    }

    /// 50 of the 69 feedback files carry no `modified` at all. Absent is ordinary — the caller
    /// falls back to the file's mtime, so this must be `None` rather than a defect or a guess.
    #[test]
    fn parse_reports_no_modified_date_rather_than_inventing_one() {
        let p = parse_memory_file(FLAT_SHAPE).expect("must parse");
        assert_eq!(p.modified, None);
    }

    #[test]
    fn parse_returns_the_body_after_the_closing_fence() {
        let p = parse_memory_file(FLAT_SHAPE).expect("must parse");
        assert_eq!(p.body, "Never ship code with \"for now\".\n");
    }

    #[test]
    fn a_file_with_no_frontmatter_is_a_defect_not_an_empty_memory() {
        assert_eq!(
            parse_memory_file("# just a heading\n"),
            Err(FileDefect::NoFence)
        );
    }

    /// A real file on disk, `feedback_nextest_summary_lies.md`: its `description:` is an
    /// unquoted plain scalar containing `tests run: N passed`, and a colon-space inside a plain
    /// scalar is a YAML error. The fence is perfectly well-formed — so reporting this as "no
    /// frontmatter" sends whoever reads the skip list looking for a missing `---` that is right
    /// there. The two defects have different fixes and must not share a message.
    #[test]
    fn a_well_fenced_file_with_invalid_yaml_is_reported_as_invalid_yaml_not_a_missing_fence() {
        let content = "\
---
name: a memory
description: prints a \"Summary [N tests run: N passed]\" line per binary
type: feedback
---
body
";
        let err =
            parse_memory_file(content).expect_err("colon-space in a plain scalar is a YAML error");
        assert!(
            matches!(err, FileDefect::InvalidYaml(_)),
            "the fence is well-formed; only the YAML is bad — got {err:?}"
        );
        assert!(
            !err.to_string().contains("frontmatter block"),
            "the message must not send a reader looking for a missing fence: {err}"
        );
    }

    #[test]
    fn a_file_with_no_description_is_a_defect() {
        let content = "---\nname: x\ntype: feedback\n---\nbody\n";
        assert_eq!(
            parse_memory_file(content),
            Err(FileDefect::MissingKey("description")),
            "descriptor is what makes a memory findable; an empty one is not a default"
        );
    }

    #[test]
    fn a_file_with_no_type_is_a_defect() {
        let content = "---\nname: x\ndescription: d\n---\nbody\n";
        assert_eq!(
            parse_memory_file(content),
            Err(FileDefect::MissingKey("type"))
        );
    }

    #[test]
    fn harvest_maps_a_link_target_to_its_text() {
        let index = "- [ground in data, not the AST](feedback_ground_in_data_not_ast.md) · more\n";
        let titles = harvest_titles(index);
        assert_eq!(
            titles
                .get("feedback_ground_in_data_not_ast.md")
                .map(String::as_str),
            Some("ground in data, not the AST"),
            "the link text is the curated title"
        );
    }

    #[test]
    fn harvest_collects_every_link_on_a_single_line() {
        let index =
            "- **The ritual** — [a](feedback_a.md) · [b](feedback_b.md) · [c](project_c.md)\n";
        let titles = harvest_titles(index);
        assert_eq!(
            titles.len(),
            3,
            "one line routinely carries several memories"
        );
        assert_eq!(titles.get("project_c.md").map(String::as_str), Some("c"));
    }

    /// The index links to far more than memory files — design docs, PRs, issues, http URLs. A
    /// harvest that swept those up would invent titles for files that do not exist and, worse,
    /// would let a `docs/…/foo.md` path collide with a memory named `foo.md`.
    #[test]
    fn harvest_ignores_links_that_do_not_name_a_bare_md_file() {
        let index = "\
- [a real memory](feedback_real.md)
- [a design doc](docs/superpowers/specs/2026-08-01-memories-in-temper-design.md)
- [an issue](https://github.com/tasker-systems/temper/issues/581)
- [a temper resource](temper://019fbef0-437d-73b0-b4f5-bd082241cf89)
";
        let titles = harvest_titles(index);
        assert_eq!(
            titles.len(),
            1,
            "only bare .md filenames name a memory file"
        );
        assert!(titles.contains_key("feedback_real.md"));
    }

    /// Titles carry backticks, commas, em-dashes and slashes verbatim — they are prose, and the
    /// harvest must not normalize them into something the author did not write.
    #[test]
    fn harvest_preserves_punctuation_in_the_title() {
        let index = "- [`--edge-type` takes the LABEL, filters the KIND](project_x.md)\n";
        let titles = harvest_titles(index);
        assert_eq!(
            titles.get("project_x.md").map(String::as_str),
            Some("`--edge-type` takes the LABEL, filters the KIND")
        );
    }
}
