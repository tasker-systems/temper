//! Typed write composition over the substrate mutation functions (WS6 4c live write path).
//!
//! The `DbBackend` (temper-api) calls these. Identity is resolved by **natural key** (handle /
//! entity-name / context-slug) — the same keys synthesis writes by — so no old→new id-map table is
//! needed. Each op opens one transaction and fires through the single [`crate::events::fire`] surface;
//! the connection carries the schema search_path (`public`), so the SQL functions + triggers resolve
//! their unqualified references correctly.
//!
//! Resolver SQL is runtime `sqlx::query` (not the compile-time macro) so it needs no `.sqlx` cache
//! entry — the macro cache is reserved for the substrate read/mutation queries.

use anyhow::{Context, Result};
use sqlx::PgPool;
use uuid::Uuid;

use crate::affinity::EdgeKind;
use crate::content::{prepare_block, prepare_block_from_chunks, IncomingChunk, PreparedBlock};
use crate::events::{fire, fire_with, EdgeHome, EventContext, SeedAction};
use crate::ids::{
    BlobId, BlockId, ChunkId, CogmapId, ContextId, DataArtifactId, EdgeId, EntityId, EventId,
    InvocationId, ProfileId, PropertyId, ResourceId, ShapeId,
};
use crate::payloads::{self, AnchorRef, EdgePolarity, Incorporation, ProvenanceSource};
use crate::text::slugify;
use temper_core::types::property_owner::PropertyOwner;
use temper_ingest::section::slice_sections;

// ── identity resolution (natural-key) ───────────────────────────────────────────

/// The caller's profile id resolved against the (single) schema. Post-collapse the caller's profile id
/// IS the substrate profile id — synthesis preserves profile ids verbatim (WS2), and the auth path
/// (`check_can_modify`) already binds it directly as the substrate principal — so this is an existence
/// check that returns the same id typed. Errors if no such profile exists.
pub async fn resolve_profile(pool: &PgPool, prod_profile: Uuid) -> Result<ProfileId> {
    let id = sqlx::query_scalar!("SELECT id FROM kb_profiles WHERE id = $1", prod_profile)
        .fetch_one(pool)
        .await
        .with_context(|| format!("profile {prod_profile} not found"))?;
    Ok(ProfileId::from(id))
}

/// The durable per-surface emitter entity `<handle>@<surface>` for a profile (§1b). `surface` is
/// the lowercase surface marker (`cli` / `mcp` / `web` / `sdk` — see `Surface::marker`);
/// `<handle>` is the profile's `kb_profiles.handle`. Resolves by joining through kb_profiles so the
/// actor name is handle-derived (no hardcoded literal) and needs no extra round-trip.
///
/// `fetch_one`, so a missing emitter is a hard error — there is no lazy creation. A new marker
/// therefore needs its entity provisioned (`profile_service`) *and* backfilled (a migration)
/// before any caller can send it.
pub async fn resolve_emitter(pool: &PgPool, profile: ProfileId, surface: &str) -> Result<EntityId> {
    let id = sqlx::query_scalar!(
        r#"SELECT e.id FROM kb_entities e
             JOIN kb_profiles p ON p.id = e.profile_id
            WHERE e.profile_id = $1 AND e.name = p.handle || '@' || $2"#,
        profile.uuid(),
        surface,
    )
    .fetch_one(pool)
    .await
    .with_context(|| format!("no emitter entity <handle>@{surface} for the resolved profile"))?;
    Ok(EntityId::from(id))
}

/// Home context by `(owner profile, slugify(name))` — the owner-scoped shape (§2 amendment).
///
/// **Retained for the substrate write-path integration test only** (`tests/write_path_mutations.rs`).
/// Production resolves contexts via `temper_services::services::context_service::resolve_context_ref`
/// (visibility-gated, UUID-primary). Do not introduce new callers of this function in production code.
pub async fn resolve_context(pool: &PgPool, owner: ProfileId, name: &str) -> Result<ContextId> {
    let slug = slugify(name);
    let id = sqlx::query_scalar!(
        r#"SELECT id FROM kb_contexts
            WHERE owner_table = 'kb_profiles' AND owner_id = $1 AND slug = $2"#,
        owner.uuid(),
        slug,
    )
    .fetch_one(pool)
    .await
    .with_context(|| format!("no context {slug:?} owned by the resolved profile"))?;
    Ok(ContextId::from(id))
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Begin a write transaction. The connection carries the schema search_path (`public`), so the SQL
/// functions + triggers resolve their unqualified references correctly with no per-txn `SET LOCAL`.
async fn begin_scoped(pool: &PgPool) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    Ok(pool.begin().await?)
}

// ── resource writes ────────────────────────────────────────────────────────────

/// Create a resource: one body block (chunked + embedded inline) homed in `home`, then one property
/// per `(key, value)` pair. Returns the new resource id.
#[derive(Debug)]
pub struct CreateParams<'a> {
    pub title: &'a str,
    pub origin_uri: &'a str,
    pub body: &'a str,
    pub doc_type: &'a str,
    pub home: AnchorRef,
    pub owner: ProfileId,
    pub originator: ProfileId,
    pub emitter: EntityId,
    /// Managed (§7-Property-fated) + open property pairs, each fired as a
    /// [`SeedAction::PropertySet`].
    ///
    /// `[corrected — 2026-08-15]` This said `PropertyAssert`, which the loop below has not fired
    /// for as long as the current code has stood. The two are not interchangeable and the
    /// difference is the whole semantics: `PropertySet` **folds** the key's live set before
    /// inserting (one current value per key), while `PropertyAssert` **appends** (several live rows
    /// per key, which is how facets work). Found while reasoning about a projector-level duplicate
    /// on `tags` — a test written from this comment asserted through this path expecting append
    /// semantics, and did not get them.
    pub properties: &'a [(String, serde_json::Value)],
    /// Caller-supplied, already-embedded chunks. When `Some`, the body block is built from these
    /// verbatim (no server-side embed — the client did extract→chunk→embed); when `None`, the server
    /// chunks + embeds `body` itself (the fallback path). Reverses PR#71's discard-client-chunks contract.
    pub chunks: Option<Vec<IncomingChunk>>,
    /// Provenance sources the body was distilled from — applied to the resource's body block and
    /// recorded into `kb_block_provenance`. Empty for an ordinary create with no attribution.
    pub sources: Vec<Incorporation>,
    /// Owner-scoped create idempotency key (issue #581, spike rung 3). `Some(key)` claims the
    /// `(owner, key)` slot in `kb_idempotency_keys` **atomically with the create**: the first create
    /// under a key mints a resource under a server-generated id and records the mapping; a replay of
    /// the same key returns that already-committed resource instead of minting a twin. `None` ⇒ an
    /// ordinary, non-idempotent create (today's behaviour). The key is scoped to `owner`, so it is
    /// **not** an existence oracle — a foreign key is simply absent from your namespace, so you mint
    /// fresh, exactly as a read cannot distinguish nonexistent from hidden. The client never supplies
    /// a resource id; the server mints it.
    pub idempotency_key: Option<Uuid>,
}

/// How a create is initiated — the two orthogonal switches [`create_resource_with_mode`] branches on.
///
/// Deliberately **not** fields on [`CreateParams`]: neither describes the resource being created, they
/// describe how *this call* was initiated. (`CreateParams` also has 36 construction sites; these have
/// one apiece.)
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateMode {
    /// Embed **off-request** (issue #299): when the caller supplies no precomputed `chunks`, the body
    /// block is chunked but NOT embedded — its chunks land with a NULL vector, so the resource is fully
    /// FTS-searchable immediately and its embeddings are backfilled later (see
    /// [`crate::embed::embed_resource_chunks`]). Caller-supplied `chunks` (bring-your-own vectors) are
    /// honored verbatim either way — there is nothing to defer.
    pub defer: bool,
    /// This create **opens a segmented ingest**: block 0 has landed, the rest of the body has not. The
    /// resource is born `ingest_state = 'in_progress'` — excluded from list and search, still readable
    /// by `show` — and only `resource_finalize` may flip it to `complete`.
    ///
    /// `false` for every ordinary create: a one-shot create is atomic, so there is no interruption
    /// window and nothing to finalize.
    pub segmented: bool,
}

pub async fn create_resource(pool: &PgPool, p: CreateParams<'_>) -> Result<ResourceId> {
    create_resource_with(pool, p, EventContext::default()).await
}

/// [`create_resource`] under an explicit [`EventContext`] — the authored `resource_created` act is
/// stamped with the caller's authorship (→ `kb_events.metadata`) and invocation correlator
/// (→ `kb_events.invocation_id`). The property acts fired at creation stay un-stamped (out of the
/// authored-act scope). Mirrors the [`crate::events::fire`]/`fire_with` split.
pub async fn create_resource_with(
    pool: &PgPool,
    p: CreateParams<'_>,
    ctx: EventContext,
) -> Result<ResourceId> {
    create_resource_with_mode(pool, p, ctx, CreateMode::default()).await
}

/// [`create_resource_with`] with embedding **deferred** — see [`CreateMode::defer`].
pub async fn create_resource_deferred_with(
    pool: &PgPool,
    p: CreateParams<'_>,
    ctx: EventContext,
) -> Result<ResourceId> {
    let mode = CreateMode {
        defer: true,
        ..CreateMode::default()
    };
    create_resource_with_mode(pool, p, ctx, mode).await
}

/// [`create_resource_with`] under an explicit [`CreateMode`] — the entry point the **segmented begin**
/// uses (`db_backend::begin_segmented_ingest`), since it is the only caller that needs a create born
/// `in_progress`. The three fns above are the ordinary-create conveniences over this one.
pub async fn create_resource_with_mode(
    pool: &PgPool,
    p: CreateParams<'_>,
    ctx: EventContext,
    mode: CreateMode,
) -> Result<ResourceId> {
    Ok(create_resource_impl(pool, p, ctx, mode).await?.0)
}

/// Like [`create_resource_with_mode`], but also reports whether the create was an **idempotent
/// replay** (the `(owner, idempotency_key)` slot was already claimed, so the already-committed
/// resource is returned) versus a fresh mint. The backend needs this to skip the post-create work
/// (goal edge, region clocks, standing) on a replay. `false` whenever `p.idempotency_key` is `None`.
pub async fn create_resource_with_mode_idempotent(
    pool: &PgPool,
    p: CreateParams<'_>,
    ctx: EventContext,
    mode: CreateMode,
) -> Result<(ResourceId, bool)> {
    create_resource_impl(pool, p, ctx, mode).await
}

/// The verbatim bytes to store for a body block: `Some(body)` for real prose, `None` for an EMPTY body.
///
/// An empty body is a "no whole-body prose — reblock from chunks" sentinel: the cognitive-map reconcile
/// path (`db_backend::apply_resource_phase`) passes `body: ""` with real `chunks`, because a distilled
/// map node's content lives in its chunks, not a whole-body string. Storing `Some("")` would write an
/// empty `kb_block_content` row and mark the resource `body_storage = 'verbatim'` — a byte-exact
/// guarantee over ZERO bytes, which PR 4's coverage-verified readback would then surface as an empty
/// body. `None` ⇒ no bytes stored ⇒ honestly `derived` ⇒ readback falls to chunk reconstruction.
fn raw_body(body: &str) -> Option<String> {
    (!body.is_empty()).then(|| body.to_owned())
}

/// Shared create body. `mode.defer` only affects the no-precomputed-chunks case: `false` embeds inline
/// (`prepare_block`), `true` chunks without embedding (`prepare_block_deferred`). `mode.segmented` only
/// rides the event payload. Everything else — sources, event fan-out, properties — is identical.
async fn create_resource_impl(
    pool: &PgPool,
    p: CreateParams<'_>,
    ctx: EventContext,
    mode: CreateMode,
) -> Result<(ResourceId, bool)> {
    let mut tx = begin_scoped(pool).await?;

    // Owner-scoped idempotency claim (issue #581, spike rung 3) — atomic with the create below. When a
    // key is supplied, claim the `(owner, key)` slot under a server-minted candidate id; a conflict
    // means a prior committed create already took it, so return THAT resource without minting a twin —
    // and without the block prep / inline embed the winning path does, which is why this runs first.
    // The candidate id we record here is the id the create mints under, so a replay converges on it.
    // No key ⇒ an ordinary create (mint a fresh id in `fire_with`).
    let mint_id: Option<ResourceId> = match p.idempotency_key {
        None => None,
        Some(key) => {
            let candidate = Uuid::now_v7();
            let claimed: Option<Uuid> = sqlx::query_scalar!(
                "INSERT INTO kb_idempotency_keys (owner_profile_id, idempotency_key, resource_id) \
                 VALUES ($1, $2, $3) \
                 ON CONFLICT (owner_profile_id, idempotency_key) DO NOTHING \
                 RETURNING resource_id",
                p.owner.uuid(),
                key,
                candidate,
            )
            .fetch_optional(&mut *tx)
            .await?;
            match claimed {
                Some(_) => Some(ResourceId::from(candidate)),
                None => {
                    // Replay: the slot is already claimed by a committed create. Read its resource id
                    // and return it — no twin, no post-create work. Scoped to `owner`, so this can
                    // only ever be the caller's own prior create.
                    let existing: Uuid = sqlx::query_scalar!(
                        "SELECT resource_id FROM kb_idempotency_keys \
                          WHERE owner_profile_id = $1 AND idempotency_key = $2",
                        p.owner.uuid(),
                        key,
                    )
                    .fetch_one(&mut *tx)
                    .await?;
                    tx.commit().await?;
                    return Ok((ResourceId::from(existing), true));
                }
            }
        }
    };

    let mut block = match (p.chunks, mode.defer) {
        (Some(chunks), _) => prepare_block_from_chunks(0, None, chunks),
        (None, false) => prepare_block(0, None, p.body)?,
        (None, true) => crate::content::prepare_block_deferred(0, None, p.body),
    };
    // Resource-level sources apply to the (single) body block; carried onto the manifest → provenance.
    block.incorporated = p.sources;
    // The raw body bytes are stored verbatim (kb_block_content) — threaded here, at the call site, not
    // in the prepare helpers, because the chunks arm (every CLI create) never sees prose. Same pattern
    // as `incorporated` one line up. `raw_body` maps an empty body ⇒ `None` (see its doc).
    block.raw_text = raw_body(p.body);
    let blocks = [block];
    let new_id = fire_with(
        &mut tx,
        SeedAction::ResourceCreate {
            title: p.title,
            origin_uri: p.origin_uri,
            // Mint under the server-generated candidate id recorded in the idempotency claim above (so
            // a replay converges on THIS resource), or a fresh id when no key was supplied.
            resource_id: mint_id,
            home: p.home,
            owner: p.owner,
            originator: Some(p.originator),
            blocks: &blocks,
            doc_type: Some(p.doc_type),
            emitter: p.emitter,
            segmented: mode.segmented,
        },
        ctx.clone(),
    )
    .await?
    .resource()?;
    for (key, value) in p.properties {
        fire(
            &mut tx,
            SeedAction::PropertySet {
                resource: new_id,
                key,
                value,
                weight: 1.0,
                emitter: p.emitter,
            },
        )
        .await?;
    }
    // Write-path policy application (the goal's synchronous arm): an ordinary create commits
    // already policy-partitioned. A bodyless/empty-body create has no prose to partition (and its
    // block stores no verbatim bytes, which the op refuses) — the later real-body update applies
    // policy. A segmented create is born `in_progress`; the resource is not complete yet and the
    // op refuses a partition decision over a still-arriving body — `finalize_ingest` applies
    // policy when the body lands complete.
    if !p.body.is_empty() && !mode.segmented {
        apply_blocking_policy_in_tx(&mut tx, new_id, p.emitter, ctx, Decline::Fatal).await?;
    }
    tx.commit().await?;
    Ok((new_id, false))
}

