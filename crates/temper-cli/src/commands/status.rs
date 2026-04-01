use crate::config::Config;
use crate::error::Result;
use crate::output;

pub fn run(config: &Config, _verbose: bool) -> Result<()> {
    output::header("Temper Vault");
    output::label("Root", config.vault_root.display());
    output::blank();

    // File counts across contexts
    let mut total_sessions = 0usize;
    let mut total_tasks = 0usize;
    let mut total_goals = 0usize;

    for ctx in &config.contexts {
        total_sessions += count_md_files(&config.doc_type_dir(ctx, "session"));
        total_tasks += count_md_files(&config.doc_type_dir(ctx, "task"));
        total_goals += count_md_files(&config.doc_type_dir(ctx, "goal"));
    }

    output::header("Files");
    output::label("Sessions", total_sessions);
    output::label("Tasks", total_tasks);
    output::label("Goals", total_goals);
    output::blank();

    // Contexts
    output::header("Contexts");
    if config.contexts.is_empty() {
        output::hint("  (none configured)");
    } else {
        let mut names = config.contexts.clone();
        names.sort();
        for name in &names {
            output::plain(format!("  {}", name));
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
