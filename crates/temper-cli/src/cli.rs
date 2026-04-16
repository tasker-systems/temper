use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "temper",
    about = "Developer workflow tool for agent-assisted development",
    styles = crate::output::clap_styles()
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
        #[arg(long)]
        format: Option<String>,
    },
    /// Manage resources (tasks, goals, sessions, research, concepts, decisions)
    Resource {
        #[command(subcommand)]
        action: ResourceAction,
    },
    /// Manage contexts (projects)
    Context {
        #[command(subcommand)]
        action: ContextAction,
    },
    /// Validate vault frontmatter and repair drift
    Doctor {
        #[command(subcommand)]
        action: Option<DoctorAction>,
        /// Filter by context
        #[arg(long)]
        context: Option<String>,
        /// Output format (pretty, no-tty, json — auto-detected from TTY by default)
        #[arg(long)]
        format: Option<String>,
    },
    /// Context primer for new sessions
    Warmup {
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        format: Option<String>,
    },
    /// Manage Claude Code skill
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// Authenticate with temper cloud
    #[command(name = "auth")]
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },

    /// Manage team membership and access
    Team {
        #[command(subcommand)]
        action: TeamAction,
    },

    /// Add a file, URL, or directory to the vault
    Add {
        /// File path, directory path, URL, or resource UUID (for promotion)
        path: String,
        /// Add all files in a directory
        #[arg(long)]
        dir: bool,
        /// Context name (required for file imports, unless --doc-type auto)
        #[arg(long)]
        context: Option<String>,
        /// Doc type — use "auto" to read from each file's YAML frontmatter
        #[arg(long, default_value = "research")]
        doc_type: String,
        /// Output format
        #[arg(long)]
        format: Option<String>,
        /// Override size guardrails
        #[arg(long)]
        force: bool,
        /// Preview what would be added without uploading
        #[arg(long)]
        dry_run: bool,
        /// Regex pattern to exclude files (matched against filename)
        #[arg(long)]
        ignore: Option<String>,
    },

    /// Pull a resource from the cloud
    Pull {
        /// Resource UUID
        resource_id: String,
    },

    /// Remove a resource from the cloud
    Remove {
        /// Resource UUID
        resource_id: String,
        /// Skip confirmation for vault file removal
        #[arg(long)]
        force: bool,
    },

    /// Sync local vault with temper cloud
    Sync {
        #[command(subcommand)]
        action: SyncAction,
    },

    /// Manage temper global config
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Search the knowledge base
    Search {
        /// Search query text
        query: String,
        /// Filter by context name
        #[arg(long)]
        context: Option<String>,
        /// Filter by document type
        #[arg(long)]
        doc_type: Option<String>,
        /// Maximum results (default 10)
        #[arg(long)]
        limit: Option<i64>,
        /// Output format (pretty, no-tty, json — auto-detected from TTY by default)
        #[arg(long)]
        format: Option<String>,
        /// Use text-only search (no local embedding needed)
        #[arg(long)]
        text_only: bool,
        /// Explicit seed resource IDs for graph expansion (repeatable)
        #[arg(long = "seed")]
        seed_ids: Vec<uuid::Uuid>,
        /// Edge type filter for graph expansion (repeatable)
        #[arg(long = "edge-type")]
        edge_types: Vec<String>,
        /// Max hops for graph traversal (default 2, max 10)
        #[arg(long)]
        depth: Option<i32>,
        /// Disable graph expansion (enabled by default)
        #[arg(long)]
        no_graph: bool,
    },

    /// Build, inspect, or manage the knowledge graph from vault frontmatter
    Graph {
        #[command(subcommand)]
        action: GraphAction,
    },

    /// Build an HNSW vector index over the vault
    Index {
        /// Scope to a single context (default: all contexts)
        #[arg(long)]
        context: Option<String>,
        /// Force a full rebuild (delete existing index)
        #[arg(long)]
        full: bool,
    },
}

