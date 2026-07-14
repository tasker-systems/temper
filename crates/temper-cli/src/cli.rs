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

/// CLI-local enum mirroring `ElementKind` for clap `value_enum` parsing — the
/// element a trail belongs to. Kept in `cli.rs` (not `temper-core`) to avoid a
/// `clap` dependency there, mirroring `CliEdgeKind`/`CliPolarity`. Maps to
/// `temper_core::types::element_trail::ElementKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CliElementKind {
    Node,
    Edge,
}

impl From<CliElementKind> for temper_core::types::element_trail::ElementKind {
    fn from(k: CliElementKind) -> Self {
        use temper_core::types::element_trail::ElementKind;
        match k {
            CliElementKind::Node => ElementKind::Node,
            CliElementKind::Edge => ElementKind::Edge,
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
    /// Stitch this write into an act-grain thread shared with other writes (a bare UUID you mint).
    /// Provenance only — it never authorizes. Omit and the event self-roots.
    #[arg(long)]
    pub correlation: Option<String>,
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
        let correlation_id = self
            .correlation
            .as_deref()
            .map(|r| {
                temper_workflow::operations::parse_ref(r)
                    .map(|id| temper_core::types::ids::CorrelationId::from(id.0))
            })
            .transpose()?;
        Ok(temper_core::types::ActInput {
            invocation_id,
            correlation_id,
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
    version = env!("CARGO_PKG_VERSION"),
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

    /// ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide.
    /// Default: this machine's performance-core count (NOT its total core count —
    /// efficiency cores measurably slow the batch down).
    /// Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1.
    #[arg(long, global = true, value_name = "N")]
    pub embed_threads: Option<usize>,

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
    /// List the pending team invitations addressed to you
    Invitations,
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
        /// Restrict graph expansion to your explicit --seed ids, skipping the automatic top-N seed
        /// union (no effect unless at least one --seed is given)
        #[arg(long = "seed-only")]
        seed_only: bool,
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

    /// Read the event trail (append-only history) of a graph element — a resource
    /// node or a relationship edge.
    ///
    /// Wraps the same access-gated ledger read the web UI's trail rail uses
    /// (`GET /api/graph/elements/{kind}/{id}/trail`): a time-ordered list of the
    /// events that produced and mutated the element, each with its actor, timestamp,
    /// confidence, and replay-sufficient payload. An element you cannot read (or that
    /// does not exist) yields an empty trail, never an error.
    Trail {
        /// Which element to trail: `node` (a resource) or `edge` (a relationship).
        kind: CliElementKind,
        /// The element, by ref: a resource ref (UUID or decorated `slug-<uuid>`) for a
        /// node, or the edge's UUID for an edge. Only the trailing UUID is used.
        r#ref: String,
    },

    /// Print the CLI version, optionally with the running binary's SHA-256.
    ///
    /// `temper --version` / `-V` (injected by clap) is the terse form. This
    /// subcommand renders a typed report through the `--format json|toon`
    /// machinery; `--checksum` folds in the running binary's own SHA-256 and
    /// resolved path (self-attestation — NOT the published archive checksum).
    Version {
        /// Also compute and print the SHA-256 of the running binary.
        #[arg(long)]
        checksum: bool,
    },

    /// Self-update the CLI to the latest release (curl-script installs only).
    ///
    /// Resolves the latest published release, compares it against the running
    /// binary's compiled version, and — when newer or `--force` — invokes the
    /// embedded installer to download, checksum-verify, and atomically replace
    /// the whole install directory (binary + bundled `lib/libonnxruntime.*`),
    /// re-pointing the on-PATH symlink. Refuses on `cargo install` builds (no
    /// archive provenance). `--check` reports current-vs-latest, mutating
    /// nothing. Unix-first; Windows self-update is a follow-up.
    Update {
        /// Report current-vs-latest and exit without mutating anything (dry run).
        #[arg(long)]
        check: bool,
        /// Pin a specific release tag to install (e.g. v0.3.0), bypassing the
        /// latest-release lookup and the already-current no-op.
        #[arg(long)]
        version: Option<String>,
        /// Reinstall even when already on the latest version (repair path).
        #[arg(long)]
        force: bool,
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
        /// Work mode: plan or build (task only)
        #[arg(long)]
        mode: Option<String>,
        /// Work effort: small, medium, large (task only)
        #[arg(long)]
        effort: Option<String>,
        /// Open (caller-defined) frontmatter as a JSON object string, e.g.
        /// --open-meta '{"marker":"x","reviewed":true}'. These are the free-form
        /// "bring-your-own" fields; the closed temper-* vocabulary uses the typed
        /// flags (--mode/--effort/…). Must be a JSON object.
        #[arg(long)]
        open_meta: Option<String>,
        /// Link this resource to a goal by ref (UUID or decorated `slug-<uuid>`).
        /// Projects a live `advances`→goal edge from the new resource on create.
        #[arg(long)]
        goal: Option<String>,
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
        /// Source path or http(s) URL — extract markdown via temper-ingest and use as body.
        /// Supported formats: md/markdown, txt/text, html/htm (PDF is not built into this binary
        /// — convert to text first). Mutually exclusive with --body. A URL is detected by the
        /// http:// or https:// prefix; unlike --sources, a plain path (not a file:// URI) is used
        /// for local files.
        #[arg(long, conflicts_with = "body")]
        from: Option<String>,
        /// Provenance sources this body was distilled from — comma-separated resource
        /// refs (UUID or decorated) and/or external http/https URLs. Each becomes a
        /// block-provenance record on the resource's body block (URLs via the 'remote' kind).
        #[arg(long, value_delimiter = ',')]
        sources: Vec<String>,
        /// Also assert a `derived_from` edge from the new resource to each
        /// resource-valued `--sources` entry. Remote URLs are skipped (no edge target).
        ///
        /// Not atomic: the edges are asserted after the create commits. A failed edge
        /// warns rather than failing the command — `edge assert` is idempotent, so
        /// re-asserting is safe, while re-running a create is not.
        #[arg(long, requires = "sources")]
        sources_as_edges: bool,
        /// Suppress the `--from <url>` provenance default. By default a URL `--from` sets the
        /// resource's origin and seeds a Remote block-provenance record from it (so `create
        /// --from <url>` is citation-grade with no extra flags); `--no-source` opts out, leaving
        /// the origin empty and recording no provenance. Mutually exclusive with `--sources`.
        #[arg(long, conflicts_with = "sources")]
        no_source: bool,
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
        /// Maximum results (default 20; 50 with --meta-only). The response always
        /// carries `total` (the full match count) and `truncated`, so a capped
        /// page is self-evident. Conflicts with --all.
        #[arg(long, conflicts_with = "all")]
        limit: Option<usize>,
        /// Return ALL matching results (no page cap). Reach for this before
        /// asserting a set is complete or a resource is absent. Conflicts with --limit.
        #[arg(long)]
        all: bool,
        /// Skip the first N matching results (pagination).
        #[arg(long)]
        offset: Option<usize>,
        /// Sort as `<field>[:asc|desc]`. Fields: updated, created, title, stage,
        /// seq, context, doctype. Direction defaults per field (time/seq → desc,
        /// text → asc). Omit for the default `updated:desc`.
        #[arg(long)]
        sort: Option<String>,
        /// Filter to titles containing this substring (case-insensitive). A cheap
        /// way to narrow a large set instead of paging blind.
        #[arg(long)]
        title_contains: Option<String>,
        /// Filter by stage (task only)
        #[arg(long)]
        stage: Option<String>,
        /// Filter by goal (task only)
        #[arg(long)]
        goal: Option<String>,
        /// Filter by status (goal only)
        #[arg(long)]
        status: Option<String>,
        /// Full per-row view minus the body: each row carries both the
        /// managed and open meta tiers on top of the usual row fields
        /// (`Vec<ResourceDetail>`, vs the default `Vec<ResourceRow>` which
        /// carries neither tier). Hits GET /api/resources?meta_only=true.
        #[arg(long)]
        meta_only: bool,
        /// Subselect top-level response keys on each row (anchor key
        /// always preserved). Use jq for nested projection.
        #[arg(long, value_delimiter = ',')]
        fields: Vec<String>,
    },
    /// Describe the recognized open_meta conventions (the self-describing schema)
    ///
    /// Prints the recognized open (caller-defined) frontmatter keys, their shapes, and — via each
    /// key's description — whether it is FTS-indexed (and at what weight) or shape-only, plus the
    /// discouraged bare keys. The open tier stays free-form; this is guidance, not a closed
    /// vocabulary. Mirrors the MCP `describe_open_meta` tool.
    DescribeOpenMeta,
    /// Show a resource's content
    Show {
        /// Resource ref: a UUID or the decorated `slug-<uuid>` form
        r#ref: String,
        /// Show graph edges connected to this resource
        #[arg(long)]
        edges: bool,
        /// Show the resource's derived_from lineage — what it derives from
        /// (ancestors) and what derives from it (descendants), access-gated.
        /// Calls GET /lineage.
        #[arg(long, conflicts_with = "meta_only")]
        lineage: bool,
        /// Show itemized per-block provenance — the sources each of the
        /// resource's content blocks was distilled from. Calls GET /provenance.
        #[arg(long, conflicts_with = "meta_only")]
        provenance: bool,
        /// Show everything except the body: the full resource view
        /// (title, type, context, owner, and both the managed and open
        /// meta tiers) minus the reconstructed markdown body.
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
        /// Open (caller-defined) frontmatter as a JSON object string, e.g.
        /// --open-meta '{"marker":"x","reviewed":true}'. Merged over the
        /// repeatable open-tier flags above (explicit keys win). Free-form
        /// "bring-your-own" fields; temper-* keys use the typed flags. Must be
        /// a JSON object.
        #[arg(long)]
        open_meta: Option<String>,
        // --- Managed (temper-*) fields: a closed vocabulary; caller-defined
        //     tags/relationships are open-tier (see --tags/--relates-to above) ---
        /// Task stage (backlog, in-progress, done, cancelled)
        #[arg(long)]
        stage: Option<String>,
        /// Task mode (plan, build)
        #[arg(long)]
        mode: Option<String>,
        /// Task effort (small, medium, large)
        #[arg(long)]
        effort: Option<String>,
        /// Task sequence number
        #[arg(long)]
        seq: Option<i64>,
        /// Git branch
        #[arg(long)]
        branch: Option<String>,
        /// Pull request URL
        #[arg(long)]
        pr: Option<String>,
        /// Set (or replace) the resource's goal by ref (UUID or decorated `slug-<uuid>`).
        /// Folds any existing `advances`→goal edge and asserts the new one. Conflicts
        /// with --clear-goal.
        #[arg(long, conflicts_with = "clear_goal")]
        goal: Option<String>,
        /// Clear the resource's goal — retract its `advances`→goal edge, leaving it
        /// goal-less. Conflicts with --goal.
        #[arg(long)]
        clear_goal: bool,
        // --- Goal-specific fields ---
        /// Goal status (active, completed, paused, cancelled)
        #[arg(long)]
        status: Option<String>,
        /// Body source: omit (auto-detect stdin), `-` (explicit stdin), or `@<path>` (file)
        #[arg(long)]
        body: Option<String>,
        /// Provenance sources this body was distilled from — comma-separated resource
        /// refs (UUID or decorated) or http(s) URLs. Each becomes a block-provenance
        /// record on the addressed block. Requires a body update.
        #[arg(long, value_delimiter = ',')]
        sources: Vec<String>,
        /// Which content block the body revise + `--sources` target (a block UUID). Omit to
        /// address the resource's sole body block (the default); required to revise a resource
        /// that has more than one block. The block must belong to the resource and be non-folded.
        #[arg(long)]
        content_block: Option<uuid::Uuid>,
        /// Per-act authorship + invocation-correlation flags.
        #[command(flatten)]
        act: ActArgs,
    },
    /// Attach provenance sources to a resource's block — WITHOUT a body revise (issue #355).
    ///
    /// The annotate-only backfill: records block-provenance rows on the addressed block without
    /// re-chunking or re-embedding (body_hash and embeddings are unchanged), so a corpus imported
    /// without sources can be made citation-grade cheaply. Verify with `resource show --provenance`.
    ///
    /// Span locators ride the source URL verbatim via a URL-fragment convention — e.g.
    /// `--sources 'https://example.com/doc.md#L120-L180'` records the line range and surfaces it in
    /// `--provenance` output (no schema change; the fragment is preserved end-to-end).
    Annotate {
        /// Resource ref: a UUID or the decorated `slug-<uuid>` form
        r#ref: String,
        /// Provenance sources to attach — comma-separated resource refs (UUID or decorated) or
        /// http(s) URLs (optionally with a `#L<start>-L<end>` locator fragment). At least one
        /// required. Each becomes a block-provenance record on the addressed block.
        #[arg(long, value_delimiter = ',', required = true)]
        sources: Vec<String>,
        /// Which content block to annotate (a block UUID). Omit to address the resource's sole body
        /// block (the default); required for a resource that has more than one block. The block must
        /// belong to the resource and be non-folded.
        #[arg(long)]
        content_block: Option<uuid::Uuid>,
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
    /// Reassign a resource's owner (mis-attribution self-fix, or a team admin
    /// acting over a resource scoped to their team).
    ///
    /// Sends a `POST /api/resources/{id}/reassign` request via `temper-client`.
    Reassign {
        /// Resource ref: a UUID or the decorated `slug-<uuid>` form
        r#ref: String,
        /// Recipient profile UUID
        #[arg(long)]
        to: String,
    },
    /// Grant a capability on a resource to a profile or team (system-admin, a can_grant
    /// holder, or the resource owner).
    Grant {
        /// Resource ref: a UUID or the decorated `slug-<uuid>` form.
        r#ref: String,
        /// Grant to this profile (UUID). Mutually exclusive with `--to-team`.
        #[arg(long = "to-profile")]
        to_profile: Option<uuid::Uuid>,
        /// Grant to this team: a team slug (optionally `+`-prefixed), a decorated
        /// `slug-<uuid>` ref, or a team UUID. Mutually exclusive with `--to-profile`.
        #[arg(long = "to-team")]
        to_team: Option<String>,
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
    /// Revoke a capability grant on a resource (system-admin, a can_grant holder, or the owner).
    Revoke {
        /// Resource ref: a UUID or the decorated `slug-<uuid>` form.
        r#ref: String,
        /// Revoke this profile's grant (UUID). Mutually exclusive with `--from-team`.
        #[arg(long = "from-profile")]
        from_profile: Option<uuid::Uuid>,
        /// Revoke this team's grant: a team slug (optionally `+`-prefixed), a decorated
        /// `slug-<uuid>` ref, or a team UUID. Mutually exclusive with `--from-profile`.
        #[arg(long = "from-team")]
        from_team: Option<String>,
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
    /// Subscribe to a context locally so `temper pull` materializes it. Local
    /// config only — this does NOT create the context on the server (use
    /// `context create`) and has no server/RBAC effect.
    Subscribe {
        /// Context name to subscribe to
        name: String,
    },
    /// Unsubscribe from a context locally (drops it from the local pull set).
    /// Local config only — no server effect.
    Unsubscribe { name: String },
    /// Create a new context on the server
    Create {
        /// Context name to create
        name: String,
        /// Owner of the context: `@me` (default) or `+<team-slug>` for a
        /// team-owned context (requires owner/maintainer on the team).
        #[arg(long)]
        owner: Option<String>,
    },
    /// List the contexts you can see on the server (with owner ref + resource counts)
    List,
    /// Share a context into a team's read-reach. Requires that you administer the context
    /// (own it, or manage its owning team) AND manage the target team (owner/maintainer), OR
    /// that you are an instance administrator. The context ref is a UUID or the
    /// `@handle/slug` / `+team-slug/slug` form (from `context list`); `@me` shorthand is not accepted.
    Share {
        /// Context ref: a UUID or `@handle/slug` / `+team-slug/slug`.
        context: String,
        /// Team to share into: a team slug (optionally `+`-prefixed) or a team UUID.
        team: String,
    },
    /// Unshare a context from a team (same authority as `share`).
    Unshare {
        /// Context ref: a UUID or `@handle/slug` / `+team-slug/slug`.
        context: String,
        /// Team to unshare: a team slug (optionally `+`-prefixed) or a team UUID.
        team: String,
    },
    /// Orient in a context by its REGIONS: the distilled, region-level view of everything homed
    /// there, most salient first. The fastest way to see what a context is about without reading
    /// any single resource in it.
    ///
    /// Empty means the context has not materialized regions yet — run `context materialize`.
    Shape {
        /// Context ref: a UUID or `@me/slug` / `+team-slug/slug`.
        context: String,
        /// Optional lens ref to narrow the read; omit for all lenses.
        #[arg(long)]
        lens: Option<String>,
    },
    /// Per-region analytics for a context: centrality, content cohesion, internal tension,
    /// reference standing, telos alignment.
    #[command(name = "region-metrics")]
    RegionMetrics {
        /// Context ref: a UUID or `@me/slug` / `+team-slug/slug`.
        context: String,
        /// Optional lens ref to narrow the read; omit for all lenses.
        #[arg(long)]
        lens: Option<String>,
    },
    /// Re-form a context's regions when enough has changed since the last materialize. Below the
    /// threshold this is a safe no-op (`materialized: false`). Requires write access to the context.
    Materialize {
        /// Context ref: a UUID or `@me/slug` / `+team-slug/slug`.
        context: String,
        /// Formation-event threshold to gate on; omit for the default.
        #[arg(long)]
        threshold: Option<i64>,
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
    /// Request system access (the invite_only gate). Reviewed by an admin.
    RequestAccess {
        /// Message for the admin reviewing your request.
        #[arg(long)]
        message: Option<String>,
    },
    /// Withdraw your pending system-access request.
    WithdrawRequest,
}

#[derive(Subcommand)]
pub enum TeamAction {
    /// Accept a team invitation by its token.
    Join {
        /// Invitation token (from `temper team invite`).
        token: String,
    },
    /// Invite an email to a team (owner/maintainer).
    Invite {
        /// Team slug (optionally `+`-prefixed) or UUID.
        team: String,
        /// Email address to invite.
        email: String,
        /// Role to grant on acceptance: maintainer | member | watcher.
        #[arg(long)]
        role: String,
    },
    /// Decline a team invitation by its token.
    Decline {
        /// Invitation token.
        token: String,
    },
    /// List pending invitations for a team (owner/maintainer).
    Invitations {
        /// Team slug (optionally `+`-prefixed) or UUID.
        team: String,
    },
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
    /// Bulk-reassign a departing member's team-scoped resources (offboarding).
    ///
    /// Reassigns every resource owned by `--from` and homed in a context shared
    /// to this team, over to `--to` (who must be a team member). Owner/maintainer
    /// only. Sends a `POST /api/teams/{id}/reassign` request via `temper-client`.
    Reassign {
        /// Team slug (optionally `+`-prefixed) or UUID
        team: String,
        /// Current owner (departing) profile UUID
        #[arg(long)]
        from: String,
        /// New owner profile UUID (must be a team member)
        #[arg(long)]
        to: String,
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
    /// Register and rotate machine (client_credentials) principals
    Machine {
        #[command(subcommand)]
        action: AdminMachineAction,
    },
    /// Re-embed chunks whose vectors were produced by an older model (the drain does the work)
    ///
    /// Nothing is destroyed: a stale vector stays searchable until a fresh one replaces it. Staleness
    /// is derived, not marked, so this is idempotent and safe to re-run. Start with --dry-run.
    Reembed {
        /// Re-embed just this resource (UUID or decorated ref)
        #[arg(long)]
        resource: Option<String>,
        /// Re-embed every stale resource in this context (`@me/slug`, `+team/slug`, or UUID)
        #[arg(long)]
        context: Option<String>,
        /// Re-embed everything stale. Must be asked for by name — never the default.
        #[arg(long)]
        all: bool,
        /// Max resources to enqueue in this call (walk the index in bounded steps)
        #[arg(long)]
        limit: Option<i32>,
        /// Report what is stale without enqueuing anything
        #[arg(long = "dry-run")]
        dry_run: bool,
    },
}

#[derive(Debug, clap::Subcommand)]
pub enum AdminMachineAction {
    /// Register a machine principal: creates its agent profile, emitters, gating-team
    /// membership, and the reach you specify. Run this BEFORE the machine's first call.
    Provision {
        /// The IdP client id (Auth0 M2M application client id)
        #[arg(long = "client-id")]
        client_id: String,
        /// Human-facing label
        #[arg(long)]
        label: String,
        /// Team recorded as this machine's OWNER. Not its reach.
        #[arg(long = "owner-team")]
        owner_team: Option<String>,
        /// Team to enroll in, as `<ref>` or `<ref>:<role>` (role defaults to `member`).
        /// Repeatable. Reach is plural and never inferred from --owner-team.
        #[arg(long = "team")]
        teams: Vec<String>,
        /// Cogmap to grant, as `<ref>` or `<ref>:ro` (defaults to read+write). Repeatable.
        #[arg(long = "cogmap")]
        cogmaps: Vec<String>,
    },
    /// Point a fresh client id at an existing agent profile, preserving its authorship
    /// history. Revokes the old client unless --no-revoke-old.
    Rebind {
        /// The machine client being rotated away from (its `id`, from `list`)
        from: String,
        /// The new IdP client id
        #[arg(long = "client-id")]
        client_id: String,
        /// Label for the new registration
        #[arg(long)]
        label: String,
        /// Leave both credentials live for an overlap window
        #[arg(long = "no-revoke-old")]
        no_revoke_old: bool,
    },
    /// Issue a temper-minted machine credential (client_credentials on temper's own AS).
    /// temper mints the client id and a secret; the secret is printed once.
    Issue {
        /// Human-facing label
        #[arg(long)]
        label: String,
        /// Team recorded as this machine's OWNER. Not its reach.
        #[arg(long = "owner-team")]
        owner_team: Option<String>,
        /// Team to enroll in, as `<ref>` or `<ref>:<role>` (role defaults to `member`). Repeatable.
        #[arg(long = "team")]
        teams: Vec<String>,
        /// Cogmap to grant, as `<ref>` or `<ref>:ro` (defaults to read+write). Repeatable.
        #[arg(long = "cogmap")]
        cogmaps: Vec<String>,
    },
    /// Rotate a temper-issued secret. The previous secret stays valid for a grace window.
    RotateSecret {
        /// The machine client to rotate (its `id`, from `list`)
        id: String,
        /// Seconds the previous secret stays valid after rotation (default 86400 = 24h).
        #[arg(long = "grace", default_value_t = 86_400)]
        grace_seconds: i64,
    },
    /// List registered machine clients
    List {
        /// Include revoked clients
        #[arg(long = "include-revoked")]
        include_revoked: bool,
    },
    /// Show one machine client
    Show { id: String },
    /// Revoke a machine client. Denies authentication; grants and memberships survive.
    Revoke { id: String },
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
    /// embeds the charter client-side, and POSTs to `/api/cognitive-maps` (open to any authenticated
    /// profile; idempotent). Manifest/`--id` ids are honored only for a system-admin — a non-admin
    /// always receives a server-minted id.
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
    /// Re-materialize a cognitive map's regions when its event delta clears the threshold.
    ///
    /// Regions only exist *after* a materialize. A map below the threshold is a no-op
    /// (`materialized: false`), not an error.
    Materialize {
        /// The cognitive map, by ref (UUID or `slug-<uuid>`).
        cogmap: String,
        /// Minimum unmaterialized-event count required to trigger. Server default when omitted.
        #[arg(long)]
        threshold: Option<i64>,
    },
    /// Bind a cognitive map to a team. Requires system-admin, OR that you manage the team
    /// (owner/maintainer) AND administer the map (hold a grant on it). Widens the map's reach to
    /// the team's shared resources.
    Bind {
        /// Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form.
        r#ref: String,
        /// Team to bind to: a team slug (optionally `+`-prefixed) or a team UUID.
        team: String,
    },
    /// Unbind a cognitive map from a team (same authority as bind).
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
        /// Grant to this team: a team slug (optionally `+`-prefixed), a decorated
        /// `slug-<uuid>` ref, or a team UUID. Mutually exclusive with `--to-profile`.
        #[arg(long = "to-team")]
        to_team: Option<String>,
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
        /// Revoke this team's grant: a team slug (optionally `+`-prefixed), a decorated
        /// `slug-<uuid>` ref, or a team UUID. Mutually exclusive with `--from-profile`.
        #[arg(long = "from-team")]
        from_team: Option<String>,
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
        #[arg(
            long,
            value_enum,
            long_help = "Terminal disposition for the invocation.\n\n\
                         completed  — the run achieved its purpose\n\
                         failed     — the run errored or produced an unusable result\n\
                         abandoned  — the run was cancelled, aborted, or superseded\n\n\
                         There is no `cancelled` value: use `abandoned`."
        )]
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
    fn goal_flag_is_first_class_on_create_update_and_list() {
        use clap::Parser;
        // task 019f3d55: `--goal` is now a first-class write flag on create/update (projects a
        // live `advances`→goal edge) AND the long-standing list filter. All three must parse.
        assert!(
            Cli::try_parse_from([
                "temper",
                "resource",
                "create",
                "--type",
                "task",
                "--title",
                "T",
                "--context",
                "@me/temper",
                "--goal",
                "some-goal-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            ])
            .is_ok(),
            "--goal must be accepted on create"
        );
        assert!(
            Cli::try_parse_from([
                "temper",
                "resource",
                "update",
                "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
                "--goal",
                "some-goal-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            ])
            .is_ok(),
            "--goal must be accepted on update"
        );
        assert!(
            Cli::try_parse_from([
                "temper",
                "resource",
                "list",
                "--type",
                "task",
                "--goal",
                "some-goal-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            ])
            .is_ok(),
            "list --goal filter must remain valid"
        );
    }

    #[test]
    fn update_rejects_goal_and_clear_goal_together() {
        use clap::Parser;
        // --goal and --clear-goal are mutually exclusive (clap `conflicts_with`); supplying both
        // is a parse error, so the tri-state can never arrive ambiguous at the backend.
        assert!(
            Cli::try_parse_from([
                "temper",
                "resource",
                "update",
                "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
                "--goal",
                "some-goal-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
                "--clear-goal",
            ])
            .is_err(),
            "--goal and --clear-goal must conflict"
        );
    }

    #[test]
    fn trail_parses_node_and_edge_kinds() {
        use clap::Parser;
        let node = Cli::try_parse_from([
            "temper",
            "trail",
            "node",
            "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
        ])
        .expect("trail node <ref> must parse");
        match node.command {
            Commands::Trail { kind, r#ref } => {
                assert_eq!(kind, CliElementKind::Node);
                assert_eq!(r#ref, "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2");
            }
            _ => panic!("expected Trail"),
        }

        let edge = Cli::try_parse_from([
            "temper",
            "trail",
            "edge",
            "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
        ])
        .expect("trail edge <ref> must parse");
        match edge.command {
            Commands::Trail { kind, .. } => assert_eq!(kind, CliElementKind::Edge),
            _ => panic!("expected Trail"),
        }
    }

    #[test]
    fn trail_rejects_unknown_kind() {
        use clap::Parser;
        assert!(
            Cli::try_parse_from(["temper", "trail", "bogus", "some-ref"]).is_err(),
            "an element kind other than node|edge must be a parse error"
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
                // `--from-team` now accepts a slug/decorated/UUID ref (issue #366), carried
                // verbatim as a String and resolved client-side against the caller's teams.
                assert_eq!(from_team, Some(id.to_string()));
                assert_eq!(from_profile, None);
            }
            _ => panic!("expected Cogmap::Revoke"),
        }
    }

    #[test]
    fn cogmap_materialize_parses() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "temper",
            "cogmap",
            "materialize",
            "my-map-00000000-0000-0000-0000-000000000001",
            "--threshold",
            "25",
        ])
        .expect("cogmap materialize should parse");

        match cli.command {
            Commands::Cogmap {
                cmd: CogmapCmd::Materialize { cogmap, threshold },
            } => {
                assert_eq!(cogmap, "my-map-00000000-0000-0000-0000-000000000001");
                assert_eq!(threshold, Some(25));
            }
            _ => panic!("expected cogmap materialize"),
        }
    }

    #[test]
    fn cogmap_materialize_threshold_is_optional() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["temper", "cogmap", "materialize", "some-ref"])
            .expect("threshold is optional");
        match cli.command {
            Commands::Cogmap {
                cmd: CogmapCmd::Materialize { threshold, .. },
            } => assert_eq!(threshold, None),
            _ => panic!("expected cogmap materialize"),
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

#[cfg(test)]
mod skill_content_verb_tests {
    use super::*;

    /// Every CLI verb the installable skill content names must resolve against the clap
    /// command tree.
    ///
    /// Issue #330 was filed because `skill-content/cognitive-maps.md` told an agent that
    /// `facet_set` was "agent-surface only" when `temper resource facet` had existed all
    /// along. Prose cannot be type-checked; its referents can. If a verb is renamed or
    /// removed, this fails and points at the doc that now lies.
    #[test]
    fn every_verb_named_by_the_skill_content_resolves() {
        use clap::CommandFactory;

        // The verb paths asserted by crates/temper-cli/skill-content/*.md.
        const DOCUMENTED_VERBS: &[&[&str]] = &[
            &["resource", "create"],
            &["resource", "show"],
            &["resource", "update"],
            &["resource", "list"],
            &["resource", "facet"],
            &["resource", "grant"],
            &["resource", "revoke"],
            &["edge", "assert"],
            &["edge", "fold"],
            &["cogmap", "materialize"],
            &["cogmap", "shape"],
            &["cogmap", "analytics"],
            &["cogmap", "grant"],
            &["invocation", "open"],
            &["invocation", "close"],
            &["invocation", "show"],
            &["search"],
            &["context", "share"],
            &["skill", "generate"],
            &["skill", "install"],
            &["invitations"],
            &["team", "create"],
            &["team", "list"],
            &["team", "show"],
            &["team", "add-member"],
            &["team", "invite"],
            &["team", "join"],
            &["team", "decline"],
            &["team", "invitations"],
            &["team", "set-role"],
            &["team", "remove-member"],
            &["team", "leave"],
            &["team", "update"],
            &["team", "reassign"],
            &["team", "delete"],
        ];

        let root = Cli::command();
        for path in DOCUMENTED_VERBS {
            let mut node = &root;
            for (depth, segment) in path.iter().enumerate() {
                node = node.find_subcommand(segment).unwrap_or_else(|| {
                    panic!(
                        "skill content names `temper {}`, but `{segment}` does not resolve \
                         (depth {depth}). Either restore the verb or fix the docs.",
                        path.join(" ")
                    )
                });
            }
        }
    }
}
