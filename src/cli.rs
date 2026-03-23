use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "temper",
    about = "Developer workflow tool for agent-assisted development"
)]
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
    /// Show recent vault events
    Events {
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value = "20")]
        limit: usize,
        #[arg(long, default_value = "text")]
        format: String,
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
    /// Create a new note
    Note {
        #[command(subcommand)]
        action: NoteAction,
    },
    /// Session management
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Manage tickets
    Ticket {
        #[command(subcommand)]
        action: TicketAction,
    },
    /// Manage milestones
    Milestone {
        #[command(subcommand)]
        action: MilestoneAction,
    },
    /// Manage projects
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Manage Claude Code skill
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
}

#[derive(Subcommand)]
pub enum NoteAction {
    /// Create a new note from template
    Create {
        #[arg(value_name = "TYPE")]
        note_type: String,
        title: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        stdin: bool,
    },
}

#[derive(Subcommand)]
pub enum TicketAction {
    /// Create a new ticket
    Create {
        #[arg(long)]
        title: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        milestone: Option<String>,
        #[arg(long)]
        stdin: bool,
    },
    /// Move a ticket to a new stage or milestone
    Move {
        slug: String,
        #[arg(long)]
        stage: Option<String>,
        #[arg(long)]
        milestone: Option<String>,
    },
    /// Mark a ticket as done
    Done {
        slug: String,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        pr: Option<String>,
    },
    /// List tickets
    List {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        milestone: Option<String>,
    },
    /// Show a ticket's content
    Show { slug: String },
    /// Show project board
    Board {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        milestone: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum MilestoneAction {
    /// Create a new milestone
    Create {
        #[arg(long)]
        title: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        slug: Option<String>,
    },
    /// List milestones for a project
    List {
        #[arg(long)]
        project: Option<String>,
    },
    /// Update milestone status
    Update {
        slug: String,
        #[arg(long)]
        status: String,
    },
}

#[derive(Subcommand)]
pub enum ProjectAction {
    /// Add a project to temper.toml
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        path: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Remove a project from temper.toml
    Remove { name: String },
    /// List configured projects
    List,
}

#[derive(Subcommand)]
pub enum SessionAction {
    /// Create or update today's session note
    Save {
        title: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        stdin: bool,
    },
    /// List recent sessions
    List {
        #[arg(long)]
        project: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum SkillAction {
    /// Generate skill content (preview to stdout)
    Generate,
    /// Install skill file
    Install {
        #[arg(long)]
        global: bool,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        path: Option<String>,
    },
    /// Check skill status
    Check,
}
