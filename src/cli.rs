use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "temper", about = "Developer workflow tool for agent-assisted development")]
pub struct Cli {
    /// Path to vault (overrides TEMPER_VAULT and auto-detection)
    #[arg(long, global = true)]
    pub vault: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a new vault
    Init {
        /// Path for the new vault (default: current directory)
        path: Option<String>,
        /// Skip interactive prompts
        #[arg(long)]
        no_interactive: bool,
    },
    /// Check vault integrity and tool health
    Check {
        #[arg(long)]
        quiet: bool,
    },
    /// Show vault status overview
    Status {
        #[arg(long)]
        verbose: bool,
    },
    /// Build semantic search index
    Index {
        /// Force reindex all files
        #[arg(long)]
        force: bool,
        /// Limit scope to specific paths (comma-separated or glob)
        #[arg(long)]
        paths: Option<String>,
        /// Override configured sources for this run
        #[arg(long)]
        sources: Option<String>,
    },
    /// Search the vault
    Search {
        query: String,
        #[arg(long, default_value = "text")]
        format: String,
        /// Filter by note type
        #[arg(long, name = "type")]
        note_type: Option<String>,
        /// Filter by project
        #[arg(long)]
        project: Option<String>,
        /// Max results
        #[arg(long, default_value = "10")]
        limit: usize,
    },
    /// Show topic with related context
    Context {
        topic: String,
        /// How many related hops
        #[arg(long, default_value = "1")]
        depth: usize,
        /// Max related results per hop
        #[arg(long, default_value = "5")]
        limit: usize,
        #[arg(long, default_value = "text")]
        format: String,
    },
}
