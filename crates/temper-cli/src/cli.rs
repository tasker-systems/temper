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

/// CLI-local enum mirroring `ConfidenceBand` for clap `value_enum` parsing.
///
/// Kept in `cli.rs` (not `temper-core`) to avoid adding a `clap` dependency to `temper-core`,
/// mirroring `CliEdgeKind`/`CliPolarity`. Maps to `temper_core::types::ConfidenceBand`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CliConfidence {
    Tentative,
    Probable,
    Confident,
}

impl From<CliConfidence> for temper_core::types::ConfidenceBand {
    fn from(c: CliConfidence) -> Self {
        use temper_core::types::ConfidenceBand;
        match c {
            CliConfidence::Tentative => ConfidenceBand::Tentative,
            CliConfidence::Probable => ConfidenceBand::Probable,
            CliConfidence::Confident => ConfidenceBand::Confident,
        }
    }
}

/// Per-act agent-authorship + invocation-correlation flags shared by every authored-write CLI
/// command (resource create, edge assert/fold) via `#[command(flatten)]`. All optional and
/// available to any caller — agent-driven CLI is the *expected* case, not a restricted one.
/// `confidence` is required iff any other authorship flag is set (enforced by
/// [`temper_core::types::ActInput::into_act_context`] at the consuming surface).
#[derive(clap::Args, Debug, Clone, Default)]
pub struct ActArgs {
    /// Correlate this act with an open invocation envelope (its ref/UUID from `invocation open`).
    #[arg(long)]
    pub invocation: Option<String>,
    /// Graded authorship confidence: tentative, probable, or confident.
    #[arg(long, value_enum)]
    pub confidence: Option<CliConfidence>,
    /// Free-text reasoning for the act (authorship; requires --confidence).
    #[arg(long)]
    pub reasoning: Option<String>,
    /// Structured rationale for the act (authorship; requires --confidence).
    #[arg(long)]
    pub rationale: Option<String>,
    /// Persona/role the author acted as (authorship; requires --confidence).
    #[arg(long)]
    pub persona: Option<String>,
    /// Model that authored the act (authorship; requires --confidence).
    #[arg(long)]
    pub model: Option<String>,
}

impl ActArgs {
    /// Parse the flags into the shared [`temper_core::types::ActInput`] wire shape. The invocation
    /// ref resolves trailing-UUID-only (like every other ref). The
    /// confidence-required-iff-authorship rule is enforced downstream by `into_act_context`.
    pub fn into_act_input(
        self,
    ) -> Result<temper_core::types::ActInput, temper_core::error::TemperError> {
        let invocation_id = self
            .invocation
            .as_deref()
            .map(|r| {
                temper_workflow::operations::parse_ref(r)
                    .map(|id| temper_core::types::ids::InvocationId::from(id.0))
            })
            .transpose()?;
        Ok(temper_core::types::ActInput {
            invocation_id,
            reasoning: self.reasoning,
            confidence: self.confidence.map(Into::into),
            rationale: self.rationale,
            persona: self.persona,
            model: self.model,
        })
    }
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
        ///
        /// For Auth0/Okta this requires `--auth-domain`, `--auth-client-id`, and
        /// `--auth-audience` (validated at run time); `--idp temper-as` needs only this flag.
        #[arg(long)]
        instance_url: Option<String>,
        /// Self-host: OAuth provider domain (e.g. acme.us.auth0.com or acme.okta.com)
        #[arg(long)]
        auth_domain: Option<String>,
        /// Self-host: CLI application client_id
        #[arg(long)]
        auth_client_id: Option<String>,
        /// Self-host: API audience (e.g. <https://temper.acme.com/api>)
        #[arg(long)]
        auth_audience: Option<String>,
        /// Self-host: identity provider URL shape (default: auth0)
        #[arg(long, default_value = "auth0")]
        idp: String,
        /// Self-host: Okta authorization server ID (required with --idp okta)
        #[arg(long)]
        auth_server_id: Option<String>,
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