/// A partial resource update — only the fields present in the command are written.
#[derive(Debug)]
pub struct UpdateParams<'a> {
    pub resource: ResourceId,
    /// New body prose. With `content_block` unset this is a WHOLE-BODY write: ONE replace-shaped
    /// `resource_reblocked` — computed against the PRE-update incumbents — re-partitions to the
    /// new body's heading structure (kept sections survive with identity; folded incumbents'
    /// provenance redistributes under absorbed/carried; an identical rewrite with no sources is
    /// silent). With `content_block` set, the text revises that block alone (the hook then
    /// re-partitions in the same transaction, as shipped).
    pub body: Option<&'a str>,
    pub title: Option<&'a str>,
    pub origin_uri: Option<&'a str>,
    /// Property pairs to (re)assert (stage/mode/effort/doc_type + meta keys).
    pub properties: &'a [(String, serde_json::Value)],
    /// Caller-supplied, already-embedded chunks for the body revise. When `Some` (and `body` is
    /// supplied), section chunks whose content_hash matches a caller chunk ride the caller's
    /// vector + `embedded_with` declaration; unmatched chunks fall to the async-embed backfill.
    /// When `None`, the server chunks + embeds `body` (the fallback path).
    pub chunks: Option<Vec<IncomingChunk>>,
    /// Provenance sources this revision incorporated — BODY-GRAIN on the whole-body arm: each
    /// source asserts onto every section (carried on a multi-section body, direct on a
    /// single-section one), appended across events. On the per-block arm they apply to the
    /// addressed block as before. Empty for an ordinary body revise with no attribution.
    pub sources: Vec<Incorporation>,
    /// Which content block a PER-BLOCK body revise + `sources` target. `None` → whole-body
    /// semantics (see `body`). `Some(id)` → address that block explicitly (must belong to the
    /// resource and be non-folded) — the surgical hatch when a caller wants to revise one block
    /// of a partitioned resource without touching the rest.
    pub content_block: Option<Uuid>,
    /// Destination context for a move (`move_to.context_to`).
    pub rehome_to: Option<ContextId>,
    pub emitter: EntityId,
}

pub async fn update_resource(pool: &PgPool, p: UpdateParams<'_>) -> Result<()> {
    update_resource_with(pool, p, EventContext::default()).await
}

/// [`update_resource`] under an explicit [`EventContext`] — every sub-event of the update fan-out
/// (`block_mutated` / `property_set` / `resource_updated` / `resource_rehomed`) is correlated to the
/// caller's invocation (→ `kb_events.invocation_id`) and stamped with its authorship
/// (→ `kb_events.metadata`). Mirrors the [`crate::events::fire`]/`fire_with` split.
pub async fn update_resource_with(
    pool: &PgPool,
    p: UpdateParams<'_>,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    update_resource_in_tx(&mut tx, p, ctx, false).await?;
    tx.commit().await?;
    Ok(())
}

/// [`update_resource_with`] with embedding **deferred** (issue #299) — the update twin of
/// [`create_resource_deferred_with`]. When the caller supplies no precomputed `chunks`, the revised
/// body block is re-chunked but NOT embedded (its chunks land with a NULL vector); the caller enqueues
/// the backfill. A body revise makes the new chunks `is_current` and the old generation non-current,
/// so a backfill (keyed on `is_current AND embedding IS NULL`) only ever embeds the current revision —
/// create-then-quick-update supersede is automatic.
pub async fn update_resource_deferred_with(
    pool: &PgPool,
    p: UpdateParams<'_>,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    update_resource_in_tx(&mut tx, p, ctx, true).await?;
    tx.commit().await?;
    Ok(())
}

/// In-transaction variant of [`update_resource`] — fires on a caller-supplied connection (no
/// begin/commit). The body-block lookup runs on `&mut *conn` so it shares the caller's transaction.
/// `ctx` correlates every sub-event the update fires (`EventContext::default()` for an un-attributed
/// update); it is cloned per sub-event since an update fans out to several.
/// The write face's block-addressing refusals, TYPED so the surfaces render the defined
/// states instead of a 500-class bridge (the defined-dangling-state design): a folded target
/// maps to `Gone`/410 — the row persists as history — and an absent one to `NotFound`/404.
/// The discrimination predates the typing (the messages already named folded vs
/// not-belonging); the typing changes only the error's class and shape, keeping the wording.
#[derive(Debug)]
pub enum BlockAddressError {
    /// The addressed block belongs to the resource but is folded — not addressable for writes.
    Folded { op: String, block: uuid::Uuid },
    /// No such block under the addressed resource.
    NotInResource {
        op: String,
        block: uuid::Uuid,
        resource: ResourceId,
    },
}

impl std::fmt::Display for BlockAddressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Folded { op, block } => write!(
                f,
                "{op}: content block {block} is folded (folded blocks are not addressable)"
            ),
            Self::NotInResource {
                op,
                block,
                resource,
            } => write!(
                f,
                "{op}: content block {block} does not belong to resource {resource}"
            ),
        }
    }
}

impl std::error::Error for BlockAddressError {}

/// The retraction's refusal: the addressed property id is not a live row owned by the addressed
/// edge. A missing id, a foreign owner, and an already-retracted one are ONE error — the message
/// names only what the caller already sent, so no arm of the refusal discloses more than another
/// (no existence oracle over property rows). The projector's own zero-rows outcome, typed so the
/// backend can render it `NotFound` instead of the generic bridge.
#[derive(Debug)]
pub struct PropertyRetractError {
    pub property_id: uuid::Uuid,
    pub edge_id: uuid::Uuid,
}

impl std::fmt::Display for PropertyRetractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "facet_retract: property {} is not a live facet of edge {}",
            self.property_id, self.edge_id
        )
    }
}

impl std::error::Error for PropertyRetractError {}

/// Resolve which content block a body revise / annotate targets: an explicitly-addressed
/// `content_block` (validated to belong to `resource` and be non-folded), or — when `None` — the
/// resource's single non-folded body block. Shared by the update (revise) and annotate paths so both
/// address a block by the same rules. `op` names the caller in error messages ("update_resource" /
/// "annotate_resource").
async fn resolve_target_block(
    conn: &mut sqlx::PgConnection,
    resource: ResourceId,
    content_block: Option<Uuid>,
    op: &str,
) -> Result<Uuid> {
    match content_block {
        // Explicit addressing: validate the block belongs to this resource and is non-folded.
        // A null row ⇒ not-this-resource; is_folded ⇒ folded — both rejected before any write.
        Some(target) => {
            let is_folded = sqlx::query_scalar!(
                "SELECT is_folded FROM kb_content_blocks WHERE id=$1 AND resource_id=$2",
                target,
                resource.uuid(),
            )
            .fetch_optional(&mut *conn)
            .await?;
            match is_folded {
                Some(false) => Ok(target),
                Some(true) => anyhow::bail!(BlockAddressError::Folded {
                    op: op.to_owned(),
                    block: target,
                }),
                None => anyhow::bail!(BlockAddressError::NotInResource {
                    op: op.to_owned(),
                    block: target,
                    resource,
                }),
            }
        }
        // Default: resolve the resource's single non-folded body block (CONFORM scenario runner revise).
        None => {
            let block_ids = sqlx::query_scalar!(
                "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq",
                resource.uuid(),
            )
            .fetch_all(&mut *conn)
            .await?;
            match block_ids.as_slice() {
                [one] => Ok(*one),
                [] => anyhow::bail!("{op}: resource {} has no live block", resource.uuid()),
                _ => anyhow::bail!(
                    "{op}: resource {} has >1 block (pass --content-block to address one)",
                    resource.uuid()
                ),
            }
        }
    }
}

pub async fn update_resource_in_tx(
    conn: &mut sqlx::PgConnection,
    mut p: UpdateParams<'_>,
    ctx: EventContext,
    defer: bool,
) -> Result<()> {
    // A whole-body write (body set, no `content_block`) is the CLI/UI default: the new text IS
    // the resource's entire body. On a multi-block resource — the shape every multi-section
    // document lands in under the blocking policy — the whole body REPLACES the partition: the
    // first live block takes the full new chunk set with `replaces_body = true` (the projector
    // folds the siblings), and the policy hook below re-partitions in the same transaction.
    // Explicit `content_block` addressing stays per-block; an empty body stays the reconcile
    // sentinel (per-block, no prose).
    let mut body_write: Option<(BlockId, bool)> = None;
    if let Some(body) = p.body {
        if p.content_block.is_none() && !body.is_empty() {
            let live = sqlx::query!(
                r#"SELECT b.id, bc.content AS "content: Option<String>" FROM kb_content_blocks b
                   LEFT JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id
                   WHERE b.resource_id = $1 AND NOT b.is_folded ORDER BY b.seq"#,
                p.resource.uuid()
            )
            .fetch_all(&mut *conn)
            .await?;
            body_write = match live.as_slice() {
                [] => anyhow::bail!("update_resource: resource {} has no live block", p.resource),
                // A single-block whole-body write is STILL a whole-body write: it goes through
                // the replace-shaped partition like any other (the one incumbent is an
                // incumbent like any other — kept when the section matches it, folded when the
                // rewrite spans or rewrites it).
                [only] => Some((BlockId::from(only.id), true)),
                [first, ..] => {
                    // Identical whole-body rewrite with no new sources → the ledger must stay
                    // silent (the no-op clause): no block_mutate, no re-block, no hook. Bytes,
                    // not hashes — the whitespace-straddling edge (equal hashes over different
                    // bytes) is a real change and must write. Sources break the short-circuit
                    // (block_mutate's own dedup rule: a write carrying sources is never a no-op
                    // — short-circuiting would silently drop them). A sibling without stored
                    // bytes (a derived shape) makes the composed body unknowable → no
                    // short-circuit, replace proceeds.
                    let short_circuit = p.sources.is_empty()
                        && live.iter().all(|r| r.content.is_some())
                        && live
                            .iter()
                            .filter_map(|r| r.content.as_deref())
                            .collect::<String>()
                            == body;
                    if short_circuit {
                        None
                    } else {
                        Some((BlockId::from(first.id), true))
                    }
                }
            };
        } else {
            let block_id =
                resolve_target_block(&mut *conn, p.resource, p.content_block, "update_resource")
                    .await?;
            body_write = Some((BlockId::from(block_id), false));
        }
    }

    if let Some((block_id, replaces_body)) = body_write {
        let body = p
            .body
            .expect("body_write is Some only when p.body was Some");
        // Taken here — AFTER the short-circuit decision above (which reads `p.sources`) and
        // before the two exclusive body-write branches, each of which owns them; the tail moves
        // `p` whole into `finish_update`.
        let update_chunks = std::mem::take(&mut p.chunks);
        let update_sources = std::mem::take(&mut p.sources);
        if replaces_body {
            // ── The whole-body replace arm (spec 2026-09-08, the replace-shaped redistribution) ──
            // The revised text IS the resource's entire body, so ONE replace-shaped
            // `resource_reblocked` — computed against the PRE-update incumbents — replaces the
            // shipped fold-then-hook pair (mutate with `replaces_body = true`, then the policy
            // hook over one live block). Kept sections survive with their identity; folded
            // incumbents' provenance redistributes under absorbed/carried; the caller's
            // whole-body sources assert body-grain. `replaces_body` is retired from this path:
            // no event the write path fires any longer carries it (the projector arm stays for
            // replay of shipped events).
            //
            // A partition decision over a still-arriving body is a guess (the shipped op's own
            // decline — the check moves from the hook into the arm, same Fatal face).
            let ingest_state: String = sqlx::query_scalar!(
                "SELECT ingest_state FROM kb_resources WHERE id = $1",
                p.resource.uuid()
            )
            .fetch_one(&mut *conn)
            .await
            .with_context(|| format!("update_resource: resource {} not found", p.resource))?;
            if ingest_state == "in_progress" {
                anyhow::bail!(
                    "update_resource: resource {} is mid-ingest (in_progress) — a partition \
                     decision over a still-arriving body would be a guess",
                    p.resource
                );
            }
            let base = match (update_chunks, defer) {
                (Some(chunks), _) => prepare_block_from_chunks(0, None, chunks),
                (None, false) => prepare_block(0, None, body)?,
                (None, true) => crate::content::prepare_block_deferred(0, None, body),
            };
            if base.chunks.is_empty() {
                anyhow::bail!(
                    "update_resource: body produces no chunks (empty, whitespace, or headings \
                     only) — refusing to write a contentless block"
                );
            }
            let live_blocks = read_live_blocks(&mut *conn, p.resource).await?;
            let live_chunks = read_live_chunks(&mut *conn, p.resource).await?;
            let attributions = read_attributions(&mut *conn, p.resource).await?;
            let plan = compute_replace_partition(
                p.resource,
                body,
                &live_blocks,
                &live_chunks,
                &attributions,
                base.chunks,
                &update_sources,
            )?;
            if let Some(plan) = plan {
                fire_with(
                    &mut *conn,
                    SeedAction::ResourceReblock {
                        manifest: plan.manifest,
                        slices: &plan.slices,
                        chunks: &plan.chunks,
                        emitter: p.emitter,
                    },
                    ctx.clone(),
                )
                .await?
                .reblocked_event()?;
            }
            return finish_update(conn, p, ctx).await;
        }
        let mut prepared = match (update_chunks, defer) {
            (Some(chunks), _) => prepare_block_from_chunks(0, None, chunks),
            (None, false) => prepare_block(0, None, body)?,
            (None, true) => crate::content::prepare_block_deferred(0, None, body),
        };
        if prepared.chunks.is_empty() {
            anyhow::bail!(
                "update_resource: body produces no chunks (empty, whitespace, or headings only) — \
                 refusing to write a contentless block"
            );
        }
        prepared.incorporated = update_sources;
        fire_with(
            &mut *conn,
            SeedAction::BlockMutate {
                block: block_id,
                chunks: &prepared.chunks,
                // The revised block's raw bytes, stored verbatim. This is the PER-BLOCK arm
                // (`content_block` addressing, or the `Some("")` reconcile sentinel): the text
                // revises the addressed block alone. The whole-body replace took the other
                // branch above — no event on this path carries `replaces_body = true` any more.
                raw: (!body.is_empty()).then_some(body),
                incorporated: &prepared.incorporated,
                // Always false here: the write path no longer folds siblings via block_mutate.
                replaces_body,
                emitter: p.emitter,
            },
            ctx.clone(),
        )
        .await?;
        // Write-path policy application: the revise re-partitions the WHOLE resource to the body's
        // heading structure in the same transaction, so a committed update never observably
        // carries a partition that does not correspond to its new content. Guarded on non-empty:
        // the `Some("")` sentinel (raw = None, a derived rebuild) has no prose to partition and
        // stores no verbatim bytes the op could compose — it must skip, exactly as `raw_body`
        // above skips storing bytes for it.
        if !body.is_empty() {
            apply_blocking_policy_in_tx(conn, p.resource, p.emitter, ctx.clone(), Decline::Fatal)
                .await?;
        }
    }
    finish_update(conn, p, ctx).await
}

/// The tail of [`update_resource_in_tx`]: everything after the body write. Split so the
/// whole-body replace arm can return through it without duplicating the property/title/rehome
/// fires.
async fn finish_update(
    conn: &mut sqlx::PgConnection,
    p: UpdateParams<'_>,
    ctx: EventContext,
) -> Result<()> {
    for (key, value) in p.properties {
        fire_with(
            &mut *conn,
            SeedAction::PropertySet {
                resource: p.resource,
                key,
                value,
                weight: 1.0,
                emitter: p.emitter,
            },
            ctx.clone(),
        )
        .await?;
    }

    if p.title.is_some() || p.origin_uri.is_some() {
        fire_with(
            &mut *conn,
            SeedAction::ResourceUpdate {
                resource: p.resource,
                title: p.title,
                origin_uri: p.origin_uri,
                emitter: p.emitter,
            },
            ctx.clone(),
        )
        .await?;
    }

    if let Some(dest) = p.rehome_to {
        fire_with(
            &mut *conn,
            SeedAction::ResourceRehome {
                resource: p.resource,
                home: AnchorRef::context(dest),
                emitter: p.emitter,
            },
            ctx,
        )
        .await?;
    }

    Ok(())
}

/// Attach provenance sources to an existing resource's block **without a body revise** (issue #355).
///
/// The annotate-only write: it resolves the target block (the resource's sole non-folded body block,
/// or `content_block` when addressed explicitly), then fires `block_provenance_annotated` — recording
/// `kb_block_provenance` rows via the SAME `_insert_block_provenance` helper the create/revise paths
/// use, but touching NO chunks. Body hash and embeddings are unchanged; there is no re-chunk/re-embed.
/// Rejects an empty `sources` (an annotate with nothing to attribute is a caller error).
#[derive(Debug)]
pub struct AnnotateParams {
    pub resource: ResourceId,
    /// Sources to record onto the addressed block, position → accretion `seq` (as on create/update).
    /// Non-empty by contract.
    pub sources: Vec<Incorporation>,
    /// Which block to annotate. `None` → the resource's sole non-folded body block; `Some(id)` →
    /// that block explicitly (must belong to the resource and be non-folded).
    pub content_block: Option<Uuid>,
    pub emitter: EntityId,
}

/// [`annotate_block_sources_with`] under the default (un-attributed) context.
pub async fn annotate_block_sources(pool: &PgPool, p: AnnotateParams) -> Result<BlockId> {
    annotate_block_sources_with(pool, p, EventContext::default()).await
}

/// Annotate a block's provenance under an explicit [`EventContext`] — the `block_provenance_annotated`
/// act carries the caller's authorship + invocation correlator (→ `kb_events.metadata`/`invocation_id`).
pub async fn annotate_block_sources_with(
    pool: &PgPool,
    p: AnnotateParams,
    ctx: EventContext,
) -> Result<BlockId> {
    let mut tx = begin_scoped(pool).await?;
    let block = annotate_block_sources_in_tx(&mut tx, p, ctx).await?;
    tx.commit().await?;
    Ok(block)
}

