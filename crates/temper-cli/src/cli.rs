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

    /// Materialize a context's resources into the local read-only projection
    Pull {
        /// Context name to pull
        context: String,
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
        /// Body content: '@PATH' reads a file, '-' reads stdin, omit to use
        /// piped stdin implicitly (cloud mode only; ignored in local mode)
        #[arg(long)]
        body: Option<String>,
        /// Source path or URL — extract markdown via temper-ingest and use as body.
        /// Mutually exclusive with --body. URL detected by http:// or https:// prefix.
        #[arg(long, conflicts_with = "body")]
        from: Option<String>,
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
    /// Update a resource's frontmatter and/or body
    ///
    /// Mutates frontmatter from flag args. Optionally rewrites the body
    /// via `--body @<path>` (file), `--body -` (explicit stdin), or
    /// implicit non-TTY stdin (e.g. `cat new.md | temper resource update <slug>`).
    /// Works in both local and cloud mode; in local mode the file is
    /// rewritten and best-effort published; in cloud mode the body trio
    /// (content + content_hash + chunks_packed) is PATCHed in one call.
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
        /// Body source: omit (auto-detect stdin), `-` (explicit stdin), or `@<path>` (file)
        #[arg(long)]
        body: Option<String>,
    },
    /// Delete a resource (cloud-first soft-delete; local cleanup as tail in local mode)
    ///
    /// Soft-deletes the resource server-side (`is_active = false`), then in
    /// local mode removes the vault file and clears the manifest entry.
    /// In cloud mode the API call is the entire operation. API failure means
    /// no local mutation. Use `--force` to skip the local-file confirmation
    /// prompt; non-TTY callers (agents, CI) must pass `--force`.
    Delete {
        /// Resource slug
        slug: String,
        /// Resource type (task, goal, session, research, concept, decision)
        #[arg(long)]
        r#type: String,
        /// Filter by context
        #[arg(long)]
        context: Option<String>,
        /// Skip the local-file confirmation prompt
        #[arg(long)]
        force: bool,
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
    /// Store a JWT directly, reading from stdin (avoids shell-history /
    /// `ps` / `/proc` leakage). Usage:
    ///   temper auth export-token | temper auth token
    ///   pbpaste | temper auth token
    Token {
        /// Identity provider (default: auth0). Accepts `auth0` or
        /// `auth0:DOMAIN` for custom Auth0 tenants.
        #[arg(long, default_value = "auth0")]
        provider: String,
    },
    /// Clear stored credentials
    Logout,
    /// Show current auth status
    Status,
    /// Export a refreshed access token (local mode only).
    ///
    /// Token goes to stdout (plain JWT, pipeable); security warning goes to
    /// stderr. Pipe into a cloud session's secret manager as `TEMPER_TOKEN`.
    /// Token is ~24h lifetime with no early-revoke — re-export to renew.
    ExportToken,
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
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Open config.toml in $EDITOR with validate-then-save semantics
    Edit,
}