#[derive(Subcommand)]
pub enum ResourceAction {
    /// Create a new resource
    Create {
        /// Resource type (task, goal, session, research, concept, decision)
        #[arg(long)]
        r#type: String,
        /// Resource title
        #[arg(long)]
        title: Option<String>,
        /// Context name
        #[arg(long)]
        context: Option<String>,
        /// Parent goal slug (task only)
        #[arg(long)]
        goal: Option<String>,
        /// Work mode: plan or build (task only)
        #[arg(long)]
        mode: Option<String>,
        /// Work effort: small, medium, large (task only)
        #[arg(long)]
        effort: Option<String>,
        /// Override auto-generated slug (goal only)
        #[arg(long)]
        slug: Option<String>,
        /// Print the raw template and exit
        #[arg(long)]
        show_template: bool,
        #[arg(long, hide = true)]
        stdin: bool,
        /// Output format (pretty, no-tty, json — auto-detected from TTY by default)
        #[arg(long)]
        format: Option<String>,
    },
    /// List resources of a given type
    List {
        /// Resource type (task, goal, session, research, concept, decision)
        #[arg(long)]
        r#type: String,
        /// Filter by context
        #[arg(long)]
        context: Option<String>,
        /// Maximum results
        #[arg(long)]
        limit: Option<usize>,
        /// Filter by stage (task only)
        #[arg(long)]
        stage: Option<String>,
        /// Filter by goal (task only)
        #[arg(long)]
        goal: Option<String>,
        /// Filter by status (goal only)
        #[arg(long)]
        status: Option<String>,
        /// Output format (pretty, no-tty, json — auto-detected from TTY by default)
        #[arg(long)]
        format: Option<String>,
    },
    /// Show a resource's content
    Show {
        /// Resource slug
        slug: String,
        /// Resource type (task, goal, session, research, concept, decision)
        #[arg(long)]
        r#type: String,
        /// Filter by context
        #[arg(long)]
        context: Option<String>,
        /// Output format (pretty, no-tty, json — auto-detected from TTY by default)
        #[arg(long)]
        format: Option<String>,
        /// Show graph edges connected to this resource
        #[arg(long)]
        edges: bool,
    },
    /// Update a resource's frontmatter fields
    Update {
        /// Resource slug
        slug: String,
        /// Current resource type (for lookup)
        #[arg(long)]
        r#type: Option<String>,
        /// Current resource type when changing type (use with --type-to)
        #[arg(long)]
        type_from: Option<String>,
        /// New resource type (converts the resource)
        #[arg(long)]
        type_to: Option<String>,
        /// Context to search in
        #[arg(long)]
        context: Option<String>,
        /// Move resource to a new context
        #[arg(long)]
        context_to: Option<String>,
        // --- Base schema fields ---
        /// Update title
        #[arg(long)]
        title: Option<String>,
        /// Add tag (repeatable)
        #[arg(long)]
        tags: Vec<String>,
        /// Add alias (repeatable)
        #[arg(long)]
        aliases: Vec<String>,
        /// Add relates-to reference (repeatable)
        #[arg(long)]
        relates_to: Vec<String>,
        /// Add reference (repeatable)
        #[arg(long)]
        references: Vec<String>,
        /// Add depends-on reference (repeatable)
        #[arg(long)]
        depends_on: Vec<String>,
        /// Set extends reference (repeatable)
        #[arg(long)]
        extends: Vec<String>,
        /// Set preceded-by reference (repeatable)
        #[arg(long)]
        preceded_by: Vec<String>,
        /// Set derived-from reference (repeatable)
        #[arg(long)]
        derived_from: Vec<String>,
        // --- Task-specific fields ---
        /// Task stage (backlog, in-progress, done, cancelled)
        #[arg(long)]
        stage: Option<String>,
        /// Task mode (plan, build)
        #[arg(long)]
        mode: Option<String>,
        /// Task effort (small, medium, large)
        #[arg(long)]
        effort: Option<String>,
        /// Task goal slug
        #[arg(long)]
        goal: Option<String>,
        /// Task sequence number
        #[arg(long)]
        seq: Option<i64>,
        /// Git branch
        #[arg(long)]
        branch: Option<String>,
        /// Pull request URL
        #[arg(long)]
        pr: Option<String>,
        // --- Goal-specific fields ---
        /// Goal status (active, completed, paused, cancelled)
        #[arg(long)]
        status: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ContextAction {
    /// Add a context to subscriptions
    Add {
        /// Context name to add
        name: String,
    },
    /// Remove a context from subscriptions
    Remove { name: String },
    /// Create a new context on the server
    Create {
        /// Context name to create
        name: String,
    },
    /// List configured contexts
    List,
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
pub enum TeamAction {
    /// Request to join a team (defaults to system access)
    Join {
        /// Team slug (default: system gating team)
        #[arg(long)]
        team: Option<String>,
        /// Message for the admin reviewing your request
        #[arg(long)]
        message: Option<String>,
    },
    /// Check your request or membership status
    Status {
        /// Team slug (default: system gating team)
        #[arg(long)]
        team: Option<String>,
    },
    /// Withdraw a pending request or leave a team
    Leave {
        /// Team slug (default: system gating team)
        #[arg(long)]
        team: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum SkillAction {
    /// Generate skill content (preview to stdout)
    Generate,
    /// Install skill directory and command wrapper
    Install {
        /// Override install directory (default: ~/.claude/skills/temper)
        #[arg(long)]
        path: Option<String>,
    },
    /// Check skill status
    Check,
}

#[derive(Subcommand)]
pub enum SyncAction {
    /// Run a full sync cycle
    Run {
        /// Context names to sync (default: all configured)
        #[arg(long)]
        context: Vec<String>,
        /// Output format
        #[arg(long)]
        format: Option<String>,
    },
    /// Show sync status without making changes
    Status {
        /// Context names to check
        #[arg(long)]
        context: Vec<String>,
        /// Output format
        #[arg(long)]
        format: Option<String>,
    },
    /// Refresh manifest from server (non-destructive interleave)
    Refresh {
        /// Output format
        #[arg(long)]
        format: Option<String>,
    },
    /// Reset manifest from scratch (backup + full rebuild)
    Reset {
        /// Output format
        #[arg(long)]
        format: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Open config.toml in $EDITOR with validate-then-save semantics
    Edit,
}

#[derive(Subcommand)]
pub enum DoctorAction {
    /// Auto-fix issues (rename legacy fields, backfill missing fields)
    Fix {
        /// Preview fixes without writing (dry run)
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum GraphAction {
    /// Seed the vault with graph relationships discovered from markdown bodies
    Build {
        /// Scope to a single context (default: all contexts)
        #[arg(long)]
        context: Option<String>,
        /// Preview changes without writing files
        #[arg(long)]
        dry_run: bool,
        /// Include per-file edge detail in the report
        #[arg(short, long)]
        verbose: bool,
    },
}
