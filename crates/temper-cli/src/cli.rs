use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "temper",
    about = "Developer workflow tool for agent-assisted development",
    styles = temper_cli::output::clap_styles()
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
        context: Option<String>,
        #[arg(long, default_value = "20")]
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
    /// Manage tasks
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Manage goals
    Goal {
        #[command(subcommand)]
        action: GoalAction,
    },
    /// Manage contexts (projects)
    Context {
        #[command(subcommand)]
        action: ContextAction,
    },
    /// Normalize vault structure and repair drift
    Normalize {
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        fix_slugs: bool,
    },
    /// Context primer for new sessions
    Warmup {
        #[arg(long)]
        context: Option<String>,
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Manage Claude Code skill
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// Research notes
    Research {
        #[command(subcommand)]
        action: ResearchAction,
    },
    /// Authenticate with temper cloud
    #[command(name = "auth")]
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
}

#[derive(Subcommand)]
pub enum NoteAction {
    /// Create a new note from template
    Create {
        #[arg(value_name = "TYPE", required_unless_present = "show_template")]
        note_type: Option<String>,
        #[arg(required_unless_present = "show_template")]
        title: Option<String>,
        #[arg(long)]
        context: Option<String>,
        #[arg(long, hide = true)]
        stdin: bool,
        /// Print the raw template and exit
        #[arg(long)]
        show_template: bool,
        #[arg(long, default_value = "text")]
        format: String,
    },
}

#[derive(Subcommand)]
pub enum TaskAction {
    /// Create a new task
    Create {
        #[arg(long, required_unless_present = "show_template")]
        title: Option<String>,
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        goal: Option<String>,
        #[arg(long)]
        mode: Option<String>,
        #[arg(long)]
        effort: Option<String>,
        #[arg(long, hide = true)]
        stdin: bool,
        /// Print the raw template and exit
        #[arg(long)]
        show_template: bool,
    },
    /// Move a task to a new stage or goal
    Move {
        slug: String,
        #[arg(long)]
        stage: Option<String>,
        #[arg(long)]
        goal: Option<String>,
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        mode: Option<String>,
        #[arg(long)]
        effort: Option<String>,
    },
    /// Mark a task as done
    Done {
        slug: String,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        pr: Option<String>,
        #[arg(long)]
        context: Option<String>,
    },
    /// List tasks
    List {
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        goal: Option<String>,
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Show a task's content
    Show {
        slug: String,
        #[arg(long)]
        context: Option<String>,
        #[arg(long, default_value = "text")]
        format: String,
    },
}

#[derive(Subcommand)]
pub enum GoalAction {
    /// Create a new goal
    Create {
        #[arg(long)]
        title: String,
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        slug: Option<String>,
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// List goals for a context
    List {
        #[arg(long)]
        context: Option<String>,
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Update goal status
    Update {
        slug: String,
        #[arg(long)]
        status: String,
        #[arg(long)]
        context: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ContextAction {
    /// Add a context (project) to temper.toml
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        path: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Remove a context from temper.toml
    Remove { name: String },
    /// List configured contexts
    List,
}

#[derive(Subcommand)]
pub enum SessionAction {
    /// Create or update today's session note
    Save {
        title: Option<String>,
        #[arg(long)]
        context: Option<String>,
        #[arg(long, hide = true)]
        stdin: bool,
        /// Print the raw template and exit
        #[arg(long)]
        show_template: bool,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// List recent sessions
    List {
        #[arg(long)]
        context: Option<String>,
        #[arg(long, default_value = "text")]
        format: String,
    },
}

#[derive(Subcommand)]
pub enum ResearchAction {
    /// Create or update a research note
    Save {
        #[arg(required_unless_present = "show_template")]
        title: Option<String>,
        #[arg(long)]
        context: Option<String>,
        #[arg(long, default_value = "text")]
        format: String,
        #[arg(long)]
        show_template: bool,
        #[arg(long, hide = true)]
        stdin: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum AuthAction {
    /// Log in via browser OAuth (PKCE flow)
    Login,
    /// Store a JWT directly (for API-only clients or manual auth)
    Token {
        /// The JWT access token
        jwt: String,
        /// Auth provider name (default: neon_auth)
        #[arg(long, default_value = "neon_auth")]
        provider: String,
    },
    /// Clear stored credentials
    Logout,
    /// Show current auth status
    Status,
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
        context: Option<String>,
        #[arg(long)]
        path: Option<String>,
    },
    /// Check skill status
    Check,
}