/// In-transaction variant of [`annotate_block_sources`] — fires on a caller-supplied connection (no
/// begin/commit). Resolves the target block on `&mut *conn` so it shares the caller's transaction.
pub async fn annotate_block_sources_in_tx(
    conn: &mut sqlx::PgConnection,
    p: AnnotateParams,
    ctx: EventContext,
) -> Result<BlockId> {
    if p.sources.is_empty() {
        anyhow::bail!("annotate_resource: no sources to attach (annotate requires ≥1 source)");
    }
    let block_id =
        resolve_target_block(&mut *conn, p.resource, p.content_block, "annotate_resource").await?;
    fire_with(
        &mut *conn,
        SeedAction::BlockAnnotate {
            block: BlockId::from(block_id),
            incorporated: &p.sources,
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .block()
}

// ── the re-block substrate (2026-09-04) ──────────────────────────────────────

/// Re-block one resource's blocks. No options: the partition is a function of the body (heading
/// sections via `temper_ingest::section`), the kept/created/folded mapping is a function of the
/// live blocks' derived hashes, and the refusals are the design's refusal face (a derived-shape
/// resource, a still-arriving `in_progress` resource, an unreproducible chunking — decline rather
/// than guess).
#[derive(Debug)]
pub struct ReblockParams {
    pub resource: ResourceId,
    pub emitter: EntityId,
}

/// What a re-block did.
///
/// - [`ReblockOutcome::NoOp`] — the partition already matches; the ledger is indistinguishable
///   from the operation never having run.
/// - [`ReblockOutcome::Declined`] — a precondition for a trustworthy partition decision did not
///   hold (mid-ingest, no live blocks, a block without stored bytes, or a stored chunking that a
///   fresh chunking of the body does not reproduce). Returned as a VALUE, not an error, because
///   the right handling is the CALLER's: the write-path hook declines silently on finalize
///   (stranding an upload forever is worse than an unpartitioned commit) and treats a decline as
///   fatal elsewhere; a direct caller (adoption tooling) gets the typed reason to surface.
/// - [`ReblockOutcome::Reblocked`] — the manifest fired; the ledger carries the act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReblockOutcome {
    NoOp,
    Declined { reason: String },
    Reblocked { event: EventId },
}

/// One live block as the partition sees it.
#[derive(Debug, Clone)]
struct LiveBlock {
    id: Uuid,
    seq: i32,
    /// The block's DERIVED `block_body_hash` — the kept-identity key, never bytes-to-hash
    /// (chunk `content_hash` is over trimmed text; bytes are stored verbatim).
    body_hash: Option<String>,
    /// The block's verbatim bytes (from `kb_block_content` via `current_revision_id`). `None`
    /// = a derived block, which the op refuses outright.
    bytes: Option<String>,
}

/// One live current chunk, in the canonical ingest order (block seq, then chunk index).
#[derive(Debug, Clone)]
struct LiveChunk {
    id: Uuid,
    block_id: Uuid,
    content_hash: String,
}

/// One asserted attribution row on a live block (`is_corrected = false`).
#[derive(Debug, Clone)]
struct AttributionRow {
    block_id: Uuid,
    source: ProvenanceSource,
    accretion_seq: i32,
}

/// The PRE-update reads both partition computations share — the direct op and the whole-body
/// replace arm see the same live state (all incumbents are unfolded at this instant, so the
/// attribution read's `NOT b.is_folded` filter sees every incumbent's rows).
async fn read_live_blocks(
    conn: &mut sqlx::PgConnection,
    resource: ResourceId,
) -> Result<Vec<LiveBlock>> {
    Ok(sqlx::query!(
        r#"SELECT b.id, b.seq,
                  rev.block_body_hash AS "block_body_hash: Option<String>",
                  bc.content AS "content: Option<String>"
             FROM kb_content_blocks b
             LEFT JOIN kb_block_revisions rev ON rev.id = b.current_revision_id
             LEFT JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id
            WHERE b.resource_id = $1 AND NOT b.is_folded
            ORDER BY b.seq"#,
        resource.uuid()
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|r| LiveBlock {
        id: r.id,
        seq: r.seq,
        body_hash: r.block_body_hash,
        bytes: r.content,
    })
    .collect())
}

async fn read_live_chunks(
    conn: &mut sqlx::PgConnection,
    resource: ResourceId,
) -> Result<Vec<LiveChunk>> {
    Ok(sqlx::query!(
        r#"SELECT c.id, c.block_id, c.content_hash
             FROM kb_chunks c
             JOIN kb_content_blocks b ON b.id = c.block_id
            WHERE b.resource_id = $1 AND c.is_current AND NOT b.is_folded
            ORDER BY b.seq, c.chunk_index"#,
        resource.uuid()
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|r| LiveChunk {
        id: r.id,
        block_id: r.block_id,
        content_hash: r.content_hash,
    })
    .collect())
}

async fn read_attributions(
    conn: &mut sqlx::PgConnection,
    resource: ResourceId,
) -> Result<Vec<AttributionRow>> {
    sqlx::query!(
        r#"SELECT p.block_id, p.source_kind::text AS "source_kind!",
                  p.source_id, p.accretion_seq, r.uri AS "uri: Option<String>"
             FROM kb_block_provenance p
             JOIN kb_content_blocks b ON b.id = p.block_id
             LEFT JOIN kb_remote_sources r ON p.source_kind = 'remote' AND r.id = p.source_id
            WHERE b.resource_id = $1 AND NOT b.is_folded AND NOT p.is_corrected"#,
        resource.uuid()
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|r| {
        let source = match r.source_kind.as_str() {
            "remote" => ProvenanceSource::Remote(
                r.uri
                    .context("remote provenance row with no kb_remote_sources uri")?,
            ),
            "resource" => ProvenanceSource::Resource(r.source_id),
            "event" => ProvenanceSource::Event(r.source_id),
            other => anyhow::bail!("unknown provenance_source_kind {other:?}"),
        };
        Ok(AttributionRow {
            block_id: r.block_id,
            source,
            accretion_seq: r.accretion_seq,
        })
    })
    .collect()
}

/// The computed re-partition: the payload to fire plus each created block's verbatim slice
/// bytes (the `__blocks` sidecar).
#[derive(Debug)]
struct ReblockPlan {
    manifest: payloads::ResourceReblocked,
    slices: Vec<(BlockId, String)>,
}

/// Compute the re-partition of `body` over the live blocks/chunks. `Ok(None)` = no-op: the
/// partition is already the live one, so firing anything would violate the dedup contract.
///
/// Identity preservation is DERIVED-HASH identity: a section whose ordered chunk hashes merkle
/// to an incumbent block's current `block_body_hash` KEEPS that block row. Attribution rides
/// the chunk-run geometry — a folded block whose chunk run lies inside a created section is
/// absorbed (union, `carried = false`); one spanning the section boundary is split (copies,
/// `carried = true`); a kept block's rows ride along untouched and are never re-listed.
///
/// The decision is a value, not an error: a [`Partition::Declined`] names the precondition that
/// failed so the caller can handle it per its own recovery posture (see [`Decline`]).
#[derive(Debug)]
enum Partition {
    NoOp,
    Declined(String),
    Plan(ReblockPlan),
}

/// The shared first half of both partition computations: the per-section `(text, chunks)` list
/// plus the flat expected chunk sequence.
type Sections = (
    Vec<(String, Vec<temper_ingest::chunk::ChunkData>)>,
    Vec<temper_ingest::chunk::ChunkData>,
);

/// The shared first half of both partition computations: slice the body at heading boundaries,
/// fold heading-only slices into chunked neighbors, and prove byte-exactness. Returns the
/// per-section `(text, chunks)` list plus the flat expected chunk sequence, or `None` when the
/// body chunks to nothing (headings only) — for the direct op that is silence (no prose to
/// partition), for the whole-body replace arm the caller has already refused a chunkless body,
/// so reaching `None` there is a caller-contract violation.
fn partition_sections(body: &str) -> Option<Sections> {
    // The slices ARE the new partition; their concatenation must BE the body (asserted, not
    // assumed — this is the byte-exactness contract's compute-side half).
    let slices = slice_sections(body);
    let mut composed = String::new();
    for s in &slices {
        composed.push_str(&s.text);
    }
    assert_eq!(
        composed, body,
        "slice_sections must rejoin the body byte-for-byte"
    );

    // A heading-only slice — a document title above subsections (`# T` then `## S`) is the
    // common shape — produces NO chunks: chunk content excludes heading lines. Such a slice
    // cannot own a block (the projector refuses a created block with no chunks), so it folds
    // into a neighbor that carries content: leading slices prepend into the first chunked
    // section, trailing/interior ones append into the previous. The fold concatenates slice
    // bytes, so block bytes still compose to the body (re-asserted below).
    let mut sections: Vec<(String, Vec<temper_ingest::chunk::ChunkData>)> = Vec::new();
    let mut pending_leading = String::new();
    for s in &slices {
        let chunks =
            temper_ingest::chunk::chunk_markdown_with_prefix(&s.text, &s.initial_breadcrumb);
        if chunks.is_empty() {
            if sections.is_empty() {
                pending_leading.push_str(&s.text);
            } else if let Some((text, _)) = sections.last_mut() {
                text.push_str(&s.text);
            }
            continue;
        }
        let text = if sections.is_empty() && !pending_leading.is_empty() {
            let merged = pending_leading.clone() + &s.text;
            pending_leading.clear();
            merged
        } else {
            s.text.clone()
        };
        sections.push((text, chunks));
    }
    if sections.is_empty() {
        return None;
    }
    let mut composed = String::new();
    for (text, _) in &sections {
        composed.push_str(text);
    }
    assert_eq!(
        composed, body,
        "the folded sections must rejoin the body byte-for-byte"
    );
    let expected: Vec<temper_ingest::chunk::ChunkData> = sections
        .iter()
        .flat_map(|(_, chunks)| chunks.iter().cloned())
        .collect();
    Some((sections, expected))
}

fn compute_reblock_partition(
    resource: ResourceId,
    body: &str,
    live_blocks: &[LiveBlock],
    live_chunks: &[LiveChunk],
    attributions: &[AttributionRow],
) -> Result<Partition> {
    let Some((sections, expected)) = partition_sections(body) else {
        // The body chunked to nothing (headings only): there is no prose to partition and no
        // chunk set any block could own — the same no-prose judgment as the hook's
        // composable-skip. Silence: no event, no write.
        return Ok(Partition::NoOp);
    };

    if expected.len() != live_chunks.len() {
        return Ok(Partition::Declined(format!(
            "resource {resource} has {live} live chunk(s) but its body now chunks to {fresh} — \
             the stored chunking does not reproduce",
            live = live_chunks.len(),
            fresh = expected.len(),
        )));
    }
    for (i, (e, l)) in expected.iter().zip(live_chunks).enumerate() {
        if e.content_hash != l.content_hash {
            return Ok(Partition::Declined(format!(
                "resource {resource} live chunk #{i} does not match a fresh chunking of its body \
                 (hash {live_hash} vs expected {fresh_hash})",
                live_hash = l.content_hash,
                fresh_hash = e.content_hash,
            )));
        }
    }

    // Section k owns live chunk positions [start, end): sections partition the expected
    // sequence in order, and expected ≡ live positionally (just proven hash-by-hash).
    let mut section_ranges: Vec<(usize, usize)> = Vec::with_capacity(sections.len());
    let mut offset = 0usize;
    for (_, chunks) in &sections {
        section_ranges.push((offset, offset + chunks.len()));
        offset += chunks.len();
    }

    // Kept detection: derived-merkle equality against an unclaimed incumbent.
    let mut kept: Vec<payloads::ReblockKeptBlock> = Vec::new();
    let mut created: Vec<payloads::ReblockCreatedBlock> = Vec::new();
    let mut slices_out: Vec<(BlockId, String)> = Vec::new();
    let mut claimed: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    // Section k's surviving block, kept or created — the disposition map's successor names.
    let mut section_block_ids: Vec<BlockId> = Vec::with_capacity(sections.len());

    for (k, (text, _)) in sections.iter().enumerate() {
        let (start, end) = section_ranges[k];
        let run = &live_chunks[start..end];

        let merkle = temper_ingest::merkle::block_merkle(
            &run.iter()
                .map(|c| c.content_hash.clone())
                .collect::<Vec<_>>(),
        );
        // Kept requires BOTH the derived-hash match AND byte equality with the slice: chunk
        // content_hash is over TRIMMED text, so a boundary straddling edge whitespace can
        // produce equal hashes over different bytes — keeping such a row would re-compose the
        // body from the incumbent's old bytes and silently drop the separator the folded
        // neighbor carried. Byte-different incumbents fold, and the section creates over the
        // slice's verbatim bytes.
        let incumbent = live_blocks.iter().find(|b| {
            b.body_hash.as_deref() == Some(merkle.as_str())
                && !claimed.contains(&b.id)
                && b.bytes.as_deref() == Some(text.as_str())
        });
        match incumbent {
            Some(b) => {
                claimed.insert(b.id);
                section_block_ids.push(BlockId::from(b.id));
                kept.push(payloads::ReblockKeptBlock {
                    block_id: BlockId::from(b.id),
                    seq: k as i32,
                    // The shipped op shape asserts nothing NEW onto a kept row: the survivor's
                    // own provenance rides along untouched (the delta-only rule).
                    attribution: Vec::new(),
                });
            }
            None => {
                let new_id = BlockId::from(Uuid::now_v7());
                section_block_ids.push(new_id);
                created.push(payloads::ReblockCreatedBlock {
                    block_id: new_id,
                    seq: k as i32,
                    chunks: run
                        .iter()
                        .enumerate()
                        .map(|(i, c)| payloads::ChunkManifest {
                            chunk_id: c.id.into(),
                            chunk_index: i as i32,
                            content_hash: c.content_hash.clone(),
                        })
                        .collect(),
                    attribution: Vec::new(), // filled below
                });
                slices_out.push((new_id, text.clone()));
            }
        }
    }

    // Attribution delta. A folded block whose chunk run lies fully inside one created section
    // is ABSORBED: its asserted rows join that section's union, carried = false (the content IS
    // in the block). A folded or kept block whose run SPANS a created section's boundary is
    // SPLIT: the section gets a copy, carried = true. (Both arms stay within one resource's ACL
    // — the blessed split stance — so the copy needs no extra qualification here.) Kept blocks'
    // own rows are deliberately not re-listed: the survivor's provenance rides along untouched.
    let block_range: std::collections::HashMap<Uuid, (usize, usize)> = live_blocks
        .iter()
        .filter_map(|b| {
            let positions: Vec<usize> = live_chunks
                .iter()
                .enumerate()
                .filter(|(_, c)| c.block_id == b.id)
                .map(|(i, _)| i)
                .collect();
            let first = *positions.first()?;
            let last = *positions.last()?;
            Some((b.id, (first, last + 1)))
        })
        .collect();

    let mut delta: std::collections::HashMap<usize, Vec<(ProvenanceSource, i32, bool)>> =
        std::collections::HashMap::new();
    for a in attributions {
        let Some(&(start, end)) = block_range.get(&a.block_id) else {
            continue;
        };
        for (k, &(s_start, s_end)) in section_ranges.iter().enumerate() {
            let absorbed = start >= s_start && end <= s_end;
            let spans = start < s_end && end > s_start && !absorbed;
            let is_created_section = created.iter().any(|c| c.seq == k as i32);
            if absorbed {
                delta
                    .entry(k)
                    .or_default()
                    .push((a.source.clone(), a.accretion_seq, false));
            } else if spans && is_created_section {
                delta
                    .entry(k)
                    .or_default()
                    .push((a.source.clone(), a.accretion_seq, true));
            }
        }
    }
    // Dedup per section: one source asserted on two incumbents (one absorbed, one split) must
    // resolve deterministically — the direct (absorbed) union wins over a carried copy, then
    // lower accretion_seq.
    for c in &mut created {
        let k = c.seq as usize;
        if let Some(mut entries) = delta.remove(&k) {
            entries.sort_by_key(|(_, seq, carried)| (*carried, *seq));
            // ProvenanceSource is Eq, not Hash — a Vec scan is fine at attribution grain.
            let mut seen: Vec<ProvenanceSource> = Vec::new();
            entries.retain(|(source, _, _)| {
                if seen.contains(source) {
                    false
                } else {
                    seen.push(source.clone());
                    true
                }
            });
            c.attribution = entries
                .into_iter()
                .map(|(source, seq, carried)| payloads::ReblockAttribution {
                    source,
                    seq,
                    carried,
                })
                .collect();
        }
    }

    let folded: Vec<BlockId> = live_blocks
        .iter()
        .filter(|b| !claimed.contains(&b.id))
        .map(|b| BlockId::from(b.id))
        .collect();

    // No-op dedup: the partition is identical — everything kept in place, nothing folded,
    // nothing created (so no attribution delta exists to write). Firing would put an event on
    // the ledger for a change that never happened.
    let seqs_moved = kept.iter().any(|k| {
        live_blocks
            .iter()
            .any(|b| b.id == k.block_id.uuid() && b.seq != k.seq)
    });
    if created.is_empty() && folded.is_empty() && !seqs_moved {
        return Ok(Partition::NoOp);
    }

    // The disposition map (D-D2): where each folded incumbent's content went, captured HERE —
    // one step before the attribution delta drops incumbent identity — by the same positional
    // geometry the delta consumes. Absorbers are sections fully containing the incumbent's
    // chunk run (kept AND created); carries are sections holding a strict subset (a partial
    // overlap, kept or created — content location, not the delta's write targets). A chunkless
    // incumbent has no entry in `block_range`: no geometry, nothing locatable.
    let mut dispositions: payloads::FoldDispositions = payloads::FoldDispositions::new();
    for b in live_blocks.iter().filter(|b| !claimed.contains(&b.id)) {
        let disposition = match block_range.get(&b.id) {
            None => payloads::FoldDisposition::ContentGone,
            Some(&(start, end)) => {
                let mut absorbers = Vec::new();
                let mut carried = Vec::new();
                for (k, &(s_start, s_end)) in section_ranges.iter().enumerate() {
                    if start >= s_start && end <= s_end {
                        absorbers.push(section_block_ids[k]);
                    } else if start < s_end && end > s_start {
                        carried.push(section_block_ids[k]);
                    }
                }
                payloads::FoldDisposition::from_geometry(absorbers, carried)
            }
        };
        dispositions.insert(BlockId::from(b.id), disposition);
    }

    Ok(Partition::Plan(ReblockPlan {
        manifest: payloads::ResourceReblocked {
            resource_id: resource,
            created,
            kept,
            folded,
            replaces_body: false,
            dispositions,
        },
        slices: slices_out,
    }))
}

/// One replace-shaped partition: the manifest to fire plus each created block's verbatim slice
/// bytes and its PREPARED chunks (new ids, content, embeddings — the sidecar's chunk map).
#[derive(Debug)]
struct ReplacePlan {
    manifest: payloads::ResourceReblocked,
    slices: Vec<(BlockId, String)>,
    chunks: Vec<crate::content::PreparedChunk>,
}

/// The replace-shaped partition (whole-body write, spec 2026-09-08): sections chunk the caller's
/// NEW body and are compared against the PRE-update incumbents by CONTENT, never by ordinal —
/// the shipped op's drift check is what proves its two index spaces identical before ranges are
/// compared (`compute_reblock_partition`), and the replace shape has no such proof, so the
/// correspondence runs on chunk `content_hash` identity, the same currency kept detection runs
/// on.
///
/// - KEPT: unchanged contract — derived-merkle equality AND byte equality against an unclaimed
///   incumbent. Byteless incumbents cannot kept-match and fold (the same fold extent
///   `replaces_body` dealt them under the shipped arm).
/// - ABSORBED/CARRIED, content-grounded: an incumbent whose chunk-hash multiset is fully
///   contained in one section's is ABSORBED there (`carried = false` — the content IS in the
///   block); a section containing part of it receives a `carried = true` copy; the absorbing
///   section never also receives a copy (the shipped `!absorbed` carve-out). An incumbent
///   contained in SEVERAL duplicate sections is absorbed into each. Per section the dedup is
///   the shipped sort — absorbed beats carried, then lower accretion_seq.
/// - CONTENT-GONE (ruled, 2026-09-08): an incumbent whose hashes appear in no section describes
///   deleted content — there is no honest live target, so its rows stay history on the folded
///   row. Not redistributed is not lost.
/// - ABSORBED-INTO-KEPT: a folded duplicate's rows union onto the kept survivor (`carried =
///   false`), SKIPPING sources the survivor already holds (the union adds nothing). This arm
///   retires the shipped delta's latent kept-section drop (the shipped delta is consumed by
///   created sections only).
/// - CALLER WHOLE-BODY SOURCES are body-grain over the new chunk space: `carried` is true on a
///   multi-section body, false on a single-section one (the shipped single-block posture). They
///   APPEND across events on kept blocks — the same semantic as `block_mutate`'s incorporated
///   rows — but within one manifest each source lands once per block, deduped by the same sort.
/// - NO-OP: everything kept at unchanged seqs AND no new assertions — the exact twin of the
///   SQL entry guard, decided here (the Rust op's decision, never a second SQL opinion).
fn compute_replace_partition(
    resource: ResourceId,
    body: &str,
    live_blocks: &[LiveBlock],
    live_chunks: &[LiveChunk],
    attributions: &[AttributionRow],
    prepared: Vec<crate::content::PreparedChunk>,
    sources: &[payloads::Incorporation],
) -> Result<Option<ReplacePlan>> {
    let Some((sections, expected)) = partition_sections(body) else {
        // Unreachable through the arm (a chunkless body is refused before this runs) — a
        // headings-only body here would mean the caller contract broke.
        anyhow::bail!(
            "update_resource: body produces no chunks (empty, whitespace, or headings only) — \
             refusing to write a contentless block"
        );
    };

    // Positional expectation: the server chunker over the whole body yields exactly the flat
    // section chunking (the same equivalence the streaming segment boundary rests on). Caller-
    // supplied chunk sets may be a DIFFERENT chunking of the same body (chunker skew), so the
    // base prepared set is claimed BY HASH, never by position — a hash with no caller match
    // mints a fresh deferred chunk (content rides; the vector is backfilled by the drain).
    let mut by_hash: std::collections::HashMap<String, Vec<crate::content::PreparedChunk>> =
        std::collections::HashMap::new();
    for pc in prepared {
        by_hash.entry(pc.content_hash.clone()).or_default().push(pc);
    }
    let mut aligned: Vec<crate::content::PreparedChunk> = Vec::with_capacity(expected.len());
    for (i, e) in expected.iter().enumerate() {
        let claimed = by_hash.get_mut(&e.content_hash).and_then(|q| {
            if q.is_empty() {
                None
            } else {
                Some(q.remove(0))
            }
        });
        match claimed {
            Some(mut pc) => {
                pc.chunk_index = i as i32;
                aligned.push(pc);
            }
            None => {
                let (header_path, heading_depth) =
                    crate::content::map_heading(e.header_path.clone(), e.heading_depth);
                aligned.push(crate::content::PreparedChunk {
                    chunk_id: ChunkId::from(Uuid::now_v7()),
                    chunk_index: i as i32,
                    content_hash: e.content_hash.clone(),
                    content: e.content.clone(),
                    // No caller vector matched this chunk's content: the async-embed posture
                    // persists text + hash now and the drain backfills the vector (issue #299).
                    embedding: None,
                    embedded_with: None,
                    header_path,
                    heading_depth,
                });
            }
        }
    }

    // Kept detection: derived-merkle equality against an unclaimed incumbent, AND byte equality
    // with the section text (chunk content_hash is over TRIMMED text — a boundary-straddling
    // whitespace edge can hold equal hashes over different bytes; keeping such a row would
    // re-compose the body from the incumbent's old bytes). Slots stay in SECTION order so every
    // later stage (delta fold, sidecar assembly) indexes sections directly.
    enum Slot {
        Kept {
            block_id: BlockId,
            incumbent_seq: i32,
            attribution: Vec<payloads::ReblockAttribution>,
        },
        Created {
            block_id: BlockId,
            attribution: Vec<payloads::ReblockAttribution>,
        },
    }
    let mut slices_out: Vec<(BlockId, String)> = Vec::new();
    let mut slots: Vec<Slot> = Vec::with_capacity(sections.len());
    let mut claimed: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    let mut offset = 0usize;
    for (text, chunks) in &sections {
        let run = &aligned[offset..offset + chunks.len()];
        offset += chunks.len();
        let merkle = temper_ingest::merkle::block_merkle(
            &run.iter()
                .map(|c| c.content_hash.clone())
                .collect::<Vec<_>>(),
        );
        let incumbent = live_blocks.iter().find(|b| {
            b.body_hash.as_deref() == Some(merkle.as_str())
                && !claimed.contains(&b.id)
                && b.bytes.as_deref() == Some(text.as_str())
        });
        match incumbent {
            Some(b) => {
                claimed.insert(b.id);
                slots.push(Slot::Kept {
                    block_id: BlockId::from(b.id),
                    incumbent_seq: b.seq,
                    attribution: Vec::new(), // filled below
                });
            }
            None => {
                let new_id = BlockId::from(Uuid::now_v7());
                slots.push(Slot::Created {
                    block_id: new_id,
                    attribution: Vec::new(), // filled below
                });
                slices_out.push((new_id, text.clone()));
            }
        }
    }

    // ── The attribution delta, CONTENT-grounded ──
    // Incumbent chunk-hash runs (multiplicity-aware) and per-section hash count maps.
    let mut block_hashes: std::collections::HashMap<Uuid, Vec<&str>> =
        std::collections::HashMap::new();
    for c in live_chunks {
        block_hashes
            .entry(c.block_id)
            .or_default()
            .push(c.content_hash.as_str());
    }
    let section_counts: Vec<std::collections::HashMap<&str, usize>> = sections
        .iter()
        .map(|(_, chunks)| {
            let mut m: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
            for c in chunks {
                *m.entry(c.content_hash.as_str()).or_default() += 1;
            }
            m
        })
        .collect();

    /// One delta entry for section k. `from_caller` discriminates a caller whole-body assertion
    /// (appends across events) from an absorbed union of a folded incumbent (skipped when the
    /// survivor already holds the source).
    struct DeltaEntry {
        source: ProvenanceSource,
        seq: i32,
        carried: bool,
        from_caller: bool,
    }

    /// Per-source seats in one section's delta: the absorbed union (if a folded incumbent
    /// contributed it) and the caller assertion (if the caller asserted it).
    struct SourceSeats {
        source: ProvenanceSource,
        union: Option<DeltaEntry>,
        caller: Option<DeltaEntry>,
    }
    let mut delta: std::collections::HashMap<usize, Vec<DeltaEntry>> =
        std::collections::HashMap::new();
    for a in attributions {
        let Some(hashes) = block_hashes.get(&a.block_id) else {
            continue; // a chunkless incumbent has no content to locate — no geometry
        };
        if claimed.contains(&a.block_id) {
            continue; // a kept incumbent's own rows ride along, never re-listed
        }
        let mut present_any = false;
        let mut absorbed: Vec<bool> = vec![false; sections.len()];
        for (k, counts) in section_counts.iter().enumerate() {
            let mut need: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
            for h in hashes.iter() {
                *need.entry(h).or_default() += 1;
            }
            absorbed[k] = need
                .iter()
                .all(|(h, n)| counts.get(h).is_some_and(|c| c >= n));
            present_any |= hashes.iter().any(|h| counts.contains_key(h));
        }
        if !present_any {
            // Content-gone (ruled): the caller deleted this incumbent's content. Its rows stay
            // history on the folded row — there is no honest live target.
            continue;
        }
        for (k, counts) in section_counts.iter().enumerate() {
            let some = hashes.iter().any(|h| counts.contains_key(h));
            if absorbed[k] {
                delta.entry(k).or_default().push(DeltaEntry {
                    source: a.source.clone(),
                    seq: a.accretion_seq,
                    carried: false,
                    from_caller: false,
                });
            } else if some {
                delta.entry(k).or_default().push(DeltaEntry {
                    source: a.source.clone(),
                    seq: a.accretion_seq,
                    carried: true,
                    from_caller: false,
                });
            }
        }
    }
    // Caller whole-body sources: body-grain over the NEW chunk space — carried iff the body is
    // multi-section; landed on every section, kept or created.
    for inc in sources {
        for k in 0..sections.len() {
            delta.entry(k).or_default().push(DeltaEntry {
                source: inc.source.clone(),
                seq: inc.seq,
                carried: sections.len() > 1,
                from_caller: true,
            });
        }
    }

    // Fold the delta into the slots. Per section: the shipped dedup sort (absorbed beats
    // carried, then lower accretion_seq), one entry per source; then the kept-skip — a deduped
    // ABSORBED union entry whose source the survivor already holds is dropped (the union adds
    // nothing), while a caller assertion appends by construction. On a created block nothing is
    // ever "held" — every deduped entry lands.
    let held: std::collections::HashMap<Uuid, Vec<&ProvenanceSource>> =
        attributions
            .iter()
            .fold(std::collections::HashMap::new(), |mut m, a| {
                m.entry(a.block_id).or_default().push(&a.source);
                m
            });
    for (k, slot) in slots.iter_mut().enumerate() {
        let Some(entries) = delta.remove(&k) else {
            continue;
        };
        // Resolve per SOURCE, not per entry — a caller assertion and an absorbed union of the
        // same source can meet in one section, and resolution must never depend on seq
        // ordering: the union (direct-quality, carried = false) wins when it lands; when the
        // union is dropped (survivor already holds the source) the CALLER's assertion is the
        // fallback and still appends. Within one event a source lands at most one row per
        // block, deterministically.
        let block_id = match slot {
            Slot::Kept { block_id, .. } => block_id.uuid(),
            Slot::Created { .. } => Uuid::nil(),
        };
        let survivor_holds =
            |e: &DeltaEntry| held.get(&block_id).is_some_and(|v| v.contains(&&e.source));
        // ProvenanceSource is Eq, not Hash — a Vec scan, the same grain the shipped dedup uses.
        let mut by_source: Vec<SourceSeats> = Vec::new();
        for e in entries {
            let seat = match by_source.iter_mut().find(|s| s.source == e.source) {
                Some(seat) => seat,
                None => {
                    by_source.push(SourceSeats {
                        source: e.source.clone(),
                        union: None,
                        caller: None,
                    });
                    by_source.last_mut().unwrap()
                }
            };
            if e.from_caller {
                // lowest caller seq wins the representative seat
                match &seat.caller {
                    Some(prev) if prev.seq <= e.seq => {}
                    _ => seat.caller = Some(e),
                }
            } else {
                match &seat.union {
                    Some(prev) if prev.seq <= e.seq => {}
                    _ => seat.union = Some(e),
                }
            }
        }
        let mut resolved: Vec<DeltaEntry> = by_source
            .into_iter()
            .filter_map(|seat| match seat.union {
                Some(u) if !survivor_holds(&u) => Some(u),
                Some(_) => seat.caller, // union held: the caller's append is the fallback
                None => seat.caller,
            })
            .collect();
        resolved.sort_by_key(|e| (e.carried, e.seq));
        let attribution: Vec<payloads::ReblockAttribution> = resolved
            .into_iter()
            .map(|e| payloads::ReblockAttribution {
                source: e.source,
                seq: e.seq,
                carried: e.carried,
            })
            .collect();
        match slot {
            Slot::Kept { attribution: a, .. } => *a = attribution,
            Slot::Created { attribution: a, .. } => *a = attribution,
        }
    }

    // Split the slots into the manifest's kept/created vecs, in section order.
    let mut kept: Vec<payloads::ReblockKeptBlock> = Vec::new();
    let mut created: Vec<payloads::ReblockCreatedBlock> = Vec::new();
    let mut kept_incumbent_seq: Vec<i32> = Vec::new();
    let mut created_section: Vec<usize> = Vec::new();
    for (k, slot) in slots.iter().enumerate() {
        match slot {
            Slot::Kept {
                block_id,
                incumbent_seq,
                attribution,
            } => {
                kept.push(payloads::ReblockKeptBlock {
                    block_id: *block_id,
                    seq: k as i32,
                    attribution: attribution.clone(),
                });
                kept_incumbent_seq.push(*incumbent_seq);
            }
            Slot::Created {
                block_id,
                attribution,
            } => {
                created.push(payloads::ReblockCreatedBlock {
                    block_id: *block_id,
                    seq: k as i32,
                    chunks: Vec::new(), // filled below
                    attribution: attribution.clone(),
                });
                created_section.push(k);
            }
        }
    }

    let folded: Vec<BlockId> = live_blocks
        .iter()
        .filter(|b| !claimed.contains(&b.id))
        .map(|b| BlockId::from(b.id))
        .collect();

    // No-op: everything kept at unchanged seqs AND no new assertions anywhere — the exact twin
    // of the SQL entry guard. (The identical-bytes-no-sources whole-body write never reaches
    // here on a multi-block resource — the arm's byte short-circuit fires first — but on a
    // single-block resource it lands HERE: silence, the named behavior change.)
    let seqs_moved = kept
        .iter()
        .zip(&kept_incumbent_seq)
        .any(|(kb, inc_seq)| kb.seq != *inc_seq);
    let has_assertions = !sources.is_empty()
        || kept.iter().any(|kb| !kb.attribution.is_empty())
        || created.iter().any(|c| !c.attribution.is_empty());
    if created.is_empty() && folded.is_empty() && !seqs_moved && !has_assertions {
        return Ok(None);
    }

    // The disposition map (D-D2): where each folded incumbent's content went, captured HERE —
    // one step before the attribution delta drops incumbent identity — by the same
    // hash-multiset geometry the delta consumes. Created absorbers are the COMMON case on this
    // arm (a rewritten-away incumbent is absorbed into a freshly minted section), so absorbers
    // name kept AND created blocks. A chunkless incumbent has no entry in `block_hashes`: no
    // geometry, its disposition is content-gone — the named arm, not an inference.
    let mut dispositions: payloads::FoldDispositions = payloads::FoldDispositions::new();
    for b in live_blocks.iter().filter(|b| !claimed.contains(&b.id)) {
        let disposition = match block_hashes.get(&b.id) {
            None => payloads::FoldDisposition::ContentGone,
            Some(hashes) => {
                // Multiplicity-aware: the section must hold every hash as often as the
                // incumbent does (the same containment the delta's absorbed[] computes).
                let mut need: std::collections::HashMap<&str, usize> =
                    std::collections::HashMap::new();
                for h in hashes {
                    *need.entry(h).or_default() += 1;
                }
                let mut absorbers = Vec::new();
                let mut carried = Vec::new();
                for (k, counts) in section_counts.iter().enumerate() {
                    let section_id = match &slots[k] {
                        Slot::Kept { block_id, .. } | Slot::Created { block_id, .. } => *block_id,
                    };
                    if need
                        .iter()
                        .all(|(h, n)| counts.get(h).is_some_and(|c| c >= n))
                    {
                        absorbers.push(section_id);
                    } else if hashes.iter().any(|h| counts.contains_key(h)) {
                        carried.push(section_id);
                    }
                }
                payloads::FoldDisposition::from_geometry(absorbers, carried)
            }
        };
        dispositions.insert(BlockId::from(b.id), disposition);
    }

    // The sidecar chunk set: every CREATED block's prepared chunks, renumbered per block, in
    // manifest order. Kept blocks' chunks ride their untouched rows and contribute nothing.
    let mut chunks_out: Vec<crate::content::PreparedChunk> = Vec::new();
    let mut created_i = 0usize;
    for (k, (_, chunks)) in sections.iter().enumerate() {
        if created_section.get(created_i) == Some(&k) {
            let c = &mut created[created_i];
            created_i += 1;
            let start: usize = sections[..k].iter().map(|(_, cs)| cs.len()).sum();
            let run = &aligned[start..start + chunks.len()];
            c.chunks = run
                .iter()
                .enumerate()
                .map(|(i, pc)| payloads::ChunkManifest {
                    chunk_id: pc.chunk_id,
                    chunk_index: i as i32,
                    content_hash: pc.content_hash.clone(),
                })
                .collect();
            let mut renumbered: Vec<crate::content::PreparedChunk> = run.to_vec();
            for (i, pc) in renumbered.iter_mut().enumerate() {
                pc.chunk_index = i as i32;
            }
            chunks_out.extend(renumbered);
        }
    }

    Ok(Some(ReplacePlan {
        manifest: payloads::ResourceReblocked {
            resource_id: resource,
            created,
            kept,
            folded,
            replaces_body: true,
            dispositions,
        },
        slices: slices_out,
        chunks: chunks_out,
    }))
}

/// [`reblock_resource_with`] under the default (un-attributed) context.
pub async fn reblock_resource(pool: &PgPool, p: ReblockParams) -> Result<ReblockOutcome> {
    reblock_resource_with(pool, p, EventContext::default()).await
}

/// Re-block a resource under an explicit [`EventContext`]. Computes the partition, refuses the
/// guessable, fires `resource_reblocked` when there is something to change.
pub async fn reblock_resource_with(
    pool: &PgPool,
    p: ReblockParams,
    ctx: EventContext,
) -> Result<ReblockOutcome> {
    let mut tx = begin_scoped(pool).await?;
    let outcome = reblock_resource_in_tx(&mut tx, p, ctx).await?;
    tx.commit().await?;
    Ok(outcome)
}

/// In-transaction variant of [`reblock_resource`] — reads the live projection and fires on a
/// caller-supplied connection.
pub async fn reblock_resource_in_tx(
    conn: &mut sqlx::PgConnection,
    p: ReblockParams,
    ctx: EventContext,
) -> Result<ReblockOutcome> {
    // A partition decision over a still-arriving body is a guess.
    let ingest_state: String = sqlx::query_scalar!(
        "SELECT ingest_state FROM kb_resources WHERE id = $1",
        p.resource.uuid()
    )
    .fetch_one(&mut *conn)
    .await
    .with_context(|| format!("reblock_resource: resource {} not found", p.resource))?;
    if ingest_state == "in_progress" {
        return Ok(ReblockOutcome::Declined {
            reason: format!(
                "resource {} is mid-ingest (in_progress) — a partition decision over a \
                 still-arriving body would be a guess",
                p.resource
            ),
        });
    }

    let live_blocks: Vec<LiveBlock> = read_live_blocks(&mut *conn, p.resource).await?;
    if live_blocks.is_empty() {
        return Ok(ReblockOutcome::Declined {
            reason: format!("resource {} has no live blocks to partition", p.resource),
        });
    }
    // The design slices STORED block content and never mutates text — a block whose bytes were
    // never stored (a derived charter/scenario shape) would force the body to be re-derived
    // from chunks, fabricating bytes the ledger never carried.
    if let Some(missing) = live_blocks.iter().find(|b| b.bytes.is_none()) {
        return Ok(ReblockOutcome::Declined {
            reason: format!(
                "block {} (seq {}) of resource {} stores no verbatim bytes (a derived shape) — \
                 re-blocking composes the body from stored bytes only",
                missing.id, missing.seq, p.resource
            ),
        });
    }

    let live_chunks: Vec<LiveChunk> = read_live_chunks(&mut *conn, p.resource).await?;

    let attributions: Vec<AttributionRow> = read_attributions(&mut *conn, p.resource).await?;

    // The body composes from verbatim block bytes only — never from chunk reconstruction.
    let mut body = String::new();
    for b in &live_blocks {
        body.push_str(b.bytes.as_deref().expect("refused above"));
    }

    let plan = match compute_reblock_partition(
        p.resource,
        &body,
        &live_blocks,
        &live_chunks,
        &attributions,
    )? {
        Partition::NoOp => return Ok(ReblockOutcome::NoOp),
        Partition::Declined(reason) => return Ok(ReblockOutcome::Declined { reason }),
        Partition::Plan(plan) => plan,
    };

    let event = fire_with(
        &mut *conn,
        SeedAction::ResourceReblock {
            manifest: plan.manifest,
            slices: &plan.slices,
            // The shipped op shape inserts no chunks — the sidecar chunk map stays empty and
            // the projector reparents existing CAS rows.
            chunks: &[],
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .reblocked_event()?;
    Ok(ReblockOutcome::Reblocked { event })
}

/// The write-path policy application point: re-partition a just-written body to the blocking
/// policy (v1: heading-aligned sections) inside the write's own transaction.
///
/// This is the ONE server-side application point behind the goal register's convergence claim —
/// every cloud write surface (API, CLI, UI, MCP; human or machine principal) reaches its body
/// write through `create_resource` / `update_resource` / `finalize_ingest`, and each of those
/// calls this at its tail, so a surface cannot produce an observable partition that contradicts
/// policy. Authorization is never re-checked here: the caller has already run the standard gate
/// train (DbBackend gates before dispatching), and the re-block fires on-behalf-of the write's
/// acting principal — `ctx` carries the authorship/correlation into `kb_events` (the authored-4
/// pattern), keeping the substrate principal-free by architecture. The op is reachable ONLY
/// through these gated write paths (enforced by the `reblock_scope_fence` tripwire).
///
/// `NoOp` is silence by design: a write that does not change the effective partition must be
/// indistinguishable in the ledger from one that never happened (the op fires nothing). The op's
/// refusals (derived shape, chunker drift) propagate as errors — the enclosing write rolls back
/// whole, a well-formed no rather than an approximation presented as a partition.
async fn apply_blocking_policy_in_tx(
    conn: &mut sqlx::PgConnection,
    resource: ResourceId,
    emitter: EntityId,
    ctx: EventContext,
    decline: Decline,
) -> Result<()> {
    // A resource whose live blocks do not all store verbatim bytes (a derived shape — the
    // reconcile sentinel, or the contract-legal chunks-without-content append) has no body to
    // compose, so this write can make no partition decision: skip. Same "no prose to partition"
    // judgment as the empty-body guards at the call sites — and unlike a direct op invocation,
    // the enclosing write often has NO recovery path (a finalize refusal would strand the
    // resource `in_progress` forever; the upload cannot be re-offered), so declining is not
    // available here. Derived shapes stay outside v1 policy reach (the goal register's declared
    // boundary); the skip is silence, never an approximation presented as a partition.
    let byteless: Option<i64> = sqlx::query_scalar!(
        r#"SELECT count(*) FROM kb_content_blocks b
           LEFT JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id
           WHERE b.resource_id = $1 AND NOT b.is_folded AND bc.block_revision_id IS NULL"#,
        resource.uuid()
    )
    .fetch_one(&mut *conn)
    .await?;
    if byteless.unwrap_or(0) > 0 {
        return Ok(());
    }
    match reblock_resource_in_tx(conn, ReblockParams { resource, emitter }, ctx).await? {
        ReblockOutcome::NoOp | ReblockOutcome::Reblocked { .. } => Ok(()),
        ReblockOutcome::Declined { reason } => match decline {
            Decline::Fatal => anyhow::bail!("reblock_resource: {reason}"),
            Decline::Skip => Ok(()),
        },
    }
}

/// What the hook does when the re-block op [`ReblockOutcome::Declined`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decline {
    /// The enclosing write is caller-fixable (create/update): surface the decline as an error.
    Fatal,
    /// The enclosing write has no recovery path (finalize): commit the honest, unpoliced
    /// partition. Silence, never an approximation presented as a partition.
    Skip,
}

/// Record an auditor's signed verdict on one `(block, source)` citation (Set 5, spec §4.1-4.2).
/// Append-only: fires `citation_audited`, which the SQL projector (`_project_citation_audited`)
/// inserts into `kb_citation_audits` with no supersession — a later audit never overwrites an
/// earlier one, even an opposite-signed one (spec §4.1's "a later +1.0 never erases an earlier
/// -1.0").
#[derive(Debug)]
pub struct CitationAuditParams<'a> {
    pub block: BlockId,
    pub source: ProvenanceSource,
    pub value: f64,
    pub reason: Option<&'a str>,
    pub emitter: EntityId,
}

/// [`record_citation_audit_with`] under the default (un-attributed) context.
pub async fn record_citation_audit(pool: &PgPool, p: CitationAuditParams<'_>) -> Result<Uuid> {
    record_citation_audit_with(pool, p, EventContext::default()).await
}

/// Record a citation audit under an explicit [`EventContext`] — the auditor's own confidence in
/// its verdict rides `ctx.authorship` (→ `kb_events.metadata`), never the payload: the audit's
/// signed `value` is the only thing the projection ever sees (spec §4.2's self-grading
/// prohibition — an agent's own confidence must never move its own standing).
pub async fn record_citation_audit_with(
    pool: &PgPool,
    p: CitationAuditParams<'_>,
    ctx: EventContext,
) -> Result<Uuid> {
    let mut tx = begin_scoped(pool).await?;
    let audit = fire_with(
        &mut tx,
        SeedAction::CitationAudit {
            block: p.block,
            source: p.source,
            value: p.value,
            reason: p.reason,
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .citation_audit()?;
    tx.commit().await?;
    Ok(audit)
}

/// Soft-delete a resource.
pub async fn delete_resource(pool: &PgPool, resource: ResourceId, emitter: EntityId) -> Result<()> {
    delete_resource_with(pool, resource, emitter, EventContext::default()).await
}

/// [`delete_resource`] under an explicit [`EventContext`] — the `resource_deleted` act is correlated
/// to the caller's invocation + stamped with its authorship. Mirrors `fire`/`fire_with`.
pub async fn delete_resource_with(
    pool: &PgPool,
    resource: ResourceId,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    delete_resource_in_tx(&mut tx, resource, emitter, ctx).await?;
    tx.commit().await?;
    Ok(())
}

/// In-transaction variant of [`delete_resource`] — fires on a caller-supplied connection (no
/// begin/commit). `ctx` correlates the `resource_deleted` act (`EventContext::default()` for an
/// un-attributed delete).
pub async fn delete_resource_in_tx(
    conn: &mut sqlx::PgConnection,
    resource: ResourceId,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    fire_with(conn, SeedAction::ResourceDelete { resource, emitter }, ctx).await?;
    Ok(())
}

/// Reassign a resource's owner (event-sourced, in-place). Un-attributed convenience.
pub async fn reassign_resource(
    pool: &PgPool,
    resource: ResourceId,
    from: ProfileId,
    to: ProfileId,
    emitter: EntityId,
) -> Result<()> {
    reassign_resource_with(pool, resource, from, to, emitter, EventContext::default()).await
}

/// [`reassign_resource`] under an explicit [`EventContext`] — the `resource_reassigned`
/// act is correlated to the caller's invocation + stamped with its authorship.
pub async fn reassign_resource_with(
    pool: &PgPool,
    resource: ResourceId,
    from: ProfileId,
    to: ProfileId,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    reassign_resource_in_tx(&mut tx, resource, from, to, emitter, ctx).await?;
    tx.commit().await?;
    Ok(())
}

/// In-transaction variant — fires on a caller-supplied connection (no begin/commit),
/// so the bulk path can reassign N resources atomically.
pub async fn reassign_resource_in_tx(
    conn: &mut sqlx::PgConnection,
    resource: ResourceId,
    from: ProfileId,
    to: ProfileId,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    fire_with(
        conn,
        SeedAction::ResourceReassign {
            resource,
            from_profile: from,
            to_profile: to,
            emitter,
        },
        ctx,
    )
    .await?;
    Ok(())
}

/// Reassign a context's owner (event-sourced, in-place) under an explicit [`EventContext`].
/// The single write path for context ownership transfer; `kb_contexts` is a replay input
/// table, so the `context_reassigned` projector is an idempotent re-apply on replay.
pub async fn reassign_context_with(
    pool: &PgPool,
    context: ContextId,
    from_owner: (&str, Uuid),
    to_owner: (&str, Uuid),
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    fire_with(
        &mut tx,
        SeedAction::ContextReassign {
            context,
            from_owner_table: from_owner.0,
            from_owner_id: from_owner.1,
            to_owner_table: to_owner.0,
            to_owner_id: to_owner.1,
            emitter,
        },
        ctx,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Rename a context in place (event-sourced) under an explicit [`EventContext`].
/// The single write path for a context's `(name, slug)` pair; `kb_contexts` is a replay input
/// table, so the `context_renamed` projector is an idempotent re-apply on replay.
/// `from` is `(name, slug)` before, `to` is `(name, slug)` after — the `from` pair is carried for
/// the trail only (`kb_contexts` keeps no before-image), never read by the projector.
pub async fn rename_context_with(
    pool: &PgPool,
    context: ContextId,
    from: (&str, &str),
    to: (&str, &str),
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    fire_with(
        &mut tx,
        SeedAction::ContextRename {
            context,
            from_name: from.0,
            from_slug: from.1,
            to_name: to.0,
            to_slug: to.1,
            emitter,
        },
        ctx,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Retire a context in place (event-sourced) under an explicit [`EventContext`]. Flips
/// `kb_contexts.is_active` to false and rewrites its slug to the already-mangled address the
/// caller computed (via `next_unique_context_slug`); `kb_contexts` is a replay input table, so the
/// `context_retired` projector is an idempotent re-apply on replay. `from_slug` is the address
/// before the mangle, `to_slug` after — `from_slug` is carried for the trail only (`kb_contexts`
/// keeps no before-image), never read by the projector.
pub async fn retire_context_with(
    pool: &PgPool,
    context: ContextId,
    from_slug: &str,
    to_slug: &str,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    fire_with(
        &mut tx,
        SeedAction::ContextRetire {
            context,
            from_slug,
            to_slug,
            emitter,
        },
        ctx,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Restore a retired context in place (event-sourced) under an explicit [`EventContext`]. The
/// mirror of [`retire_context_with`]: flips `kb_contexts.is_active` back to true and writes the
/// re-derived address; the `context_restored` projector is likewise an idempotent re-apply on
/// replay.
pub async fn restore_context_with(
    pool: &PgPool,
    context: ContextId,
    from_slug: &str,
    to_slug: &str,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    fire_with(
        &mut tx,
        SeedAction::ContextRestore {
            context,
            from_slug,
            to_slug,
            emitter,
        },
        ctx,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ── cogmap-homed kernel writes (L0 reconcile) ──────────────────────────────────

/// Create a kernel resource homed to a **cogmap** (not a context) — the shape the L0 reconciler
/// uses. Mirrors [`create_resource`] but homes `AnchorRef::cogmap(p.cogmap)` and passes
/// `originator: None` (kernel content's originator COALESCEs to `owner` = system). The post-create
/// property loop of `create_resource` is intentionally omitted: kernel facets/provenance are stamped
/// by the caller via [`set_property`] / [`set_facet`].
#[derive(Debug)]
pub struct KernelCreateParams<'a> {
    pub cogmap: CogmapId,
    /// The STABLE landmark identity the resource is minted under (the reconcile diff key). Supplying it
    /// (rather than minting) makes a duplicate create a PRIMARY-KEY conflict — fail-loud, never a silent
    /// twin.
    pub resource_id: Uuid,
    pub title: &'a str,
    pub origin_uri: &'a str,
    pub doc_type: &'a str,
    pub body: &'a str,
    /// Caller-supplied, already-embedded chunks. When `Some`, the body block is built from these
    /// verbatim (the client embedded); when `None`, the server chunks + embeds `body` (fallback) —
    /// the same client-embed-or-server-fallback affordance as [`create_resource`].
    pub chunks: Option<Vec<IncomingChunk>>,
    pub owner: ProfileId,
    pub emitter: EntityId,
}

pub async fn create_kernel_resource(
    pool: &PgPool,
    p: KernelCreateParams<'_>,
) -> Result<ResourceId> {
    let mut tx = begin_scoped(pool).await?;
    let new_id = create_kernel_resource_in_tx(&mut tx, p, EventContext::default()).await?;
    tx.commit().await?;
    Ok(new_id)
}

/// In-transaction variant of [`create_kernel_resource`] — fires on a caller-supplied connection (no
/// begin/commit) so the L0 reconcile can run every mutation in ONE serializable transaction. `ctx`
/// correlates the `resource_created` act to the reconcile run (`EventContext::default()` for an
/// un-attributed create).
pub async fn create_kernel_resource_in_tx(
    conn: &mut sqlx::PgConnection,
    p: KernelCreateParams<'_>,
    ctx: EventContext,
) -> Result<ResourceId> {
    let mut block = match p.chunks {
        Some(chunks) => prepare_block_from_chunks(0, None, chunks),
        None => prepare_block(0, None, p.body)?,
    };
    // Reconcile creates kernel resources with `body: ""` (content rides in `chunks`) — that empty
    // sentinel must NOT store an empty verbatim row. See `raw_body`.
    block.raw_text = raw_body(p.body);
    let blocks = [block];
    let new_id = fire_with(
        conn,
        SeedAction::ResourceCreate {
            title: p.title,
            origin_uri: p.origin_uri,
            // Mint under the caller's STABLE landmark id (the diff key) — so a duplicate create is a
            // primary-key conflict, never a silent twin.
            resource_id: Some(ResourceId::from(p.resource_id)),
            home: AnchorRef::cogmap(p.cogmap),
            owner: p.owner,
            // Kernel content's originator COALESCEs to owner (= system).
            originator: None,
            blocks: &blocks,
            doc_type: Some(p.doc_type),
            emitter: p.emitter,
            // The kernel resource is created whole, in one act — never segmented.
            segmented: false,
        },
        ctx,
    )
    .await?
    .resource()?;
    Ok(new_id)
}

/// Replace a cogmap's telos charter with `blocks` (role-tagged, pre-embedded), in a caller-supplied
/// transaction. Fires `SeedAction::CharterSet` → `cogmap_charter_set` (fold-then-reproject). Returns the
/// telos resource id. The L0 charter reconciler calls this when the desired charter's body merkle differs
/// from the telos's current `body_hash` (see [`crate::readback::telos_charter_state`]).
pub async fn set_charter_in_tx(
    conn: &mut sqlx::PgConnection,
    cogmap: CogmapId,
    blocks: &[PreparedBlock],
    emitter: EntityId,
    ctx: EventContext,
) -> Result<ResourceId> {
    fire_with(
        conn,
        SeedAction::CharterSet {
            cogmap,
            blocks,
            emitter,
        },
        ctx,
    )
    .await?
    .charter()
}

/// Set the **clustering** facet on an owner — **one `kb_properties` row per inner key** of `values`
/// (`{layer: concept, status: open}` → two rows). This is what materialization/affinity read, and the
/// grain matches what `expand_facets` consumes. NOT interchangeable with [`set_property`]
/// (Decision #6): `provenance` is per-key, not a clustering facet.
///
/// **Patch, not replace.** Asserting a key folds the prior live row for *that key* and leaves every
/// unnamed mark untouched, so a one-key assert never silently drops a resource's other facets. Returns
/// one id per row written — plural because one assert can mint many
/// (task `019f6d08-2b55-7ee0-b9ac-1959cf4d736b`).
///
/// The owner is a [`PropertyOwner`], so a facet may hang off an **edge** as well as a resource. The
/// event still anchors on a context or cogmap either way — `_property_owner_anchor` resolves it from
/// the resource's home, or from the edge's own `home_anchor_*` columns.
pub async fn set_facet(
    pool: &PgPool,
    owner: PropertyOwner,
    values: &serde_json::Value,
    weight: f64,
    emitter: EntityId,
) -> Result<Vec<PropertyId>> {
    set_facet_with(
        pool,
        owner,
        values,
        weight,
        emitter,
        EventContext::default(),
    )
    .await
}

/// [`set_facet`] under an explicit [`EventContext`] — the `facet_set` act is correlated to the
/// caller's invocation + stamped with its authorship. Mirrors `fire`/`fire_with`. Returns the
/// `kb_properties.id`s the fire produced (surfaced from `Fired::Facet`).
pub async fn set_facet_with(
    pool: &PgPool,
    owner: PropertyOwner,
    values: &serde_json::Value,
    weight: f64,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<Vec<PropertyId>> {
    let mut tx = begin_scoped(pool).await?;
    let property_ids = set_facet_in_tx(&mut tx, owner, values, weight, emitter, ctx).await?;
    tx.commit().await?;
    Ok(property_ids)
}

/// In-transaction variant of [`set_facet`] — fires on a caller-supplied connection (no begin/commit).
/// `ctx` correlates the `facet_set` act (`EventContext::default()` for an un-attributed facet). Returns
/// the `kb_properties.id`s the fire produced.
pub async fn set_facet_in_tx(
    conn: &mut sqlx::PgConnection,
    owner: PropertyOwner,
    values: &serde_json::Value,
    weight: f64,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<Vec<PropertyId>> {
    fire_with(
        conn,
        SeedAction::FacetSet {
            owner,
            values,
            weight,
            emitter,
        },
        ctx,
    )
    .await?
    .facet()
}

/// Assert one keyed property row on a [`PropertyOwner`] — the write behind the edge facet
/// surface's keyed mode (the `anchored-at` span qualification). **Insert-if-not-live**: a live
/// row for the exact (owner, key, value) acks its own id and appends no event; otherwise one
/// row fires through [`SeedAction::KeyedPropertyAssert`] and its id is returned. Never a
/// fold-then-reinsert — a second address appends a second row, and a re-assert after the row
/// is folded mints a fresh one, because the partial unique index (`uq_kb_properties_active`)
/// sees live rows only.
///
/// The liveness pre-check shares the fire's transaction: a row that commits between the check
/// and the insert still collides on `uq_kb_properties_active`, aborting the transaction with
/// no event appended — the caller's conflict to retry, which then acks.
pub async fn assert_keyed_property_with(
    pool: &PgPool,
    owner: PropertyOwner,
    key: &str,
    value: &serde_json::Value,
    weight: f64,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<PropertyId> {
    let mut tx = begin_scoped(pool).await?;
    let acked: Option<Uuid> = sqlx::query_scalar!(
        "SELECT id FROM kb_properties \
          WHERE owner_table = $1 AND owner_id = $2 AND property_key = $3 \
            AND property_value = $4 AND NOT is_folded",
        owner.owner_table(),
        owner.uuid(),
        key,
        value,
    )
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(existing) = acked {
        tx.commit().await?;
        return Ok(PropertyId::from(existing));
    }
    let ids = fire_with(
        &mut tx,
        SeedAction::KeyedPropertyAssert {
            owner,
            key,
            value,
            weight,
            emitter,
        },
        ctx,
    )
    .await?
    .facet()?;
    let id = ids
        .into_iter()
        .next()
        .context("keyed property assert returned no row id")?;
    tx.commit().await?;
    Ok(id)
}

/// Retract one property row bound to its owning edge — the correction verb for the `anchored-at`
/// span qualifications. The edge-bound fold runs inside the fire's transaction, so a refusal
/// (foreign owner, missing id, already retracted — one indistinguishable error,
/// [`PropertyRetractError`]) appends no ledger event. Success folds the row and stamps the
/// retraction event; the row persists, and the address is re-assertable.
pub async fn retract_property_with(
    pool: &PgPool,
    edge: EdgeId,
    property_id: PropertyId,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<PropertyId> {
    let mut tx = begin_scoped(pool).await?;
    let retracted = fire_with(
        &mut tx,
        SeedAction::PropertyRetract {
            edge,
            property_id,
            emitter,
        },
        ctx,
    )
    .await?
    .property_retract()?;
    tx.commit().await?;
    Ok(retracted)
}

/// Set a single-valued **per-key** property — folds prior active `(owner, key)` rows then asserts the
/// new value, so the key holds one current value (`property_key=<key>`). This is the shape
/// `readback::kernel_slice` reads; the reconciler stamps `provenance: kernel` through it.
pub async fn set_property(
    pool: &PgPool,
    resource: ResourceId,
    key: &str,
    value: &serde_json::Value,
    emitter: EntityId,
) -> Result<()> {
    set_property_with(pool, resource, key, value, emitter, EventContext::default()).await
}

/// [`set_property`] under an explicit [`EventContext`] — the `property_set` act is correlated to the
/// caller's invocation + stamped with its authorship. Mirrors `fire`/`fire_with`.
pub async fn set_property_with(
    pool: &PgPool,
    resource: ResourceId,
    key: &str,
    value: &serde_json::Value,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    set_property_in_tx(&mut tx, resource, key, value, emitter, ctx).await?;
    tx.commit().await?;
    Ok(())
}

/// In-transaction variant of [`set_property`] — fires on a caller-supplied connection (no begin/commit).
/// `ctx` correlates the `property_set` act (`EventContext::default()` for an un-attributed property).
pub async fn set_property_in_tx(
    conn: &mut sqlx::PgConnection,
    resource: ResourceId,
    key: &str,
    value: &serde_json::Value,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    fire_with(
        conn,
        SeedAction::PropertySet {
            resource,
            key,
            value,
            weight: 1.0,
            emitter,
        },
        ctx,
    )
    .await?;
    Ok(())
}

/// Assert (or idempotently re-assert) a **cogmap-homed** edge `src → tgt`, returning its id. Mirrors
/// [`assert_relationship`] but homes `EdgeHome::Cogmap(p.cogmap)` (kernel landmarks home to the map,
/// not a context).
#[derive(Debug)]
pub struct KernelEdgeParams<'a> {
    pub cogmap: CogmapId,
    pub src: ResourceId,
    pub tgt: ResourceId,
    pub kind: EdgeKind,
    pub polarity: EdgePolarity,
    pub label: Option<&'a str>,
    pub weight: f64,
    pub emitter: EntityId,
}

pub async fn assert_kernel_edge(pool: &PgPool, p: KernelEdgeParams<'_>) -> Result<EdgeId> {
    assert_kernel_edge_with(pool, p, EventContext::default()).await
}

/// [`assert_kernel_edge`] under an explicit [`EventContext`] — the authored `relationship_asserted`
/// act carries the caller's authorship + invocation correlator. This is the pool-level ctx variant
/// `DbBackend::assert_relationship` dispatches to when the source resource is **cogmap-homed** (a
/// steward's authored-4 node), homing the edge to the map rather than a context.
pub async fn assert_kernel_edge_with(
    pool: &PgPool,
    p: KernelEdgeParams<'_>,
    ctx: EventContext,
) -> Result<EdgeId> {
    let mut tx = begin_scoped(pool).await?;
    let edge = assert_kernel_edge_in_tx(&mut tx, p, ctx).await?;
    tx.commit().await?;
    Ok(edge)
}

/// In-transaction variant of [`assert_kernel_edge`] — fires on a caller-supplied connection (no
/// begin/commit). `ctx` correlates the `relationship_asserted` act to the reconcile run
/// (`EventContext::default()` for an un-attributed assert).
pub async fn assert_kernel_edge_in_tx(
    conn: &mut sqlx::PgConnection,
    p: KernelEdgeParams<'_>,
    ctx: EventContext,
) -> Result<EdgeId> {
    let edge = fire_with(
        conn,
        SeedAction::RelationshipAssert {
            src: payloads::AnchorRef::resource(p.src),
            tgt: payloads::AnchorRef::resource(p.tgt),
            kind: p.kind,
            polarity: p.polarity,
            label: p.label,
            weight: p.weight,
            home: EdgeHome::Cogmap(p.cogmap),
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .relationship()?;
    Ok(edge)
}

// ── relationship writes ──────────────────────────────────────────────────────────

/// Assert (or idempotently re-assert) an edge `src → tgt`, returning its id.
#[derive(Debug)]
pub struct AssertParams<'a> {
    pub src: ResourceId,
    pub tgt: ResourceId,
    pub kind: EdgeKind,
    pub polarity: EdgePolarity,
    pub label: Option<&'a str>,
    pub weight: f64,
    pub home: ContextId,
    pub emitter: EntityId,
}

pub async fn assert_relationship(pool: &PgPool, p: AssertParams<'_>) -> Result<EdgeId> {
    assert_relationship_with(pool, p, EventContext::default()).await
}

/// [`assert_relationship`] under an explicit [`EventContext`] — the authored `relationship_asserted`
/// act carries the caller's authorship + invocation correlator. Mirrors `fire`/`fire_with`.
pub async fn assert_relationship_with(
    pool: &PgPool,
    p: AssertParams<'_>,
    ctx: EventContext,
) -> Result<EdgeId> {
    let mut tx = begin_scoped(pool).await?;
    let edge = fire_with(
        &mut tx,
        SeedAction::RelationshipAssert {
            src: payloads::AnchorRef::resource(p.src),
            tgt: payloads::AnchorRef::resource(p.tgt),
            kind: p.kind,
            polarity: p.polarity,
            label: p.label,
            weight: p.weight,
            home: EdgeHome::Context(p.home),
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .relationship()?;
    tx.commit().await?;
    Ok(edge)
}

/// Params for [`assert_anchored_edge_with`] — the polymorphic generalisation of
/// [`AssertParams`]: the source and target are whatever the payload's
/// [`payloads::AnchorRef`] admits as an edge endpoint (resources and cogmaps on the
/// incumbent paths, plus **blobs** since the kb_edges endpoint CHECK admitted `kb_blobs` —
/// D3: relations are ordinary edges).
#[derive(Debug)]
pub struct AssertAnchoredEdgeParams<'a> {
    pub source: payloads::AnchorRef,
    pub target: payloads::AnchorRef,
    pub kind: EdgeKind,
    pub polarity: EdgePolarity,
    pub label: Option<&'a str>,
    pub weight: f64,
    /// The edge's home anchor, supplied by the caller. As on every edge write, this
    /// function performs NO authorization — the caller gates first (the incumbent
    /// contract: the home is authorized as the same value it is written to).
    pub home: EdgeHome,
    pub emitter: EntityId,
}

pub async fn assert_anchored_edge(
    pool: &PgPool,
    p: AssertAnchoredEdgeParams<'_>,
) -> Result<EdgeId> {
    assert_anchored_edge_with(pool, p, EventContext::default()).await
}

/// [`assert_anchored_edge`] under an explicit [`EventContext`] — the authored
/// `relationship_asserted` act carries the caller's authorship + invocation correlator,
/// exactly as [`assert_relationship_with`] does. One fire path for every endpoint pairing:
/// the payload's `source`/`target` tables ARE the endpoint tables the projector writes,
/// so a blob endpoint needs no new event type, no new SQL function, and no replay
/// divergence (the projector has been polymorphic since the canonical schema).
pub async fn assert_anchored_edge_with(
    pool: &PgPool,
    p: AssertAnchoredEdgeParams<'_>,
    ctx: EventContext,
) -> Result<EdgeId> {
    let mut tx = begin_scoped(pool).await?;
    let edge = fire_with(
        &mut tx,
        SeedAction::RelationshipAssert {
            src: p.source,
            tgt: p.target,
            kind: p.kind,
            polarity: p.polarity,
            label: p.label,
            weight: p.weight,
            home: p.home,
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .relationship()?;
    tx.commit().await?;
    Ok(edge)
}

pub async fn retype_relationship(
    pool: &PgPool,
    edge: EdgeId,
    kind: EdgeKind,
    polarity: EdgePolarity,
    emitter: EntityId,
) -> Result<()> {
    retype_relationship_with(pool, edge, kind, polarity, emitter, EventContext::default()).await
}

/// [`retype_relationship`] under an explicit [`EventContext`] — the `relationship_retyped` act is
/// correlated to the caller's invocation + stamped with its authorship. Mirrors `fire`/`fire_with`.
pub async fn retype_relationship_with(
    pool: &PgPool,
    edge: EdgeId,
    kind: EdgeKind,
    polarity: EdgePolarity,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    fire_with(
        &mut tx,
        SeedAction::RelationshipRetype {
            edge,
            kind,
            polarity,
            emitter,
        },
        ctx,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn reweight_relationship(
    pool: &PgPool,
    edge: EdgeId,
    weight: f64,
    emitter: EntityId,
) -> Result<()> {
    reweight_relationship_with(pool, edge, weight, emitter, EventContext::default()).await
}

/// [`reweight_relationship`] under an explicit [`EventContext`] — the `relationship_reweighted` act is
/// correlated to the caller's invocation + stamped with its authorship. Mirrors `fire`/`fire_with`.
pub async fn reweight_relationship_with(
    pool: &PgPool,
    edge: EdgeId,
    weight: f64,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    fire_with(
        &mut tx,
        SeedAction::RelationshipReweight {
            edge,
            weight,
            emitter,
        },
        ctx,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn fold_relationship(
    pool: &PgPool,
    edge: EdgeId,
    reason: Option<&str>,
    emitter: EntityId,
) -> Result<()> {
    fold_relationship_with(pool, edge, reason, emitter, EventContext::default()).await
}

/// [`fold_relationship`] under an explicit [`EventContext`] — the authored `relationship_folded` act
/// carries the caller's authorship + invocation correlator. Mirrors `fire`/`fire_with`.
pub async fn fold_relationship_with(
    pool: &PgPool,
    edge: EdgeId,
    reason: Option<&str>,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    fold_relationship_in_tx(&mut tx, edge, reason, emitter, ctx).await?;
    tx.commit().await?;
    Ok(())
}

/// In-transaction variant of [`fold_relationship`] — fires on a caller-supplied connection (no
/// begin/commit). `ctx` stamps the authored `relationship_folded` act (`EventContext::default()`
/// for an un-attributed fold).
pub async fn fold_relationship_in_tx(
    conn: &mut sqlx::PgConnection,
    edge: EdgeId,
    reason: Option<&str>,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    fire_with(
        conn,
        SeedAction::RelationshipFold {
            edge,
            reason,
            emitter,
        },
        ctx,
    )
    .await?;
    Ok(())
}

// ── invocation envelope ──────────────────────────────────────────────────────────

/// Parameters for opening an invocation. The invocation id is minted here and
/// returned (server-mint v1; caller-supplied ids for byte-exact durable-resume
/// re-issue are a deferred runtime concern).
#[derive(Debug)]
pub struct OpenParams {
    pub trigger_kind: String,
    pub originating: CogmapId,
    pub parent: Option<CogmapId>,
    pub scoped_entity: EntityId,
    pub emitter: EntityId,
}

/// Open an invocation envelope, returning the minted invocation id.
pub async fn open_invocation(pool: &PgPool, p: OpenParams) -> Result<InvocationId> {
    let mut tx = begin_scoped(pool).await?;
    let opened = open_invocation_in_tx(&mut tx, p).await?;
    tx.commit().await?;
    Ok(opened)
}

/// In-transaction variant of [`open_invocation`] — fires on a caller-supplied connection (no
/// begin/commit) so the open + the reconcile body + the close share ONE serializable transaction.
pub async fn open_invocation_in_tx(
    conn: &mut sqlx::PgConnection,
    p: OpenParams,
) -> Result<InvocationId> {
    let invocation = InvocationId::from(Uuid::now_v7());
    let opened = fire(
        conn,
        SeedAction::InvocationOpen {
            invocation,
            trigger_kind: &p.trigger_kind,
            originating: p.originating,
            parent: p.parent,
            scoped_entity: p.scoped_entity,
            emitter: p.emitter,
        },
    )
    .await?
    .invocation()?;
    Ok(opened)
}

/// Close an invocation with a terminal disposition + opaque outcome. The
/// originating cogmap is supplied by the caller (it knows it from the open /
/// from an auth lookup) so the `SeedAction` is constructed truthfully; the
/// substrate ignores it on close but the typed action requires it.
pub async fn close_invocation(
    pool: &PgPool,
    invocation: InvocationId,
    originating: CogmapId,
    disposition: payloads::Disposition,
    outcome: serde_json::Value,
    emitter: EntityId,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    close_invocation_in_tx(
        &mut tx,
        invocation,
        originating,
        disposition,
        outcome,
        emitter,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// In-transaction variant of [`close_invocation`] — fires on a caller-supplied connection (no
/// begin/commit).
pub async fn close_invocation_in_tx(
    conn: &mut sqlx::PgConnection,
    invocation: InvocationId,
    originating: CogmapId,
    disposition: payloads::Disposition,
    outcome: serde_json::Value,
    emitter: EntityId,
) -> Result<()> {
    fire(
        conn,
        SeedAction::InvocationClose {
            invocation,
            disposition,
            outcome,
            originating,
            emitter,
        },
    )
    .await?;
    Ok(())
}

// ── streaming/segmented ingest (Beat 1) ──────────────────────────────────────

/// Append one already-prepared block at `p.block.seq` to an existing resource — the
/// segmented-ingest write.
#[derive(Debug)]
pub struct AppendParams<'a> {
    pub resource: ResourceId,
    /// `seq` is authoritative (`block.seq`).
    pub block: &'a PreparedBlock,
    /// Sources this segment's content was incorporated from — recorded into
    /// `kb_block_provenance` by the projector. Empty for an ordinary append with no attribution.
    pub sources: Vec<Incorporation>,
    pub emitter: EntityId,
}

/// [`append_block`] under the default (un-attributed) context.
pub async fn append_block(pool: &PgPool, p: AppendParams<'_>) -> Result<BlockId> {
    append_block_with(pool, p, EventContext::default()).await
}

/// Append one already-prepared block at `p.block.seq` to an existing resource under an explicit
/// [`EventContext`] — the segmented-ingest write. Idempotent in SQL on (resource, seq, block
/// merkle): a re-append of the same segment is a no-op returning the existing block id.
pub async fn append_block_with(
    pool: &PgPool,
    p: AppendParams<'_>,
    ctx: EventContext,
) -> Result<BlockId> {
    // Carry resource-level sources onto the block manifest → kb_block_provenance.
    let mut block = p.block.clone();
    block.incorporated = p.sources;
    let mut tx = begin_scoped(pool).await?;
    let id = fire_with(
        &mut tx,
        SeedAction::BlockAppend {
            resource: p.resource,
            block: &block,
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .block()?;
    tx.commit().await?;
    Ok(id)
}

/// Parameters for [`finalize_ingest`].
#[derive(Debug)]
pub struct FinalizeParams {
    pub resource: ResourceId,
    pub expected_blocks: u32,
    pub expected_body_hash: String,
    /// Bare-hex sha256 of the full raw body — the byte-integrity check (W2 PR 5). `None` skips it
    /// (MCP is honestly exempt; a legacy caller predates the field).
    pub expected_content_hash: Option<String>,
    pub emitter: EntityId,
}

/// Declare a segmented ingest complete: validate the landed block set + body_hash
/// against the caller's expectation and record a `resource_finalized` event.
/// Projection-less (the ledger row is the whole effect) — calls `resource_finalize`
/// directly rather than through the `fire`/`SeedAction` surface, since there is no
/// projection half to keep in step with a typed `Fired` variant.
pub async fn finalize_ingest(pool: &PgPool, p: FinalizeParams) -> Result<EventId> {
    let payload = payloads::ResourceFinalized {
        resource_id: p.resource,
        expected_blocks: p.expected_blocks,
        expected_body_hash: p.expected_body_hash,
        expected_content_hash: p.expected_content_hash,
    };
    // Finalize and everything that must be atomic with "this resource is now complete" share one
    // transaction. `resource_finalize` is a plain plpgsql function (20260715000030 — it RAISEs
    // TF001/TF002/TF003 on mismatch and appends + projects inside its caller's tx), so wrapping it
    // in a scoped tx is behavior-preserving for the error paths (the raise rolls the whole thing
    // back, exactly as its own implicit tx did) — and it is what lets the write-path policy
    // application (see `apply_blocking_policy_in_tx`) run in the same atomic step: the resource
    // becomes complete and policy-partitioned in one commit, with no observer able to read a
    // complete resource whose partition contradicts policy.
    let mut tx = begin_scoped(pool).await?;
    let ev = sqlx::query_scalar!(
        "SELECT resource_finalize($1,$2,$3,$4)",
        serde_json::to_value(&payload)?,
        p.emitter.uuid(),
        serde_json::json!({}),
        Option::<Uuid>::None,
    )
    .fetch_one(&mut *tx)
    .await?
    .context("resource_finalize returned null")?;
    // Write-path policy application: a segmented upload lands policy-partitioned at the same
    // instant it becomes complete — the only observer-visible state is the committed one. The
    // finalize act itself is emitter-stamped without an authorship/correlation context (see the
    // `resource_finalize` call above: `{}` metadata, NULL invocation), so the re-block matches
    // that posture — never less attributed than the finalize it rides.
    apply_blocking_policy_in_tx(
        &mut tx,
        p.resource,
        p.emitter,
        EventContext::default(),
        Decline::Skip,
    )
    .await?;
    tx.commit().await?;
    Ok(EventId::from(ev))
}

/// Per-resource source-provenance record for [`upsert_ingestion_record`].
#[derive(Debug)]
pub struct IngestionRecord<'a> {
    pub resource: ResourceId,
    pub source_uri: &'a str,
    pub source_mimetype: Option<&'a str>,
    /// `"passthrough"` for raw markdown, `"kreuzberg"` for extraction.
    pub conversion_tool: &'a str,
    pub conversion_version: &'a str,
    /// sha256 of the source bytes, for resume integrity.
    pub source_hash: Option<&'a str>,
}

/// Upsert the per-resource source-provenance row (`kb_ingestion_records`, PK
/// resource_id) — its designed "ingestion idempotency" role, finally written. Holds
/// the source uri + hash the resume path checks the client's source against.
pub async fn upsert_ingestion_record(pool: &PgPool, r: IngestionRecord<'_>) -> Result<()> {
    sqlx::query!(
        "INSERT INTO kb_ingestion_records \
           (resource_id, source_uri, source_mimetype, conversion_tool, conversion_version, fetched_at, converted_at, source_hash) \
         VALUES ($1,$2,$3,$4,$5, now(), now(), $6) \
         ON CONFLICT (resource_id) DO UPDATE SET \
           source_uri = EXCLUDED.source_uri, source_mimetype = EXCLUDED.source_mimetype, \
           conversion_tool = EXCLUDED.conversion_tool, conversion_version = EXCLUDED.conversion_version, \
           converted_at = now(), source_hash = EXCLUDED.source_hash",
        r.resource.uuid(), r.source_uri, r.source_mimetype, r.conversion_tool,
        r.conversion_version, r.source_hash,
    )
    .execute(pool)
    .await?;
    Ok(())
}

// ── data artifact writes ──────────────────────────────────────────────────────

/// Parameters for [`commit_data_artifact_with`] — the attributed write that fires a
/// `DataArtifactCommit` seed action.
#[derive(Debug)]
pub struct CommitDataArtifactParams<'a> {
    pub resource: ResourceId,
    pub kind: &'a str,
    pub kind_owner: Option<payloads::KindOwner>,
    pub intent: payloads::ArtifactIntent,
    pub precedence: f64,
    pub content: &'a serde_json::Value,
    pub supersedes: &'a [DataArtifactId],
    pub emitter: EntityId,
}

/// [`commit_data_artifact_with`] under the default (un-attributed) context.
pub async fn commit_data_artifact(
    pool: &PgPool,
    p: CommitDataArtifactParams<'_>,
) -> Result<DataArtifactId> {
    commit_data_artifact_with(pool, p, EventContext::default()).await
}

/// Commit one data artifact to a resource under an explicit [`EventContext`]. Opens one
/// transaction, fires the `DataArtifactCommit` seed action (which hashes the content, calls
/// `data_artifact_commit()` SQL, and returns the new artifact id), and commits.
pub async fn commit_data_artifact_with(
    pool: &PgPool,
    p: CommitDataArtifactParams<'_>,
    ctx: EventContext,
) -> Result<DataArtifactId> {
    let mut tx = begin_scoped(pool).await?;
    let id = fire_with(
        &mut tx,
        SeedAction::DataArtifactCommit {
            resource: p.resource,
            kind: p.kind,
            kind_owner: p.kind_owner,
            intent: p.intent,
            precedence: p.precedence,
            content: p.content,
            supersedes: p.supersedes,
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .data_artifact()?;
    tx.commit().await?;
    Ok(id)
}

// ── data-artifact shape writes ────────────────────────────────────────────────

/// Parameters for [`declare_shape_with`] — the attributed write that fires a `ShapeDeclare`
/// seed action.
#[derive(Debug)]
pub struct DeclareShapeParams<'a> {
    pub home: payloads::AnchorRef,
    pub kind: &'a str,
    pub kind_owner: Option<payloads::KindOwner>,
    pub schema: &'a serde_json::Value,
    pub enforcement: payloads::EnforcementMode,
    pub emitter: EntityId,
}

// ── blob writes (spec: binary blobs, 2026-09-01) ─────────────────────────────

/// Parameters for [`commit_blob_with`] — the attributed write that fires a `BlobCommit` seed
/// action. The bytes are already in object storage at the content-addressed pathname; this
/// carries the hash, never the bytes.
#[derive(Debug)]
pub struct CommitBlobParams<'a> {
    /// Caller-minted identity (identity-as-input, D2).
    pub id: BlobId,
    /// The blob's home — a context or cogmap anchor; anything else is refused.
    pub home: payloads::AnchorRef,
    pub owner: ProfileId,
    /// Absent ⇒ originator≡owner.
    pub originator: Option<ProfileId>,
    /// Bare sha256 hex of the bytes already uploaded. The dedup key and the erasure join key.
    pub content_hash: String,
    pub content_type: String,
    pub content_bytes: i64,
    /// The D9 cap — configuration the caller supplies, passed through so the refusal teaches
    /// from the same values that enforce.
    pub max_bytes: i64,
    /// The D9 allowlist — same passage as `max_bytes`.
    pub allowlist: &'a [String],
    pub emitter: EntityId,
}

/// [`commit_blob_with`] under the default (un-attributed) context. The store is `&dyn` —
/// the surfaces hold an `Arc<dyn BlobStore>` (AppState), and the `&impl` spelling refused
/// that coercion, so the fire path takes the trait object its only surface caller has.
pub async fn commit_blob(
    pool: &PgPool,
    store: &dyn crate::blob_store::BlobStore,
    p: CommitBlobParams<'_>,
) -> Result<BlobId> {
    commit_blob_with(pool, store, p, EventContext::default()).await
}

/// Commit one blob under an explicit [`EventContext`]. Verifies the provider object exists at
/// the content-addressed pathname FIRST (D4's gate — a commit whose bytes are absent from the
/// provider is refused before the ledger sees it), then opens one transaction, fires the
/// `BlobCommit` seed action (which derives the pathname, calls `blob_commit()` SQL, and returns
/// the row id — the EXISTING id on a dedup hit within the commit's own home, D2 as amended),
/// and commits.
pub async fn commit_blob_with(
    pool: &PgPool,
    store: &dyn crate::blob_store::BlobStore,
    p: CommitBlobParams<'_>,
    ctx: EventContext,
) -> Result<BlobId> {
    let pathname = crate::blob_store::blob_pathname(&p.content_hash);
    anyhow::ensure!(
        store.exists(&pathname).await?,
        "blob_commit: the provider holds no object at {pathname} — upload the bytes before \
         committing the event; the ledger verifies presence, it does not take it on faith"
    );

    let mut tx = begin_scoped(pool).await?;
    let id = fire_with(
        &mut tx,
        SeedAction::BlobCommit {
            id: p.id,
            home: p.home,
            owner: p.owner,
            originator: p.originator,
            content_hash: p.content_hash,
            content_type: p.content_type,
            content_bytes: p.content_bytes,
            max_bytes: p.max_bytes,
            allowlist: p.allowlist,
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .blob()?;
    tx.commit().await?;
    Ok(id)
}

/// What a [`delete_blob_with`] strike did: the struck row, whether the provider bytes at
/// the pathname are releasable, and the pathname to delete them at when they are.
#[derive(Debug)]
pub struct StruckBlob {
    pub blob: BlobId,
    /// The same-transaction live-row refcount's verdict: `true` when the struck row was the
    /// LAST live row carrying its content hash — delete the provider bytes after the commit.
    /// `false` means another live home still references them; the row empties, the bytes stay.
    ///
    /// **The concurrency contract, stated honestly.** Strikes and commits on one hash
    /// serialize on a hash-keyed transaction advisory lock, so the refcount's snapshot is
    /// never stale against a concurrent strike or a concurrent commit's get-or-create —
    /// `released` is exact as of the strike's commit. What NO transaction can close is the
    /// window AFTER that commit: the caller's provider delete lands when it lands, and a
    /// commit whose own presence check ran earlier can insert a live row in between. That
    /// window is the register's declared-open rate axis, and it heals on re-upload (the
    /// re-commit re-puts the bytes at the same content-addressed pathname); the erasure
    /// build's queue fence (retry + age alerting) is what watches the residue. A build that
    /// deletes bytes MUST run that fence or its equivalent.
    pub released: bool,
    /// The content-addressed pathname the bytes live at — always present: an already-struck
    /// row is refused by the wrapper, never returned as a no-op.
    pub pathname: String,
}

/// [`delete_blob_with`] under the default (un-attributed) context.
pub async fn delete_blob(pool: &PgPool, blob: BlobId, emitter: EntityId) -> Result<StruckBlob> {
    delete_blob_with(pool, blob, emitter, EventContext::default()).await
}

/// Strike one blob under an explicit [`EventContext`] — the shared emptying act (ruled
/// 2026-09-06): one transaction appends the act's event, empties the row into the D5.2 shape
/// (pathname/type/bytes nulled; hash, home and owner kept), decides the byte fate from the
/// same-transaction live-row refcount, and touches NO edge (a folded relation reads as
/// deliberately ended — the strike's relations render absent because the blob is gone, never
/// because they were ended). The refcount is same-transaction, never a pre-count, and counts
/// LIVE rows only — struck rows never hold bytes hostage. The provider bytes are deleted by
/// the CALLER after the commit, at `pathname`, when `released` — a provider call cannot join
/// the transaction, so a crash between the two leaves the row emptied (unreachable through
/// every read path) and the bytes swept later by the erasure queue (derive-don't-remember).
pub async fn delete_blob_with(
    pool: &PgPool,
    blob: BlobId,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<StruckBlob> {
    let mut tx = begin_scoped(pool).await?;
    let struck = delete_blob_in_tx(&mut tx, blob, emitter, ctx).await?;
    tx.commit().await?;
    Ok(struck)
}

/// In-transaction variant of [`delete_blob`] — fires on a caller-supplied connection (no
/// begin/commit), so a door can run its standing checks and the strike in ONE transaction.
pub async fn delete_blob_in_tx(
    conn: &mut sqlx::PgConnection,
    blob: BlobId,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<StruckBlob> {
    let (blob, released, pathname) = fire_with(conn, SeedAction::BlobDelete { blob, emitter }, ctx)
        .await?
        .blob_strike()?;
    Ok(StruckBlob {
        blob,
        released,
        pathname,
    })
}

/// [`declare_shape_with`] under the default (un-attributed) context.
pub async fn declare_shape(pool: &PgPool, p: DeclareShapeParams<'_>) -> Result<ShapeId> {
    declare_shape_with(pool, p, EventContext::default()).await
}

/// Declare a shape for a data-artifact family within one home under an explicit [`EventContext`].
/// Opens one transaction, fires the `ShapeDeclare` seed action (which calls
/// `data_artifact_shape_declare()` SQL, defaulting the namespace and computing the chain-depth
/// version before appending the event), and commits.
pub async fn declare_shape_with(
    pool: &PgPool,
    p: DeclareShapeParams<'_>,
    ctx: EventContext,
) -> Result<ShapeId> {
    let mut tx = begin_scoped(pool).await?;
    let id = fire_with(
        &mut tx,
        SeedAction::ShapeDeclare {
            home: p.home,
            kind: p.kind,
            kind_owner: p.kind_owner,
            schema: p.schema,
            enforcement: p.enforcement,
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .shape()?;
    tx.commit().await?;
    Ok(id)
}

#[cfg(test)]
mod reblock_tests {
    use super::*;
    use temper_ingest::chunk::chunk_markdown;

    fn sha(hashes: &[&str]) -> String {
        temper_ingest::merkle::block_merkle(
            &hashes.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
    }

    /// Build a live projection from per-block parts: each part is (verbatim bytes, how many of
    /// the whole-body chunk run it holds). Block bodies concat to `body`, chunks are assigned in
    /// canonical (seq, index) order, and each block's derived hash is merkle over its own run —
    /// exactly the state the op's SQL reads produce.
    fn build_fixture(parts: &[(&str, usize)]) -> (String, Vec<LiveBlock>, Vec<LiveChunk>) {
        let body: String = parts.iter().map(|(bytes, _)| bytes.to_string()).collect();
        let chunks = chunk_markdown(&body);
        let mut blocks = Vec::new();
        let mut live = Vec::new();
        let mut pos = 0;
        for (seq, (bytes, count)) in parts.iter().enumerate() {
            let id = Uuid::now_v7();
            let run = &chunks[pos..pos + count];
            pos += count;
            for c in run {
                live.push(LiveChunk {
                    id: Uuid::now_v7(),
                    block_id: id,
                    content_hash: c.content_hash.clone(),
                });
            }
            blocks.push(LiveBlock {
                id,
                seq: seq as i32,
                body_hash: Some(sha(&run
                    .iter()
                    .map(|c| c.content_hash.as_str())
                    .collect::<Vec<_>>())),
                bytes: Some(bytes.to_string()),
            });
        }
        (body, blocks, live)
    }

    const SECTION_A: &str = "# Alpha\n\nalpha body\n";
    const SECTION_B: &str = "## Beta\n\nbeta body\n";

    #[test]
    fn a_single_unsectioned_block_is_a_no_op() {
        let body = "Just some plain text.\nNo headings here.\n";
        let (body, blocks, chunks) = build_fixture(&[(body, 1)]);
        let plan = compute_reblock_partition(
            ResourceId::from(Uuid::now_v7()),
            &body,
            &blocks,
            &chunks,
            &[],
        )
        .unwrap();
        assert!(
            matches!(plan, Partition::NoOp),
            "one body, one identical block: nothing to do"
        );
    }

    #[test]
    fn already_section_aligned_blocks_are_a_no_op_in_place() {
        let (body, blocks, chunks) = build_fixture(&[(SECTION_A, 1), (SECTION_B, 1)]);
        let plan = compute_reblock_partition(
            ResourceId::from(Uuid::now_v7()),
            &body,
            &blocks,
            &chunks,
            &[],
        )
        .unwrap();
        assert!(
            matches!(plan, Partition::NoOp),
            "every section keeps its own block at its own seq: the ledger must not hear about it"
        );
    }

    #[test]
    fn kept_blocks_with_moved_seqs_still_fire() {
        let (body, mut blocks, chunks) = build_fixture(&[(SECTION_A, 1), (SECTION_B, 1)]);
        // Same two identity-preserving sections, different slots: the partition is NOT the live
        // one, so the op must fire (kept entries carrying the NEW seqs).
        blocks[0].seq = 1;
        blocks[1].seq = 0;
        blocks.reverse(); // the op reads blocks in seq order
        let plan = compute_reblock_partition(
            ResourceId::from(Uuid::now_v7()),
            &body,
            &blocks,
            &chunks,
            &[],
        )
        .unwrap();
        let plan = match plan {
            Partition::Plan(plan) => plan,
            other => panic!("expected a plan, got {other:?}"),
        };
        assert_eq!(plan.manifest.kept.len(), 2);
        assert_eq!(plan.manifest.kept[0].seq, 0, "sections keep document order");
        assert!(plan.manifest.created.is_empty() && plan.manifest.folded.is_empty());
    }

    #[test]
    fn one_block_holding_two_sections_splits() {
        let whole = format!("{SECTION_A}{SECTION_B}");
        let (body, blocks, chunks) = build_fixture(&[(whole.as_str(), 2)]);
        let plan = compute_reblock_partition(
            ResourceId::from(Uuid::now_v7()),
            &body,
            &blocks,
            &chunks,
            &[],
        )
        .unwrap();
        let plan = match plan {
            Partition::Plan(plan) => plan,
            other => panic!("expected a plan, got {other:?}"),
        };
        assert_eq!(plan.manifest.folded, vec![BlockId::from(blocks[0].id)]);
        assert_eq!(plan.manifest.created.len(), 2);
        assert!(plan.manifest.kept.is_empty());
        let counts: Vec<usize> = plan
            .manifest
            .created
            .iter()
            .map(|c| c.chunks.len())
            .collect();
        assert_eq!(counts, vec![1, 1], "one chunk per section at this size");
        // chunk_index renumbers within the NEW block, and the assignment draws EXISTING ids.
        for c in &plan.manifest.created {
            for (i, chunk) in c.chunks.iter().enumerate() {
                assert_eq!(chunk.chunk_index, i as i32);
                assert!(chunks.iter().any(|l| l.id == chunk.chunk_id.uuid()));
            }
        }
        // The created slices' concatenation IS the body — the sidecar stores byte-exact parts.
        let mut composed = String::new();
        for (_, text) in &plan.slices {
            composed.push_str(text);
        }
        assert_eq!(composed, body);
    }

    #[test]
    fn a_mismatched_chunk_sequence_is_a_refusal_not_a_plan() {
        let (body, mut blocks, mut chunks) = build_fixture(&[(SECTION_A, 1)]);
        chunks[0].content_hash = "deadbeef".repeat(8);
        blocks[0].body_hash = Some(sha(&["deadbeef".repeat(8).as_str()]));
        let decision = compute_reblock_partition(
            ResourceId::from(Uuid::now_v7()),
            &body,
            &blocks,
            &chunks,
            &[],
        )
        .unwrap();
        let declined = match decision {
            Partition::Declined(reason) => reason,
            other => panic!("expected a decline, got {other:?}"),
        };
        assert!(
            declined.contains("does not match a fresh chunking"),
            "the decline names the refusal: {declined}"
        );
    }

    #[test]
    fn an_identical_second_section_whose_incumbent_hash_drifts_creates_a_twin() {
        // Two IDENTICAL sections; only the first has an incumbent whose DERIVED hash matches.
        // The second section's run hashes identically (duplicate paragraphs) but identity is
        // claimed by derived-hash equality against an UNCLAIMED block — the twin is created
        // over the second run of chunk ids, never re-asserted onto the first block. The drift
        // (a stale derived hash) is what forces the second section out of keep.
        let (body, blocks, chunks) = build_fixture(&[(SECTION_A, 1), (SECTION_A, 1)]);
        let mut blocks = blocks;
        blocks[1].body_hash = Some("drifted".to_string());
        let plan = compute_reblock_partition(
            ResourceId::from(Uuid::now_v7()),
            &body,
            &blocks,
            &chunks,
            &[],
        )
        .unwrap();
        let plan = match plan {
            Partition::Plan(plan) => plan,
            other => panic!("expected a plan, got {other:?}"),
        };
        assert_eq!(plan.manifest.kept.len(), 1, "section one keeps block one");
        assert_eq!(plan.manifest.created.len(), 1, "section two creates a twin");
        assert_eq!(plan.manifest.folded, vec![BlockId::from(blocks[1].id)]);
        assert_ne!(
            plan.manifest.created[0].block_id, plan.manifest.kept[0].block_id,
            "the twin is a NEW row, never the first block re-asserted"
        );
    }

    #[test]
    fn split_of_an_attributed_block_carries_to_both_created_halves() {
        let whole = format!("{SECTION_A}{SECTION_B}");
        let (body, blocks, chunks) = build_fixture(&[(whole.as_str(), 2)]);
        let source = ProvenanceSource::Resource(Uuid::now_v7());
        let attributions = vec![AttributionRow {
            block_id: blocks[0].id,
            source: source.clone(),
            accretion_seq: 3,
        }];
        let plan = compute_reblock_partition(
            ResourceId::from(Uuid::now_v7()),
            &body,
            &blocks,
            &chunks,
            &attributions,
        )
        .unwrap();
        let plan = match plan {
            Partition::Plan(plan) => plan,
            other => panic!("expected a plan, got {other:?}"),
        };
        assert_eq!(plan.manifest.created.len(), 2);
        for c in &plan.manifest.created {
            assert_eq!(c.attribution.len(), 1, "each half gets the source");
            assert_eq!(c.attribution[0].source, source);
            assert_eq!(c.attribution[0].seq, 3, "accretion order is preserved");
            assert!(
                c.attribution[0].carried,
                "a split copy is CARRIED, never direct"
            );
        }
    }

    #[test]
    fn absorbed_attribution_unions_as_asserted_and_kept_rows_are_never_relisted() {
        // Block one = section A (kept, asserted source S1). Block two = section B (hash tampered
        // so its section is CREATED) carrying S2: section B ABSORBS block two's whole run, so S2
        // unions as carried = false — and neither S1 nor S2 is re-listed onto the kept row.
        let (body, blocks, chunks) = build_fixture(&[(SECTION_A, 1), (SECTION_B, 1)]);
        let s1 = ProvenanceSource::Resource(Uuid::now_v7());
        let s2 = ProvenanceSource::Resource(Uuid::now_v7());
        let mut blocks = blocks;
        blocks[1].body_hash = Some("drifted".to_string());
        let attributions = vec![
            AttributionRow {
                block_id: blocks[0].id,
                source: s1.clone(),
                accretion_seq: 0,
            },
            AttributionRow {
                block_id: blocks[1].id,
                source: s2.clone(),
                accretion_seq: 1,
            },
        ];
        let plan = compute_reblock_partition(
            ResourceId::from(Uuid::now_v7()),
            &body,
            &blocks,
            &chunks,
            &attributions,
        )
        .unwrap();
        let plan = match plan {
            Partition::Plan(plan) => plan,
            other => panic!("expected a plan, got {other:?}"),
        };
        assert_eq!(plan.manifest.kept.len(), 1);
        assert_eq!(plan.manifest.created.len(), 1);
        let created = &plan.manifest.created[0];
        assert_eq!(created.attribution.len(), 1, "only the absorbed S2 rides");
        assert_eq!(created.attribution[0].source, s2);
        assert!(
            !created.attribution[0].carried,
            "an absorbed union is DIRECT"
        );
        assert_ne!(
            created.attribution[0].source, s1,
            "the kept block's own source is never re-listed under the new event"
        );
    }

    /// The reachability AC, made executable: the re-block op must have ZERO production callers
    /// outside this file — it is reachable only through the gated write paths
    /// (`create_resource` / `update_resource` / `finalize_ingest`, each dispatched behind the
    /// DbBackend gate train). Enforced by grep over every crate's `src/` tree rather than by
    /// trusting a maintained allowlist (the `assert_every_compiled_in_doc_is_vetoed` precedent:
    /// derive the set, never list it). Test trees are deliberately not scanned — the substrate
    /// witnesses invoke the op directly.
    #[test]
    fn reblock_op_is_reachable_only_through_the_gated_write_paths() {
        let crates_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root")
            .join("crates");
        // The op (writes.rs) and the substrate's own fire plumbing (events.rs, whose
        // `_event_append` call reaches the SQL wrapper) are the only legitimate homes.
        let allowed: &[std::path::PathBuf] = &[
            std::path::PathBuf::from("temper-substrate/src/writes.rs"),
            std::path::PathBuf::from("temper-substrate/src/events.rs"),
        ];
        let mut offenders = Vec::new();
        for crate_dir in std::fs::read_dir(&crates_dir).expect("crates/ must exist") {
            let src = crate_dir.expect("dir entry").path().join("src");
            if !src.is_dir() {
                continue;
            }
            let mut stack = vec![src];
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(&dir).expect("walk src") {
                    let path = entry.expect("dir entry").path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.extension().is_some_and(|e| e == "rs")
                        && !allowed.iter().any(|a| path.ends_with(a))
                        && std::fs::read_to_string(&path)
                            .map(|s| {
                                // The Rust op AND the SQL entry wrapper carrying the same fold
                                // semantics — either called from outside the substrate's own
                                // write/fire plumbing is a bypass.
                                s.contains("reblock_resource") || s.contains("resource_reblock(")
                            })
                            .unwrap_or(false)
                    {
                        offenders.push(path);
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "reblock_resource/resource_reblock must stay reachable ONLY through the gated write \
             paths (temper-substrate/src/writes.rs + events.rs); production callers found: {offenders:?}"
        );
    }
}