    /// Administer the instance (system settings, promote admins, review requests)
    Admin {
        #[command(subcommand)]
        action: AdminAction,
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
        /// Filter by context ref (UUID or @owner/slug, e.g. @me/temper or +team/general)
        #[arg(long)]
        context: Option<String>,
        /// Scope search to a single cognitive map (UUID or decorated ref). Mutually exclusive with --context.
        #[arg(long)]
        cogmap: Option<String>,
        /// Wayfind: lens-driven region-salience search across your visible maps. Mutually exclusive with --context / --cogmap.
        #[arg(long)]
        wayfind: bool,
        /// Lens ref (UUID or decorated) overriding wayfind region selection (requires --wayfind).
        #[arg(long)]
        lens: Option<String>,
        /// Top-N regions to scope into for --wayfind (default and ceiling are server-side).
        #[arg(long)]
        regions: Option<i64>,
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

    /// Operate on cognitive maps (admin-gated content reconcile)
    Cogmap {
        #[command(subcommand)]
        cmd: CogmapCmd,
    },

    /// Operate on agent-invocation envelopes (open / close / show / list)
    Invocation {
        #[command(subcommand)]
        cmd: InvocationCmd,
    },

    /// Team-self-cognition steward ingest trigger (delta / advance-watermark)
    Steward {
        #[command(subcommand)]
        cmd: StewardCmd,
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
        /// Context ref (UUID or @owner/slug, e.g. @me/temper or +team/general).
        /// Mutually exclusive with --cogmap; specify exactly one home.
        #[arg(long)]
        context: Option<String>,
        /// Cognitive-map ref (UUID or decorated `slug-<uuid>`) to home the
        /// resource in. Mutually exclusive with --context; specify exactly one.
        #[arg(long)]
        cogmap: Option<String>,
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
        /// Per-act authorship + invocation-correlation flags.
        #[command(flatten)]
        act: ActArgs,
    },
    /// List resources of a given type
    List {
        /// Resource type (task, goal, session, research, concept, decision)
        #[arg(long)]
        r#type: String,
        /// Filter by context ref (UUID or @owner/slug, e.g. @me/temper or +team/general)
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
        /// Resource ref: a UUID or the decorated `slug-<uuid>` form
        r#ref: String,
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
        /// Resource ref: a UUID or the decorated `slug-<uuid>` form
        r#ref: String,
        /// New resource type (converts the resource)
        #[arg(long)]
        type_to: Option<String>,
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
        /// Per-act authorship + invocation-correlation flags.
        #[command(flatten)]
        act: ActArgs,
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
        /// Resource ref: a UUID or the decorated `slug-<uuid>` form
        r#ref: String,
        /// Skip the local-file confirmation prompt
        #[arg(long)]
        force: bool,
        /// Per-act authorship + invocation-correlation flags.
        #[command(flatten)]
        act: ActArgs,
    },
    /// Set a facet property on a resource (cloud-mode-only API write).
    ///
    /// Sends a `POST /api/facets` request via `temper-client`.
    Facet {
        /// Resource ref: a UUID or the decorated `slug-<uuid>` form
        r#ref: String,
        /// The facet's typed value payload, as a JSON string.
        #[arg(long)]
        values: String,
        /// Facet weight (default: 1.0)
        #[arg(long)]
        weight: Option<f64>,
        /// Per-act authorship + invocation-correlation flags.
        #[command(flatten)]
        act: ActArgs,
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
        /// Owner of the context: `@me` (default) or `+<team-slug>` for a
        /// team-owned context (requires owner/maintainer on the team).
        #[arg(long)]
        owner: Option<String>,
    },
    /// List configured contexts
    List,
    /// Share a context into a team's read-reach (admin-only). The context ref is a UUID or the
    /// `@handle/slug` / `+team-slug/slug` form (from `context list`); `@me` shorthand is not accepted.
    Share {
        /// Context ref: a UUID or `@handle/slug` / `+team-slug/slug`.
        context: String,
        /// Team to share into: a team slug (optionally `+`-prefixed) or a team UUID.
        team: String,
    },
    /// Unshare a context from a team (admin-only).
    Unshare {
        /// Context ref: a UUID or `@handle/slug` / `+team-slug/slug`.
        context: String,
        /// Team to unshare: a team slug (optionally `+`-prefixed) or a team UUID.
        team: String,
    },
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
    /// Withdraw your pending join request.
    WithdrawRequest,
    /// Show a team's detail and member roster
    Show {
        /// Team slug (optionally `+`-prefixed) or UUID
        team: String,
    },
    /// Leave a team you are a member of (removes your membership)
    Leave {
        /// Team slug (optionally `+`-prefixed) or UUID
        team: String,
    },
    /// Remove a member from a team (owner/maintainer)
    RemoveMember {
        /// Team slug or UUID
        team: String,
        /// Member profile UUID
        profile: String,
    },
    /// Change a member's role (owner/maintainer)
    SetRole {
        /// Team slug or UUID
        team: String,
        /// Member profile UUID
        profile: String,
        /// New role: maintainer | member | watcher
        #[arg(long)]
        role: String,
    },
    /// Update a team's metadata (owner/maintainer)
    Update {
        /// Team slug (optionally `+`-prefixed) or UUID
        team: String,
        /// New display name
        #[arg(long)]
        name: Option<String>,
        /// New description
        #[arg(long)]
        description: Option<String>,
    },
    /// Soft-delete a team (owner only)
    Delete {
        /// Team slug (optionally `+`-prefixed) or UUID
        team: String,
    },
    /// Create a team (you become its owner)
    Create {
        /// Globally-unique team slug
        slug: String,
        /// Display name (defaults to the slug)
        #[arg(long)]
        name: Option<String>,
        /// Parent team ref (`+slug` or bare slug); creates a child team
        #[arg(long)]
        parent: Option<String>,
        /// Auto-join role for an "everyone" pool (admin-only): owner/maintainer/member/watcher
        #[arg(long = "auto-join-role")]
        auto_join_role: Option<String>,
    },
    /// Add a member to a team (owner/maintainer only)
    AddMember {
        /// Team ID (UUID)
        team: String,
        /// Profile ID (UUID)
        profile: String,
        /// Role to grant: owner/maintainer/member/watcher
        #[arg(long)]
        role: String,
    },
    /// List the teams you are a member of
    List,
}

#[derive(Subcommand)]
#[expect(
    clippy::large_enum_variant,
    reason = "clap requires arg fields inline on each variant (the Saml subcommand carries the wide Provision flag set); boxing is incompatible with the derive"
)]
pub enum AdminAction {
    /// Show system settings, or update them when any flag is provided
    Settings {
        /// Access mode: open | invite_only
        #[arg(long = "access-mode")]
        access_mode: Option<String>,
        /// Gating team slug (the team that gates invite_only access)
        #[arg(long = "gating-team")]
        gating_team_slug: Option<String>,
        /// Human-facing instance name
        #[arg(long = "instance-name")]
        instance_name: Option<String>,
        /// Terms-of-service version label
        #[arg(long = "terms-version")]
        terms_version: Option<String>,
        /// Terms-of-service resource URI
        #[arg(long = "terms-uri")]
        terms_resource_uri: Option<String>,
    },
    /// Promote a profile to admin (owner on a team; defaults to the gating team)
    Promote {
        /// Profile ID (UUID) to promote
        profile: String,
        /// Team ref (`+slug`, bare slug, or UUID); defaults to the gating team
        #[arg(long)]
        team: Option<String>,
    },
    /// Review pending join requests
    Requests {
        #[command(subcommand)]
        action: AdminRequestsAction,
    },
    /// SAML provisioning: generate keys + emit the consistent env bundle and SQL (operator tooling).
    Saml {
        #[command(subcommand)]
        action: AdminSamlAction,
    },
}

