use clap::{Parser, Subcommand};

/// CLI-local enum mirroring `EdgeKind` for clap `value_enum` parsing.
///
/// Kept in `cli.rs` (not `temper-core`) to avoid adding a `clap` dependency to
/// `temper-core`. Maps to `temper_core::types::graph::EdgeKind` at dispatch time
/// via `From<CliEdgeKind>` in `commands/edge.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CliEdgeKind {
    Express,
    Contains,
    #[value(name = "leads-to")]
    LeadsTo,
    Near,
}

/// CLI-local enum mirroring `Polarity` for clap `value_enum` parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CliPolarity {
    Forward,
    Inverse,
}

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

    /// Output format: json | toon (default: toon on a TTY, json otherwise).
    /// Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default.
    #[arg(long, global = true)]
    pub format: Option<String>,

    /// Color output: auto | always | never (default: auto).
    /// Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto.
    #[arg(long, global = true)]
    pub color: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
// clap command enums are parsed once per process at startup and never stored in
// bulk or moved on a hot path, so the variant-size disparity has no runtime cost.
// Boxing individual CLI arg fields (clippy's suggestion) hurts derive ergonomics
// and readability for no benefit.
#[expect(
    clippy::large_enum_variant,
    reason = "clap arg-definition enum, parsed once"
)]
pub enum Commands {
    /// Initialize a new vault
    Init {
        /// Path for the new vault (default: current directory)
        path: Option<String>,
        /// Skip interactive prompts
        #[arg(long)]
        no_interactive: bool,
        /// Self-host: instance base URL (e.g. <https://temper.acme.com>)
        #[arg(long, requires_all = ["auth_domain", "auth_client_id", "auth_audience"])]
        instance_url: Option<String>,
        /// Self-host: Auth0 tenant domain (e.g. acme.us.auth0.com)
        #[arg(long)]
        auth_domain: Option<String>,
        /// Self-host: Auth0 CLI application client_id
        #[arg(long)]
        auth_client_id: Option<String>,
        /// Self-host: API audience (e.g. <https://temper.acme.com/api>)
        #[arg(long)]
        auth_audience: Option<String>,
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

    /// Assert or mutate a relationship between resources (writes go through the cloud API)
    Edge {
        #[command(subcommand)]
        action: EdgeAction,
    },
}

#[derive(Subcommand)]
#[expect(
    clippy::large_enum_variant,
    reason = "clap arg-definition enum, parsed once"
)]
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
        /// Link this session to a task by slug (session only). Asserts a
        /// session→task `advances` relationship after creation.
        #[arg(long)]
        task: Option<String>,
        /// Print the raw template and exit
        #[arg(long)]
        show_template: bool,
        #[arg(long, hide = true)]
        stdin: bool,
        /// Body content: '@PATH' reads a file, '-' reads stdin, or omit to
        /// use piped stdin implicitly.
        #[arg(long)]
        body: Option<String>,
        /// Source path or URL — extract markdown via temper-ingest and use as body.
        /// Mutually exclusive with --body. URL detected by http:// or https:// prefix.
        #[arg(long, conflicts_with = "body")]
        from: Option<String>,
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
        /// Return `Vec<ResourceMetaResponse>` rows instead of
        /// `Vec<ResourceRow>` rows. Hits GET /api/resources?meta_only=true.
        #[arg(long)]
        meta_only: bool,
        /// Subselect top-level response keys on each row (anchor key
        /// always preserved). Use jq for nested projection.
        #[arg(long, value_delimiter = ',')]
        fields: Vec<String>,
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
        /// Show graph edges connected to this resource
        #[arg(long)]
        edges: bool,
        /// Return only the resource's meta tier (managed + open
        /// frontmatter, hashes); no body. Calls GET /meta endpoint.
        #[arg(long, conflicts_with = "edges")]
        meta_only: bool,
        /// Subselect top-level response keys (resource_id always
        /// preserved). Use jq for nested projection.
        #[arg(long, value_delimiter = ',')]
        fields: Vec<String>,
    },
    /// Update a resource's frontmatter and/or body
    ///
    /// Mutates frontmatter from flag args. Optionally rewrites the body via
    /// `--body @<path>` (file), `--body -` (explicit stdin), or implicit
    /// non-TTY stdin (e.g. `cat new.md | temper resource update <slug>`). The
    /// body trio (content + content_hash + chunks_packed) is PATCHed alongside
    /// any frontmatter changes in a single API call.
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
    /// Delete a resource (soft-delete via the API).
    ///
    /// Sets `is_active = false` server-side; the row is preserved. Removing a
    /// projected file from disk with `rm` is just a local cache miss and has no
    /// server effect — run `temper resource delete` to actually delete, then
    /// `temper pull <context>` to re-materialize state on a fresh device.
    /// Delete is non-interactive on all surfaces — there is no confirmation
    /// prompt. `--force` is vestigial (a no-op holdover from the pre-cloud
    /// local-mode TTY gate); it is accepted for clarity but changes nothing.
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
    /// Export a refreshed access token.
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
pub enum ConfigAction {
    /// Open config.toml in $EDITOR with validate-then-save semantics
    Edit,
}

