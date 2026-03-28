use crate::config::Config;
use crate::error::Result;
use crate::hnsw::SearchIndex;
use crate::output;
use crate::registry::Registry;

pub fn run(config: &Config, verbose: bool) -> Result<()> {
    output::header("Temper Vault");
    output::label("Root", config.vault_root.display());
    output::blank();

    // File counts per essential directory
    let sessions = count_md_files(&config.sessions_dir);
    let tickets = count_md_files(&config.tickets_dir);
    let milestones = count_md_files(&config.milestones_dir);
    let templates = count_md_files(&config.templates_dir);

    output::header("Files");
    output::label("Sessions", sessions);
    output::label("Tickets", tickets);
    output::label("Milestones", milestones);
    output::label("Templates", templates);
    output::blank();

    // Index stats
    output::header("Index");
    match SearchIndex::load(&config.state_dir) {
        Ok(idx) => {
            let reg = Registry::load(&config.state_dir).ok();
            let last = reg.and_then(|r| {
                if r.last_indexed.is_empty() {
                    None
                } else {
                    Some(r.last_indexed)
                }
            });
            let chunks_str = if let Some(ref last_str) = last {
                format!(
                    "{} from {} files (last: {})",
                    idx.entry_count(),
                    idx.file_count(),
                    last_str
                )
            } else {
                format!("{} from {} files", idx.entry_count(), idx.file_count())
            };
            output::label("Chunks", chunks_str);
        }
        Err(_) => {
            output::hint("  not built — run 'temper index embed'");
        }
    }
    output::blank();

    // Projects
    output::header("Projects");
    if config.projects.is_empty() {
        output::hint("  (none configured)");
    } else {
        let mut names: Vec<&str> = config.projects.keys().map(|s| s.as_str()).collect();
        names.sort();
        for name in &names {
            let proj = &config.projects[*name];
            if verbose {
                output::plain(format!(
                    "  {} — {} ({})",
                    name,
                    proj.repo,
                    proj.path.display()
                ));
            } else {
                output::plain(format!("  {}", name));
            }
        }
    }

    Ok(())
}

pub fn count_md_files(dir: &std::path::Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                count += count_md_files(&path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                count += 1;
            }
        }
    }
    count
}