#[derive(Subcommand)]
#[expect(
    clippy::large_enum_variant,
    reason = "clap requires the ~14 Provision flags inline on the variant; boxing is incompatible with the derive"
)]
pub enum AdminSamlAction {
    /// Generate the AS signing key + reconcile secret and emit the env bundle + kb_saml_idp SQL.
    ///
    /// Interactive by default; pass --no-interactive with the flags below for scripted runs.
    /// Emits to stdout unless --env-out / --sql-out are given; --apply runs the SQL via psql.
    Provision {
        #[arg(long)]
        no_interactive: bool,
        #[arg(long)]
        instance_url: Option<String>,
        /// API origin the AS calls for reconcile (defaults to --instance-url).
        #[arg(long)]
        api_origin: Option<String>,
        #[arg(long)]
        idp_key: Option<String>,
        /// Path to the IdP signing certificate (PEM).
        #[arg(long)]
        idp_cert_file: Option<String>,
        #[arg(long)]
        idp_sso_url: Option<String>,
        #[arg(long)]
        idp_entity_id: Option<String>,
        #[arg(
            long,
            default_value = "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent"
        )]
        nameid_format: String,
        #[arg(long, default_value = "email")]
        email_attr: String,
        #[arg(long, default_value = "uid")]
        stable_id_attr: String,
        /// Assertion attribute carrying the group list (omit for authn-only).
        #[arg(long)]
        groups_attr: Option<String>,
        /// Override the signing key id (default `as-<YYYY-MM>`).
        #[arg(long)]
        kid: Option<String>,
        /// Repeatable `client_id=redirect_uri` for AS_CLIENTS (e.g. `temper-cli=https://host/api/auth/cli-callback`).
        #[arg(long = "client")]
        clients: Vec<String>,
        /// Write the env bundle here instead of stdout (chmod 0600 — contains the private key).
        #[arg(long)]
        env_out: Option<String>,
        /// Write the SQL here instead of stdout.
        #[arg(long)]
        sql_out: Option<String>,
        /// Run the kb_saml_idp SQL against $DATABASE_URL via psql (default: emit only).
        #[arg(long)]
        apply: bool,
    },
    /// Emit a kb_saml_group_mappings INSERT for `group → (+team, role)` (run AFTER teams exist).
    MapGroup {
        #[arg(long)]
        idp_key: String,
        /// The IdP-asserted group value. Required unless `--from-seen`.
        #[arg(required_unless_present = "from_seen")]
        group: Option<String>,
        /// Team to map into: a slug (optionally `+`-prefixed) or a UUID. Required unless `--from-seen`.
        #[arg(required_unless_present = "from_seen")]
        team: Option<String>,
        #[arg(long, default_value = "member")]
        role: String,
        /// Instead of emitting a mapping, list groups the IdP has actually asserted
        /// (reads kb_saml_seen_groups via psql; needs DATABASE_URL).
        #[arg(long)]
        from_seen: bool,
        /// Run the INSERT against $DATABASE_URL via psql (default: emit only).
        #[arg(long)]
        apply: bool,
    },
    /// Verify a provisioned instance: AS metadata reachable, caller is a system admin
    /// (the gating_team_slug silent-403 check), and — with --db — one active kb_saml_idp row.
    Verify {
        /// Instance base URL to probe (e.g. <https://temper.acme.com>).
        #[arg(long)]
        instance_url: String,
        /// Also check kb_saml_idp via psql (needs DATABASE_URL).
        #[arg(long)]
        db: bool,
    },
}