#[derive(Subcommand, Debug)]
pub enum EdgeAction {
    /// Assert a new relationship between two resources.
    ///
    /// Sends a `POST /api/relationships` request. Returns a `correlation_id`
    /// that identifies the relationship chain for subsequent retype/reweight/fold.
    Assert {
        /// Owner of the source resource (e.g. "@me" or "+team-acme")
        #[arg(long)]
        source_owner: String,
        /// Context of the source resource
        #[arg(long)]
        source_context: String,
        /// Doc type of the source resource (e.g. "task", "goal")
        #[arg(long)]
        source_doctype: String,
        /// Slug of the source resource
        #[arg(long)]
        source_slug: String,
        /// Slug of the target resource (resolved within source's context)
        #[arg(long)]
        target: String,
        /// Edge kind (express, contains, leads-to, near)
        #[arg(long, value_enum)]
        kind: CliEdgeKind,
        /// Edge polarity (forward, inverse)
        #[arg(long, value_enum)]
        polarity: CliPolarity,
        /// Human-readable label for the relationship (e.g. "depends_on")
        #[arg(long)]
        label: String,
        /// Edge weight (default: 1.0)
        #[arg(long, default_value = "1.0")]
        weight: f64,
    },
    /// Change the kind and polarity of an existing relationship.
    ///
    /// Sends `POST /api/relationships/{correlation_id}/retype`.
    Retype {
        /// Correlation ID of the relationship to retype
        correlation_id: uuid::Uuid,
        /// New edge kind
        #[arg(long, value_enum)]
        kind: CliEdgeKind,
        /// New edge polarity
        #[arg(long, value_enum)]
        polarity: CliPolarity,
    },
    /// Adjust the weight of an existing relationship.
    ///
    /// Sends `POST /api/relationships/{correlation_id}/reweight`.
    Reweight {
        /// Correlation ID of the relationship to reweight
        correlation_id: uuid::Uuid,
        /// New weight value
        #[arg(long)]
        weight: f64,
    },
    /// Retract (soft-delete) an existing relationship.
    ///
    /// Sends `POST /api/relationships/{correlation_id}/fold`.
    Fold {
        /// Correlation ID of the relationship to fold
        correlation_id: uuid::Uuid,
        /// Optional human-readable reason for folding
        #[arg(long)]
        reason: Option<String>,
    },
}

#[cfg(test)]
mod meta_only_flag_tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn show_accepts_meta_only_and_fields() {
        let cmd = Cli::command();
        let m = cmd.try_get_matches_from([
            "temper",
            "resource",
            "show",
            "my-slug",
            "--type",
            "task",
            "--context",
            "temper",
            "--meta-only",
            "--fields",
            "managed_meta,open_meta",
        ]);
        assert!(
            m.is_ok(),
            "show with --meta-only and --fields failed to parse: {:?}",
            m.err()
        );
    }

    #[test]
    fn show_meta_only_conflicts_with_edges() {
        let cmd = Cli::command();
        let m = cmd.try_get_matches_from([
            "temper",
            "resource",
            "show",
            "my-slug",
            "--type",
            "task",
            "--meta-only",
            "--edges",
        ]);
        assert!(m.is_err(), "--meta-only and --edges must conflict");
    }

    #[test]
    fn list_accepts_meta_only_and_fields() {
        let cmd = Cli::command();
        let m = cmd.try_get_matches_from([
            "temper",
            "resource",
            "list",
            "--type",
            "task",
            "--meta-only",
            "--fields",
            "managed_meta",
        ]);
        assert!(
            m.is_ok(),
            "list with --meta-only and --fields failed: {:?}",
            m.err()
        );
    }
}