#[derive(Subcommand)]
pub enum AdminRequestsAction {
    /// List pending join requests for the gating team
    List,
    /// Approve or reject a join request
    Review {
        /// Join request ID (UUID)
        id: String,
        /// Approve the request
        #[arg(long, conflicts_with = "reject")]
        approve: bool,
        /// Reject the request
        #[arg(long)]
        reject: bool,
        /// Optional decision note
        #[arg(long)]
        note: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum CogmapCmd {
    /// Reconcile a cognitive map's content to a committed manifest.
    ///
    /// Reads the authored manifest, embeds each entry client-side, and PUTs a pre-embedded
    /// desired-state request to `/api/cognitive-maps/{id}` (admin-gated, idempotent).
    Reconcile {
        /// Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form
        r#ref: String,
        /// Path to the committed manifest (YAML)
        #[arg(long)]
        manifest: String,
        /// Per-act authorship + invocation-correlation flags.
        #[command(flatten)]
        act: ActArgs,
    },
    /// Genesis (create) a new cognitive map from a committed manifest.
    ///
    /// Reads the authored genesis manifest (name, telos title, optional ids + telos charter),
    /// embeds the charter client-side, and POSTs to `/api/cognitive-maps` (admin-gated, idempotent).
    /// Ids absent from the manifest are minted client-side for a stable, reproducible identity.
    Create {
        /// Path to the genesis manifest (YAML)
        #[arg(long)]
        manifest: String,
        /// Override the manifest's cogmap name
        #[arg(long)]
        name: Option<String>,
        /// Override the manifest's cogmap id (a UUID or the decorated `slug-<uuid>` form)
        #[arg(long)]
        id: Option<String>,
    },
    /// Read a cognitive map's materialized regions (surface tier).
    Shape {
        /// The cognitive map, by ref (UUID or `slug-<uuid>`).
        cogmap: String,
        /// Optional lens ref to filter regions.
        #[arg(long)]
        lens: Option<String>,
    },
    /// Read a cognitive map's per-region analytics metrics.
    RegionMetrics {
        /// The cognitive map, by ref (UUID or `slug-<uuid>`).
        cogmap: String,
        /// Optional lens ref to filter regions.
        #[arg(long)]
        lens: Option<String>,
    },
    /// Read a cognitive map's map-level analytics (telos, staleness, regulation).
    Analytics {
        /// The cognitive map, by ref (UUID or `slug-<uuid>`).
        cogmap: String,
    },
    /// Bind a cognitive map to a team (admin-only). Widens the map's reach to the
    /// team's shared resources.
    Bind {
        /// Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form.
        r#ref: String,
        /// Team to bind to: a team slug (optionally `+`-prefixed) or a team UUID.
        team: String,
    },
    /// Unbind a cognitive map from a team (admin-only).
    Unbind {
        /// Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form.
        r#ref: String,
        /// Team to unbind: a team slug (optionally `+`-prefixed) or a team UUID.
        team: String,
    },
    /// Grant a capability on a cognitive map (admin or a can_grant holder). Post-Q-A, authoring a
    /// map requires an explicit write grant, not team membership.
    Grant {
        /// Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form.
        r#ref: String,
        /// Grant to this profile (UUID). Mutually exclusive with `--to-team`.
        #[arg(long = "to-profile")]
        to_profile: Option<uuid::Uuid>,
        /// Grant to this team (UUID). Mutually exclusive with `--to-profile`.
        #[arg(long = "to-team")]
        to_team: Option<uuid::Uuid>,
        /// Grant read.
        #[arg(long)]
        read: bool,
        /// Grant write (implies read).
        #[arg(long)]
        write: bool,
        /// Grant delegated-grant authority (implies read).
        #[arg(long)]
        grant: bool,
    },
    /// Revoke a capability grant on a cognitive map (admin or a can_grant holder).
    Revoke {
        /// Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form.
        r#ref: String,
        /// Revoke this profile's grant (UUID). Mutually exclusive with `--from-team`.
        #[arg(long = "from-profile")]
        from_profile: Option<uuid::Uuid>,
        /// Revoke this team's grant (UUID). Mutually exclusive with `--from-profile`.
        #[arg(long = "from-team")]
        from_team: Option<uuid::Uuid>,
    },
}

/// CLI-local enum mirroring `Disposition` for clap `value_enum` parsing.
///
/// Kept in `cli.rs` (not `temper-core`) to avoid adding a `clap` dependency to
/// `temper-core`. Maps to `temper_core::types::invocation::Disposition` via the
/// exhaustive `to_core` method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum DispositionArg {
    Completed,
    Failed,
    Abandoned,
}

impl DispositionArg {
    /// Exhaustive map to the temper-core terminal disposition.
    pub fn to_core(self) -> temper_core::types::invocation::Disposition {
        use temper_core::types::invocation::Disposition;
        match self {
            DispositionArg::Completed => Disposition::Completed,
            DispositionArg::Failed => Disposition::Failed,
            DispositionArg::Abandoned => Disposition::Abandoned,
        }
    }
}

#[derive(Subcommand)]
pub enum InvocationCmd {
    /// Open an agent-invocation envelope. The server mints the id and returns it.
    Open {
        /// The originating cognitive map, by ref (UUID or `slug-<uuid>`).
        #[arg(long)]
        cogmap: String,
        /// Optional delegating-parent cogmap ref; omit when not spawned beneath another.
        #[arg(long)]
        parent: Option<String>,
        /// Free-form trigger label (e.g. `manual`, `delegated`, `scheduled`).
        #[arg(long = "trigger-kind")]
        trigger_kind: String,
    },
    /// Close an open envelope with a terminal disposition and optional outcome.
    Close {
        /// The invocation to close, by ref (the UUID returned by `open`).
        invocation: String,
        /// Terminal disposition: completed | failed | abandoned.
        #[arg(long, value_enum)]
        disposition: DispositionArg,
        /// Opaque, agent-defined terminal outcome as a JSON value; omit for none.
        #[arg(long)]
        outcome: Option<String>,
    },
    /// Read one envelope plus its acts by ref.
    Show {
        /// The invocation to read, by ref (UUID or `slug-<uuid>`).
        invocation: String,
    },
    /// List envelopes, optionally narrowed by originating cogmap and/or status.
    List {
        /// Optional originating cogmap ref to filter by; omit for all maps.
        #[arg(long)]
        cogmap: Option<String>,
        /// Optional lifecycle status filter: open | completed | failed | abandoned.
        #[arg(long)]
        status: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum StewardCmd {
    /// Read a team-self-cognition cogmap's ingest delta since its watermark, and whether it clears
    /// the threshold (i.e. the steward should run).
    Delta {
        /// The team-self-cognition cogmap, by ref (UUID or `slug-<uuid>`).
        cogmap: String,
        /// Ingest threshold to gate on; omit for the server default.
        #[arg(long)]
        threshold: Option<i64>,
    },
    /// Advance the ingest watermark to a given event id — the cursor a completed run records.
    AdvanceWatermark {
        /// The team-self-cognition cogmap, by ref (UUID or `slug-<uuid>`).
        cogmap: String,
        /// The `kb_events.id` (UUID) to advance the watermark to.
        event: String,
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
    /// Sends a `POST /api/relationships` request. Returns a `edge_handle`
    /// that identifies the relationship chain for subsequent retype/reweight/fold.
    Assert {
        /// Source resource ref: a UUID or the decorated `slug-<uuid>` form
        source: String,
        /// Target resource ref: a UUID or the decorated `slug-<uuid>` form
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
        /// Per-act authorship + invocation-correlation flags.
        #[command(flatten)]
        act: ActArgs,
    },
    /// Change the kind and polarity of an existing relationship.
    ///
    /// Sends `POST /api/relationships/{edge_handle}/retype`.
    Retype {
        /// Correlation ID of the relationship to retype
        edge_handle: uuid::Uuid,
        /// New edge kind
        #[arg(long, value_enum)]
        kind: CliEdgeKind,
        /// New edge polarity
        #[arg(long, value_enum)]
        polarity: CliPolarity,
        /// Per-act authorship + invocation-correlation flags.
        #[command(flatten)]
        act: ActArgs,
    },
    /// Adjust the weight of an existing relationship.
    ///
    /// Sends `POST /api/relationships/{edge_handle}/reweight`.
    Reweight {
        /// Correlation ID of the relationship to reweight
        edge_handle: uuid::Uuid,
        /// New weight value
        #[arg(long)]
        weight: f64,
        /// Per-act authorship + invocation-correlation flags.
        #[command(flatten)]
        act: ActArgs,
    },
    /// Retract (soft-delete) an existing relationship.
    ///
    /// Sends `POST /api/relationships/{edge_handle}/fold`.
    Fold {
        /// Correlation ID of the relationship to fold
        edge_handle: uuid::Uuid,
        /// Optional human-readable reason for folding
        #[arg(long)]
        reason: Option<String>,
        /// Per-act authorship + invocation-correlation flags.
        #[command(flatten)]
        act: ActArgs,
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
            "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
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
            "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "--meta-only",
            "--edges",
        ]);
        assert!(m.is_err(), "--meta-only and --edges must conflict");
    }

    #[test]
    fn show_accepts_bare_ref_and_rejects_type_flag() {
        use clap::Parser;
        // A single ref positional parses.
        assert!(Cli::try_parse_from([
            "temper",
            "resource",
            "show",
            "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
        ])
        .is_ok());
        // The removed --type flag is now an unknown arg → parse error.
        assert!(
            Cli::try_parse_from(["temper", "resource", "show", "some-ref", "--type", "task",])
                .is_err()
        );
    }

    #[test]
    fn cogmap_grant_parses_profile_write() {
        use clap::Parser;
        let id = uuid::Uuid::now_v7();
        let cli = Cli::try_parse_from([
            "temper",
            "cogmap",
            "grant",
            "map-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "--to-profile",
            &id.to_string(),
            "--write",
        ])
        .expect("cogmap grant --to-profile --write must parse");
        match cli.command {
            Commands::Cogmap {
                cmd:
                    CogmapCmd::Grant {
                        to_profile,
                        to_team,
                        write,
                        grant,
                        ..
                    },
            } => {
                assert_eq!(to_profile, Some(id));
                assert_eq!(to_team, None);
                assert!(write);
                assert!(!grant);
            }
            _ => panic!("expected Cogmap::Grant"),
        }
    }

    #[test]
    fn cogmap_revoke_parses_from_team() {
        use clap::Parser;
        let id = uuid::Uuid::now_v7();
        let cli = Cli::try_parse_from([
            "temper",
            "cogmap",
            "revoke",
            "map-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "--from-team",
            &id.to_string(),
        ])
        .expect("cogmap revoke --from-team must parse");
        match cli.command {
            Commands::Cogmap {
                cmd:
                    CogmapCmd::Revoke {
                        from_profile,
                        from_team,
                        ..
                    },
            } => {
                assert_eq!(from_team, Some(id));
                assert_eq!(from_profile, None);
            }
            _ => panic!("expected Cogmap::Revoke"),
        }
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

    #[test]
    fn map_group_from_seen_parses_without_group_team() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "temper",
            "admin",
            "saml",
            "map-group",
            "--idp-key",
            "acme-okta",
            "--from-seen",
        ])
        .expect("map-group --from-seen must parse without group/team positionals");
        match cli.command {
            Commands::Admin {
                action:
                    AdminAction::Saml {
                        action:
                            AdminSamlAction::MapGroup {
                                group,
                                team,
                                from_seen,
                                ..
                            },
                    },
            } => {
                assert!(from_seen);
                assert_eq!(group, None);
                assert_eq!(team, None);
            }
            _ => panic!("expected Admin::Saml::MapGroup"),
        }
    }

    #[test]
    fn map_group_mapping_form_requires_group_team() {
        use clap::Parser;
        // No --from-seen and no positionals → clap must reject.
        assert!(
            Cli::try_parse_from([
                "temper",
                "admin",
                "saml",
                "map-group",
                "--idp-key",
                "acme-okta"
            ])
            .is_err(),
            "map-group without --from-seen must require group and team"
        );
        // The mapping form with both positionals still parses.
        let cli = Cli::try_parse_from([
            "temper",
            "admin",
            "saml",
            "map-group",
            "--idp-key",
            "acme-okta",
            "engineers",
            "+platform",
        ])
        .expect("map-group with group+team must parse");
        match cli.command {
            Commands::Admin {
                action:
                    AdminAction::Saml {
                        action:
                            AdminSamlAction::MapGroup {
                                group,
                                team,
                                from_seen,
                                ..
                            },
                    },
            } => {
                assert!(!from_seen);
                assert_eq!(group.as_deref(), Some("engineers"));
                assert_eq!(team.as_deref(), Some("+platform"));
            }
            _ => panic!("expected Admin::Saml::MapGroup"),
        }
    }
}

#[cfg(test)]
mod invocation_parse_tests {
    use super::*;
    use clap::Parser;
    use temper_core::types::invocation::Disposition;

    const UUID: &str = "019e84ab-26ba-7560-9d34-c60d74a9fbe2";

    #[test]
    fn open_parses_into_variant() {
        let cli = Cli::try_parse_from([
            "temper",
            "invocation",
            "open",
            "--cogmap",
            UUID,
            "--trigger-kind",
            "manual",
        ])
        .expect("open should parse");
        match cli.command {
            Commands::Invocation {
                cmd:
                    InvocationCmd::Open {
                        cogmap,
                        parent,
                        trigger_kind,
                    },
            } => {
                assert_eq!(cogmap, UUID);
                assert!(parent.is_none());
                assert_eq!(trigger_kind, "manual");
            }
            _ => panic!("expected invocation open variant"),
        }
    }

    #[test]
    fn close_parses_into_variant() {
        let cli = Cli::try_parse_from([
            "temper",
            "invocation",
            "close",
            UUID,
            "--disposition",
            "completed",
        ])
        .expect("close should parse");
        match cli.command {
            Commands::Invocation {
                cmd:
                    InvocationCmd::Close {
                        invocation,
                        disposition,
                        outcome,
                    },
            } => {
                assert_eq!(invocation, UUID);
                assert_eq!(disposition, DispositionArg::Completed);
                assert_eq!(disposition.to_core(), Disposition::Completed);
                assert!(outcome.is_none());
            }
            _ => panic!("expected invocation close variant"),
        }
    }
}
