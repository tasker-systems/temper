//! The erasure act's service layer — execute + refuse (spec 2026-08-31, task 01a0577c Beat 2).
//!
//! THE DOOR IS OPERATOR-ONLY (ruled 2026-09-09): the instance-level operator — the
//! `is_system_admin` standing — is the ONLY executor. A subject (or any non-operator) asking
//! for an erasure through this door gets the `unauthorized` refusal face, RECORDED (D6);
//! self-serve is a non-goal of this build. The gate resolves BEFORE any mutation
//! (authz-before-writes), and the refusal is the only mutation an unauthorized attempt makes.
//!
//! SQL commits, it does not decide legality (`principal_standing_apply`'s shape,
//! migrations/20260720000030): the scope computation, the per-row governed-home strikes, the
//! tombstone machinery and both events live in `principal_erasure_execute` /
//! `principal_erasure_refuse` / `_erasure_apply_redaction` (migration 20260909000025). The
//! admin surface's HTTP door (`handlers::erasure::execute`) calls straight into
//! [`execute_erasure`] — this module stays the service layer and carries no HTTP types.
//!
//! The request reference is an opaque UUID supplied by the caller: it rides
//! `kb_events."references"` (rel `request`) and the act's correlation id — never the payload —
//! so the request-to-person mapping stays in the operator's DSAR records, outside the ledger.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_substrate::payloads::{ErasureRefusalReason, ErasureTargetOutcome};
use temper_substrate::writes::resolve_emitter;
use temper_workflow::operations::Surface;

use crate::error::{ApiError, ApiResult};
use crate::services::access_service;

/// One blob strike's verdict, exactly what the `blob_delete` wrapper returned. The provider
/// bytes themselves are not the service's business: a released verdict is drained by the
/// fence (`erasure_fence_service`), which derives its work from the `principal_erased`
/// payload (derive-don't-remember).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobStrikeOutcome {
    pub blob_id: Uuid,
    pub released: bool,
}

/// A completed erasure — the full act or, on a re-erase, the no-op completion (spec §5:
/// `already_erased`, every target reporting `already-erased`). Either way ONE
/// `principal_erased` event stands behind these values.
#[derive(Debug, Clone, PartialEq)]
pub struct ErasureCompletion {
    pub event_id: Uuid,
    /// The redacted set (D2): content hashes only — the governed-scope text hashes plus every
    /// hash actually struck. Team- and map-homed hashes are NOT here; they ride the targets
    /// as the named remainder.
    pub redacted_hashes: Vec<String>,
    /// Per-target outcomes and the named remainder (D6's accepted-in-part arm).
    pub targets: Vec<ErasureTargetOutcome>,
    pub blob_strikes: Vec<BlobStrikeOutcome>,
    /// True when the subject was already tombstoned: this completion erased nothing new.
    pub already_erased: bool,
}

/// A recorded refusal (D6): one `principal_erasure_refused` event, nothing else mutated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErasureRefusal {
    pub event_id: Uuid,
    pub reason: ErasureRefusalReason,
    pub detail: Option<String>,
}

/// What a door call actually did.
#[derive(Debug, Clone, PartialEq)]
pub enum ErasureOutcome {
    Completed(ErasureCompletion),
    Refused(ErasureRefusal),
}

/// The jsonb `principal_erasure_execute` returns, before mapping onto the typed outcome.
#[derive(Debug, serde::Deserialize)]
struct ExecuteOutcomeWire {
    event_id: Uuid,
    redacted_hashes: Vec<String>,
    targets: Vec<ErasureTargetOutcome>,
    already_erased: bool,
}

/// Execute the erasure act for `subject`.
///
/// Authority FIRST: a caller without the `is_system_admin` standing gets the `unauthorized`
/// refusal recorded and nothing else — the existence of the subject is never disclosed to a
/// caller the gate has already declined (existence checks come after the gate, operator-only).
///
/// The request reference tolerates retries: a retried call with the SAME reference re-executes
/// as a no-op completion on an already-erased subject (`already_erased`, spec §5). Correlation
/// is INDEXED, never unique (20260624000001_canonical_schema.sql:491) — the reference pairs
/// the act's events, it does not deduplicate the door.
pub async fn execute_erasure(
    pool: &PgPool,
    caller: ProfileId,
    subject: ProfileId,
    request_reference: Uuid,
    surface: Surface,
) -> ApiResult<ErasureOutcome> {
    // The emitter resolves on the pool, before the transaction opens (a read, not part of the
    // mutation) — and it is the CALLER's entity on both arms: an authorized act attributes to
    // the operator, a refused attempt attributes to whoever attempted it. Hard precondition,
    // the `slack_disconnect_service` shape: an unattributable authority act is worse than a
    // failed one. The surface rides from the door (`Surface::ApiHttp` from the HTTP door, the
    // one that exists today) so the ledger says where the act came from, never a hard-wired
    // guess that outlives its accuracy — the blob commit's S5 correction.
    let emitter = resolve_emitter(pool, caller, surface.marker())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let is_operator = access_service::is_system_admin(pool, caller).await?;

    if !is_operator {
        let refusal = refuse_erasure(
            pool,
            subject,
            caller,
            request_reference,
            ErasureRefusalReason::Unauthorized,
            None,
            surface,
        )
        .await?;
        return Ok(ErasureOutcome::Refused(refusal));
    }

    // Gate passed — NOW existence may be disclosed (and as an error, not a ledger row).
    let exists: Option<Uuid> = sqlx::query_scalar!(
        r#"SELECT id FROM kb_profiles WHERE id = $1"#,
        subject.uuid()
    )
    .fetch_optional(pool)
    .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound("profile not found".to_string()));
    }

    let raw = sqlx::query_scalar!(
        r#"SELECT principal_erasure_execute($1, $2, $3, $4) AS "outcome: serde_json::Value""#,
        subject.uuid(),
        caller.uuid(),
        emitter.uuid(),
        request_reference,
    )
    .fetch_one(pool)
    .await?
    .ok_or_else(|| ApiError::Internal("principal_erasure_execute returned no row".to_string()))?;
    let wire: ExecuteOutcomeWire = serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("erasure outcome shape: {e}")))?;

    // The per-row verdicts live in the payload's targets; the strikes are re-matched out of
    // them so the typed outcome can carry what a fence needs without re-deriving from prose.
    // The SQL folded `blob_delete`'s verdict into each strike's outcome string; the
    // authoritative per-row record is the `blob_erased` events, which share the completion's
    // correlation id.
    let blob_strikes = strike_verdicts(pool, wire.event_id).await?;

    Ok(ErasureOutcome::Completed(ErasureCompletion {
        event_id: wire.event_id,
        redacted_hashes: wire.redacted_hashes,
        targets: wire.targets,
        blob_strikes,
        already_erased: wire.already_erased,
    }))
}

/// The per-row strike verdicts for a completion: every `blob_erased` event sharing the
/// completion's correlation id, joined to the row it emptied. The wrapper decided the byte
/// fate inside the act's transaction (live-rows-only refcount under the hash lock); this
/// re-derives the verdict from the row state the act left behind. The fence derives its own
/// pathname from the payload's per-target outcome prose — the STRUCK row's `blob_pathname` is
/// always NULL (the strike emptied it), so it is not read here.
async fn strike_verdicts(
    pool: &PgPool,
    completion_event: Uuid,
) -> ApiResult<Vec<BlobStrikeOutcome>> {
    let rows = sqlx::query!(
        r#"
        SELECT (e.payload->>'blob_id')::uuid       AS "blob_id: Uuid",
               NOT EXISTS (
                   SELECT 1 FROM kb_blobs live
                    WHERE live.content_hash = b.content_hash
                      AND live.content_type IS NOT NULL) AS "released: bool"
          FROM kb_events e
          JOIN kb_event_types t ON t.id = e.event_type_id AND t.name = 'blob_erased'
          JOIN kb_blobs b ON b.id = (e.payload->>'blob_id')::uuid
         WHERE e.correlation_id = (SELECT correlation_id FROM kb_events WHERE id = $1)
           AND e.id <> $1
         ORDER BY e.occurred_at, e.id
        "#,
        completion_event,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| BlobStrikeOutcome {
            blob_id: r.blob_id.expect("a blob_erased payload carries blob_id"),
            // Released ⟺ no live row carries the hash — the refcount's verdict, exact AS OF
            // THIS READ. The strike-time verdict is the payload's: a re-commit after the act
            // (the fence's declared-open window) flips this read to `false` while the
            // strike-time prose still says `released=true` — both are honest about the moment
            // they speak for, and neither overrules the other.
            released: r.released.unwrap_or(false),
        })
        .collect())
}

/// Record a refusal (D6) — the negative face: ONE `principal_erasure_refused` event with the
/// reason code and nothing else mutated.
///
/// This is the recording primitive, not a gated door: [`execute_erasure`]'s authority gate is
/// its ONLY caller — a non-operator's attempt is refused there, attributed to whoever
/// attempted it, and the HTTP door never calls this directly (it cannot: the refusal face
/// belongs to the gate, and a second call site would be a second legality decision). The door
/// does not pre-gate its callers either — the 404 posture is a RENDERING of this function's
/// recorded refusal, not a separate check. `pub(crate)` until a second door exists to call it;
/// widening it before then would invite a refusal path that bypasses the gate. `detail` carries
/// the reason's evidence — the named unhonourable part, or the obligation held — and must
/// never name a person (D6: the record never re-identifies). The refusal is attributed through
/// the caller's `surface`, the same provenance the execute arm rides — a refusal is an event
/// on the ledger too, and it names where the attempt came from.
pub(crate) async fn refuse_erasure(
    pool: &PgPool,
    subject: ProfileId,
    attempted_by: ProfileId,
    request_reference: Uuid,
    reason: ErasureRefusalReason,
    detail: Option<String>,
    surface: Surface,
) -> ApiResult<ErasureRefusal> {
    let emitter = resolve_emitter(pool, attempted_by, surface.marker())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let reason_str = serde_json::to_value(reason)
        .expect("a refusal reason always serializes")
        .as_str()
        .expect("a refusal reason serializes to a string")
        .to_string();

    let event_id: Uuid = sqlx::query_scalar!(
        r#"SELECT principal_erasure_refuse($1, $2, $3, $4, $5, $6) AS "event: Uuid""#,
        subject.uuid(),
        attempted_by.uuid(),
        emitter.uuid(),
        request_reference,
        reason_str,
        detail,
    )
    .fetch_one(pool)
    .await?
    .ok_or_else(|| ApiError::Internal("principal_erasure_refuse returned no row".to_string()))?;

    Ok(ErasureRefusal {
        event_id,
        reason,
        detail,
    })
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    use sqlx::PgPool;
    use uuid::Uuid;

    use super::*;
    use crate::services::grant_crypto::VaultKey;
    use crate::services::slack_grant_vault_service;
    use crate::services::slack_link_service;
    use crate::test_support;

    /// Minimal profile + its `<handle>@web` emitter entity. The handle is the FULL id:
    /// two UUIDv7s minted in the same millisecond share leading bytes, so a truncated
    /// handle collides on `kb_profiles_handle_key` (the slack fixture's rule).
    async fn insert_profile(pool: &PgPool) -> (Uuid, String) {
        let id = Uuid::now_v7();
        let handle = format!("user-{id}");
        sqlx::query(
            "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
                     VALUES ($1, $2, $2, $3, $4)",
        )
        .bind(id)
        .bind(&handle)
        .bind(format!("{handle}@x.test"))
        .bind(serde_json::json!({ "theme": "dark" }))
        .execute(pool)
        .await
        .expect("seed profile");
        sqlx::query("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("{handle}@web"))
            .execute(pool)
            .await
            .expect("seed emitter entity");
        (id, handle)
    }

    /// A governed (personal) context with a watermark to lose, one resource homed there with
    /// the full content stack (block revision + verbatim bytes + chunk + prose + FTS row),
    /// and a region (on the subject's own context) whose member is that resource — the
    /// watermark-bearing anchor set the act must mark.
    struct ContentWorld {
        context: Uuid,
        resource: Uuid,
        chunk: Uuid,
        revision: Uuid,
        chunk_hash: String,
        block_hash: String,
        event: Uuid,
    }

    async fn seed_content(pool: &PgPool, subject: Uuid) -> ContentWorld {
        let context = Uuid::now_v7();
        let resource = Uuid::now_v7();
        let block = Uuid::now_v7();
        let chunk = Uuid::now_v7();
        let revision = Uuid::now_v7();
        let chunk_hash = fake_sha('c');
        let block_hash = fake_sha('b');

        // The subject emits the resource's genesis event (idx_kb_events_emitter's half) and
        // carries every projection row the act's machinery touches.
        let event: Uuid = sqlx::query_scalar(
            "SELECT _event_append('resource_created', $1, 'kb_contexts', $2, \
                jsonb_build_object('resource_id', $3))",
        )
        .bind(emitter_of(pool, subject).await)
        .bind(context)
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("seed genesis event");

        sqlx::query(
            "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name, \
                     shape_materialized_event_id) \
                     VALUES ($1, 'kb_profiles', $2, 'notes', 'Notes', $3)",
        )
        .bind(context)
        .bind(subject)
        .bind(event)
        .execute(pool)
        .await
        .expect("seed personal context");
        sqlx::query(
            "INSERT INTO kb_resources (id, title, origin_uri) \
                     VALUES ($1, 'Secret plans', 'test://secret')",
        )
        .bind(resource)
        .execute(pool)
        .await
        .expect("seed resource");
        sqlx::query(
            "INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, \
                     originator_profile_id, owner_profile_id) \
                     VALUES ($1, 'kb_contexts', $2, $3, $3)",
        )
        .bind(resource)
        .bind(context)
        .bind(subject)
        .execute(pool)
        .await
        .expect("seed home");
        sqlx::query(
            "INSERT INTO kb_content_blocks (id, resource_id, seq, genesis_event_id, \
                     last_event_id) VALUES ($1, $2, 0, $3, $3)",
        )
        .bind(block)
        .bind(resource)
        .bind(event)
        .execute(pool)
        .await
        .expect("seed block");
        sqlx::query(
            "INSERT INTO kb_block_revisions (id, block_id, block_body_hash, chunk_count) \
                     VALUES ($1, $2, $3, 1)",
        )
        .bind(revision)
        .bind(block)
        .bind(&block_hash)
        .execute(pool)
        .await
        .expect("seed revision");
        sqlx::query(
            "INSERT INTO kb_block_content (block_revision_id, content, content_hash) \
                     VALUES ($1, $2, $3)",
        )
        .bind(revision)
        .bind("the secret plan bytes")
        .bind(&block_hash)
        .execute(pool)
        .await
        .expect("seed verbatim bytes");
        sqlx::query(
            "INSERT INTO kb_chunks (id, block_id, resource_id, chunk_index, version, \
                     content_hash, embedding, embedded_with) \
                     VALUES ($1, $2, $3, 0, 1, $4, $5::vector, 'model-sha')",
        )
        .bind(chunk)
        .bind(block)
        .bind(resource)
        .bind(&chunk_hash)
        .bind(embedding_literal())
        .execute(pool)
        .await
        .expect("seed chunk");
        sqlx::query(
            "INSERT INTO kb_chunk_content (chunk_id, content) \
                     VALUES ($1, 'the secret plan prose')",
        )
        .bind(chunk)
        .execute(pool)
        .await
        .expect("seed chunk prose");
        sqlx::query(
            "INSERT INTO kb_resource_search_index (resource_id, search_vector) \
                     VALUES ($1, to_tsvector('english', 'secret plans prose'))",
        )
        .bind(resource)
        .execute(pool)
        .await
        .expect("seed search index");

        ContentWorld {
            context,
            resource,
            chunk,
            revision,
            chunk_hash,
            block_hash,
            event,
        }
    }

    /// A DISTINCT, faithful 64-hex stand-in for a sha256 — the fixtures must look exactly
    /// like what `content_hash` carries, because the payload's shape IS bare hex.
    fn fake_sha(tag: char) -> String {
        let hex = format!("{}{}", Uuid::now_v7().simple(), Uuid::now_v7().simple());
        format!("{tag}{}", &hex[..63])
    }

    fn embedding_literal() -> String {
        format!("[{}]", vec!["0.1"; 768].join(","))
    }

    async fn emitter_of(pool: &PgPool, profile: Uuid) -> Uuid {
        sqlx::query_scalar(
            "SELECT e.id FROM kb_entities e WHERE e.profile_id = $1 \
                            AND e.name LIKE '%@web'",
        )
        .bind(profile)
        .fetch_one(pool)
        .await
        .expect("emitter entity")
    }

    /// A live blob row, seeded through the same event machinery the substrate writes with.
    async fn seed_blob(
        pool: &PgPool,
        owner: Uuid,
        home_table: &str,
        home_id: Uuid,
        tag: &str,
    ) -> (Uuid, String, String) {
        let blob = Uuid::now_v7();
        let hash = fake_sha(tag.chars().next().expect("tag"));
        let pathname = format!("{tag}/{}", &hash[..16.min(hash.len())]);
        let event: Uuid = sqlx::query_scalar(
            "SELECT _event_append('blob_committed', $1, $2, $3, \
                jsonb_build_object('blob_id', $4))",
        )
        .bind(emitter_of(pool, owner).await)
        .bind(home_table)
        .bind(home_id)
        .bind(blob)
        .fetch_one(pool)
        .await
        .expect("seed blob event");
        sqlx::query(
            "INSERT INTO kb_blobs (id, content_hash, blob_pathname, content_type, \
                     content_bytes, home_table, home_id, owner_profile_id, originator_profile_id, \
                     asserted_by_event_id, last_event_id) \
                     VALUES ($1, $2, $3, 'image/png', 10, $4, $5, $6, $6, $7, $7)",
        )
        .bind(blob)
        .bind(&hash)
        .bind(&pathname)
        .bind(home_table)
        .bind(home_id)
        .bind(owner)
        .bind(event)
        .execute(pool)
        .await
        .expect("seed blob row");
        (blob, hash, pathname)
    }

    /// A context OWNED by the subject's personal team (the trigger made the team at profile
    /// insert) — team governance, disposition iii's not-governed home.
    async fn seed_team_context(pool: &PgPool, handle: &str) -> Uuid {
        let context = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
                     SELECT $1, 'kb_teams', t.id, 'shared', 'Shared' \
                       FROM kb_teams t WHERE t.slug = 'personal-' || $2",
        )
        .bind(context)
        .bind(handle)
        .execute(pool)
        .await
        .expect("seed team context");
        context
    }

    /// Bind a Slack principal through the REAL write helpers, so the act's deletions hit the
    /// rows the link flow actually produces.
    async fn seed_slack(pool: &PgPool, subject: Uuid, principal: &str) {
        let mut conn = pool.acquire().await.expect("acquire");
        slack_link_service::link_slack_principal(&mut conn, subject, principal)
            .await
            .expect("link");
        let key = VaultKey::from_base64(&STANDARD.encode([3u8; 32])).expect("key");
        slack_grant_vault_service::store_grant(
            &mut conn,
            &key,
            slack_grant_vault_service::NewGrant {
                profile_id: subject,
                slack_principal_id: principal,
                refresh_token: "rt",
                access_token: "at",
                access_ttl_secs: Some(3600),
            },
        )
        .await
        .expect("store grant");
    }

    async fn count(pool: &PgPool, sql: &str) -> i64 {
        sqlx::query_scalar(sql)
            .fetch_one(pool)
            .await
            .expect("count")
    }

    /// Count rows bound to a UUID column — typed, because a bound string would read
    /// `text = uuid` and fail.
    async fn count_with(pool: &PgPool, sql: &str, id: Uuid) -> i64 {
        sqlx::query_scalar(sql)
            .bind(id)
            .fetch_one(pool)
            .await
            .expect("count")
    }

    /// The TEXT-column twin of [`count_with`]: `content_hash` is sha256 hex.
    async fn count_hash(pool: &PgPool, sql: &str, hash: String) -> i64 {
        sqlx::query_scalar(sql)
            .bind(hash)
            .fetch_one(pool)
            .await
            .expect("count")
    }

    /// ── WITNESS: the authority bite ──────────────────────────────────────────────────────
    /// FAILS IF a non-operator's attempt mutates anything: exactly ONE refusal event is the
    /// whole effect — the profile, the content, the blobs and the erased-content set are all
    /// untouched, and no strike event exists.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn a_non_operator_attempt_records_the_unauthorized_refusal_and_mutates_nothing(
        pool: sqlx::PgPool,
    ) {
        let (subject, _) = insert_profile(&pool).await;
        let (caller, _) = insert_profile(&pool).await;
        let world = seed_content(&pool, subject).await;
        let (blob, hash, _) = seed_blob(&pool, subject, "kb_contexts", world.context, "aa").await;
        let request = Uuid::now_v7();

        let outcome = execute_erasure(
            &pool,
            ProfileId::from(caller),
            ProfileId::from(subject),
            request,
            Surface::ApiHttp,
        )
        .await
        .expect("the door answers");

        let ErasureOutcome::Refused(refusal) = outcome else {
            panic!("a non-operator attempt must be refused, got {outcome:?}");
        };
        assert_eq!(refusal.reason, ErasureRefusalReason::Unauthorized);
        assert!(refusal.detail.is_none());

        // The refusal is RECORDED (D6) — and attributed to the attempter.
        let (reason, actor, unanchored): (String, Uuid, bool) = sqlx::query_as(
            "SELECT e.payload->>'reason', (e.payload->>'actor')::uuid, \
                    e.producing_anchor_table IS NULL \
               FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
              WHERE t.name = 'principal_erasure_refused'",
        )
        .fetch_one(&pool)
        .await
        .expect("the refusal event exists");
        assert_eq!(reason, "unauthorized");
        assert_eq!(actor, caller, "the attempt is attributed to the attempter");
        assert!(unanchored, "the refusal face is NULL-anchored (admin)");

        // …and NOTHING else mutated.
        let (handle, display, email, prefs, tomb): (
            String,
            String,
            Option<String>,
            serde_json::Value,
            Option<i32>,
        ) = sqlx::query_as(
            "SELECT handle, display_name, email, preferences, \
                    (tombstoned_at IS NOT NULL)::int AS tomb FROM kb_profiles WHERE id = $1",
        )
        .bind(subject)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(tomb, Some(0), "the profile is not tombstoned");
        assert!(email.is_some(), "the email survives a refused attempt");
        assert_ne!(handle, format!("erased-{subject}"));
        assert_ne!(display, format!("erased-{subject}"));
        assert_eq!(prefs, serde_json::json!({ "theme": "dark" }));
        assert_eq!(
            count(
                &pool,
                "SELECT count(*) FROM kb_chunk_content WHERE content <> ''"
            )
            .await,
            1,
            "chunk prose survives"
        );
        assert_eq!(
            count(
                &pool,
                "SELECT count(*) FROM kb_blobs WHERE content_type IS NOT NULL"
            )
            .await,
            1,
            "the blob row is still live"
        );
        assert_eq!(
            count_hash(
                &pool,
                "SELECT count(*) FROM kb_erased_content WHERE content_hash = $1",
                hash,
            )
            .await,
            0,
            "the erased-content set admits nothing on a refusal"
        );
        let strikes = count(
            &pool,
            "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
             WHERE t.name IN ('principal_erased', 'blob_erased')",
        )
        .await;
        assert_eq!(strikes, 0, "no completion and no strike exists");
        let _ = (world, blob);
    }

    /// ── WITNESS: the surface flows ──────────────────────────────────────────────────────
    /// FAILS WHILE the door hard-wires the `web` emitter: an authorized act executed
    /// through a named surface must be attributed to THAT surface's emitter entity —
    /// `execute_erasure` rides the door's `Surface`, and a parameter that cannot vary
    /// cannot be said to flow (the blob S5 rule). `Surface::CliCloud` is ridden because
    /// the HTTP door answers as `web` today; the second value is the proof the parameter
    /// moves.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn an_authorized_act_is_attributed_to_the_surface_it_ran_on(pool: sqlx::PgPool) {
        let (subject, _) = insert_profile(&pool).await;
        let (operator, handle) = insert_profile(&pool).await;
        // The `@cli` emitter the call names — `resolve_emitter` resolves against the
        // profile's `<handle>@<marker>` entity, which the fixture does not mint.
        sqlx::query("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2)")
            .bind(operator)
            .bind(format!("{handle}@cli"))
            .execute(&pool)
            .await
            .expect("seed cli emitter entity");
        test_support::grant_governance(&pool, operator).await;
        let world = seed_content(&pool, subject).await;

        let outcome = execute_erasure(
            &pool,
            ProfileId::from(operator),
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::CliCloud,
        )
        .await
        .expect("the operator's act completes");
        let ErasureOutcome::Completed(_) = outcome else {
            panic!("an operator's act must complete, got {outcome:?}");
        };

        let emitter: String = sqlx::query_scalar(
            "SELECT ent.name \
               FROM kb_events e \
               JOIN kb_event_types t ON t.id = e.event_type_id \
               JOIN kb_entities ent ON ent.id = e.emitter_entity_id \
              WHERE t.name = 'principal_erased'",
        )
        .fetch_one(&pool)
        .await
        .expect("the completion event with its emitter");
        assert_eq!(
            emitter,
            format!("{handle}@cli"),
            "the act is attributed to the surface it ran on, not a hard-wired web"
        );
        let _ = world;
    }

    /// ── WITNESS: the refusal arm rides the surface too ──────────────────────────────────
    /// FAILS WHILE the refusal emitter is hard-wired: a refused attempt is a RECORDED
    /// event (D6), and its emitter names where the attempt came from. A non-operator
    /// arriving over MCP gets the refusal attributed to `<handle>@mcp`, not to the web
    /// entity that used to take every erasure event.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn a_refused_attempt_is_attributed_to_the_surface_it_came_from(pool: sqlx::PgPool) {
        let (subject, _) = insert_profile(&pool).await;
        let (attempter, handle) = insert_profile(&pool).await;
        sqlx::query("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2)")
            .bind(attempter)
            .bind(format!("{handle}@mcp"))
            .execute(&pool)
            .await
            .expect("seed mcp emitter entity");

        let outcome = execute_erasure(
            &pool,
            ProfileId::from(attempter),
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::Mcp,
        )
        .await
        .expect("the door answers");
        let ErasureOutcome::Refused(r) = outcome else {
            panic!("a non-operator attempt must be refused, got {outcome:?}");
        };
        assert_eq!(r.reason, ErasureRefusalReason::Unauthorized);

        let emitter: String = sqlx::query_scalar(
            "SELECT ent.name \
               FROM kb_events e \
               JOIN kb_event_types t ON t.id = e.event_type_id \
               JOIN kb_entities ent ON ent.id = e.emitter_entity_id \
              WHERE t.name = 'principal_erasure_refused'",
        )
        .fetch_one(&pool)
        .await
        .expect("the refusal event with its emitter");
        assert_eq!(
            emitter,
            format!("{handle}@mcp"),
            "the recorded refusal names the surface the attempt came from"
        );
    }

    /// ── WITNESS: the personal-homed blob strike ─────────────────────────────────────────
    /// FAILS IF the strike does not go through the wrapper as a per-row `blob_erased` event:
    /// the row lands in the D5.2 shape, the verdict rides the payload's per-target outcomes,
    /// and the strike is correlated to the completion (the pairing is a fact, D1).
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn a_personal_homed_blob_is_struck_and_its_verdict_rides_the_payload(pool: sqlx::PgPool) {
        let (subject, _) = insert_profile(&pool).await;
        let (operator, _) = insert_profile(&pool).await;
        test_support::grant_governance(&pool, operator).await;
        let world = seed_content(&pool, subject).await;
        let (blob, hash, pathname) =
            seed_blob(&pool, subject, "kb_contexts", world.context, "aa").await;

        let outcome = execute_erasure(
            &pool,
            ProfileId::from(operator),
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("the operator's act completes");
        let ErasureOutcome::Completed(completion) = outcome else {
            panic!("an operator's act must complete, got {outcome:?}");
        };
        assert_eq!(
            completion.redacted_hashes.len(),
            3,
            "two text hashes + one blob hash"
        );

        // The row is in the D5.2 shape — and there is deliberately no marker of WHICH act
        // emptied it.
        let (p_path, p_type, p_hash, p_owner): (Option<String>, Option<String>, String, Uuid) =
            sqlx::query_as(
                "SELECT blob_pathname, content_type, content_hash, owner_profile_id \
                   FROM kb_blobs WHERE id = $1",
            )
            .bind(blob)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(p_path, None, "pathname nulled");
        assert_eq!(
            p_type, None,
            "content_type nulled — the ONLY emptiness marker"
        );
        assert_eq!(p_hash, hash, "the hash survives");
        assert_eq!(
            p_owner, subject,
            "attribution dies only at the pseudonym break"
        );

        // A per-row domain event exists, home-anchored, correlated to the completion.
        let (kind, anchor, corr_of_strike, corr_of_completion): (String, Uuid, Uuid, Uuid) =
            sqlx::query_as(
                "SELECT t.name, e.producing_anchor_id, e.correlation_id, \
                        (SELECT correlation_id FROM kb_events WHERE id = $2) \
                   FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
                  WHERE t.name = 'blob_erased' AND e.payload->>'blob_id' = $1::text",
            )
            .bind(blob)
            .bind(completion.event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(kind, "blob_erased");
        assert_eq!(anchor, world.context, "anchored at the blob's home");
        assert_eq!(corr_of_strike, corr_of_completion, "the pairing is a fact");

        // The verdict rides the payload's per-target outcomes.
        let strike = completion
            .targets
            .iter()
            .find(|t| t.target == "kb_blobs")
            .expect("a per-target outcome for the strike");
        assert!(
            strike.outcome.contains("released=true") && strike.outcome.contains(&pathname),
            "the wrapper's verdict rides the payload, got {strike:?}"
        );
        assert_eq!(
            completion.blob_strikes.len(),
            1,
            "the typed outcome carries the strike"
        );
    }

    /// ── WITNESS: the team-homed remainder ───────────────────────────────────────────────
    /// FAILS IF the strike arm reaches beyond governed homes: a blob homed in a team-OWNED
    /// context is byte-identical after the act, its hash is named in the payload's remainder
    /// (independent_obligation-shaped) and never admitted to the erased-content set.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn a_team_homed_blob_is_untouched_and_named_in_the_remainder(pool: sqlx::PgPool) {
        let (subject, handle) = insert_profile(&pool).await;
        let (operator, _) = insert_profile(&pool).await;
        test_support::grant_governance(&pool, operator).await;
        let world = seed_content(&pool, subject).await;
        let team_context = seed_team_context(&pool, &handle).await;
        let (blob, hash, pathname) =
            seed_blob(&pool, subject, "kb_contexts", team_context, "bb").await;

        let outcome = execute_erasure(
            &pool,
            ProfileId::from(operator),
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("completes");
        let ErasureOutcome::Completed(completion) = outcome else {
            panic!("must complete, got {outcome:?}");
        };

        // The row is byte-identical: content_type untouched, pathname untouched.
        let (p_path, p_type): (Option<String>, Option<String>) =
            sqlx::query_as("SELECT blob_pathname, content_type FROM kb_blobs WHERE id = $1")
                .bind(blob)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            (p_path.as_deref(), p_type.as_deref()),
            (Some(pathname.as_str()), Some("image/png")),
            "the team-homed row is untouched"
        );

        // Its hash rides the remainder, shaped so the subject receives the reason…
        let remainder = completion
            .targets
            .iter()
            .find(|t| t.outcome.contains(&hash))
            .expect("the remainder names the hash");
        assert!(
            remainder.outcome.starts_with("independent_obligation"),
            "the subject receives the reason, got {remainder:?}"
        );
        // …and never the redacted set, and never the refusal set's admission.
        assert!(
            !completion.redacted_hashes.contains(&hash),
            "a hash the act did not strike is not 'redacted'"
        );
        assert_eq!(
            count_hash(
                &pool,
                "SELECT count(*) FROM kb_erased_content WHERE content_hash = $1",
                hash,
            )
            .await,
            0,
            "the erased-content set must not refuse writes in homes the subject does not govern"
        );
        let _ = world;
    }

    /// ── WITNESS: the erasure never reaches beyond the subject's governed homes ──────────
    /// FAILS IF the text redaction expands by hash across homes: a second principal ingested
    /// the SAME bytes (identical chunk + block hashes) into their OWN governed context.
    /// Erasing the subject empties the subject's prose, vector and search vector — and
    /// leaves the other principal's byte-identical copy untouched (the offboarding ruling,
    /// 20260911000000; the register's negative face: another principal's lawful content is
    /// never an erasure violation). The 20260909000025 shape failed exactly here: its text
    /// arms keyed on hash membership alone, so this witness could not have passed.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn a_same_hash_resource_in_another_governed_home_survives_the_erasure(
        pool: sqlx::PgPool,
    ) {
        let (subject, _) = insert_profile(&pool).await;
        let (other, _) = insert_profile(&pool).await;
        let (operator, _) = insert_profile(&pool).await;
        test_support::grant_governance(&pool, operator).await;
        let world = seed_content(&pool, subject).await;

        // The other principal's resource: byte-identical content stack, own governed home,
        // the SAME hashes — the shared-template shape. Seeded with this file's own row
        // shapes (not through create_resource) so the hash identity is exact.
        let other_context = Uuid::now_v7();
        let other_resource = Uuid::now_v7();
        let other_block = Uuid::now_v7();
        let other_chunk = Uuid::now_v7();
        let other_revision = Uuid::now_v7();
        let other_event: Uuid = sqlx::query_scalar(
            "SELECT _event_append('resource_created', $1, 'kb_contexts', $2, \
                jsonb_build_object('resource_id', $3))",
        )
        .bind(emitter_of(&pool, other).await)
        .bind(other_context)
        .bind(other_resource)
        .fetch_one(&pool)
        .await
        .expect("seed other genesis event");
        sqlx::query(
            "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name, \
                     shape_materialized_event_id) \
                     VALUES ($1, 'kb_profiles', $2, 'notes', 'Notes', $3)",
        )
        .bind(other_context)
        .bind(other)
        .bind(other_event)
        .execute(&pool)
        .await
        .expect("seed other personal context");
        sqlx::query(
            "INSERT INTO kb_resources (id, title, origin_uri) \
                     VALUES ($1, 'Shared template', 'test://template')",
        )
        .bind(other_resource)
        .execute(&pool)
        .await
        .expect("seed other resource");
        sqlx::query(
            "INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, \
                     originator_profile_id, owner_profile_id) \
                     VALUES ($1, 'kb_contexts', $2, $3, $3)",
        )
        .bind(other_resource)
        .bind(other_context)
        .bind(other)
        .execute(&pool)
        .await
        .expect("seed other home");
        sqlx::query(
            "INSERT INTO kb_content_blocks (id, resource_id, seq, genesis_event_id, \
                     last_event_id) VALUES ($1, $2, 0, $3, $3)",
        )
        .bind(other_block)
        .bind(other_resource)
        .bind(other_event)
        .execute(&pool)
        .await
        .expect("seed other block");
        sqlx::query(
            "INSERT INTO kb_block_revisions (id, block_id, block_body_hash, chunk_count) \
                     VALUES ($1, $2, $3, 1)",
        )
        .bind(other_revision)
        .bind(other_block)
        .bind(&world.block_hash)
        .execute(&pool)
        .await
        .expect("seed other revision");
        sqlx::query(
            "INSERT INTO kb_block_content (block_revision_id, content, content_hash) \
                     VALUES ($1, $2, $3)",
        )
        .bind(other_revision)
        .bind("the secret plan bytes")
        .bind(&world.block_hash)
        .execute(&pool)
        .await
        .expect("seed other verbatim bytes");
        sqlx::query(
            "INSERT INTO kb_chunks (id, block_id, resource_id, chunk_index, version, \
                     content_hash, embedding, embedded_with) \
                     VALUES ($1, $2, $3, 0, 1, $4, $5::vector, 'model-sha')",
        )
        .bind(other_chunk)
        .bind(other_block)
        .bind(other_resource)
        .bind(&world.chunk_hash)
        .bind(embedding_literal())
        .execute(&pool)
        .await
        .expect("seed other chunk");
        sqlx::query(
            "INSERT INTO kb_chunk_content (chunk_id, content) VALUES ($1, 'the secret plan prose')",
        )
        .bind(other_chunk)
        .execute(&pool)
        .await
        .expect("seed other chunk prose");
        sqlx::query(
            "INSERT INTO kb_resource_search_index (resource_id, search_vector) \
                     VALUES ($1, to_tsvector('english', 'secret plans prose'))",
        )
        .bind(other_resource)
        .execute(&pool)
        .await
        .expect("seed other search index");

        let outcome = execute_erasure(
            &pool,
            ProfileId::from(operator),
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("completes");
        let ErasureOutcome::Completed(completion) = outcome else {
            panic!("must complete, got {outcome:?}");
        };
        assert!(
            completion.redacted_hashes.contains(&world.chunk_hash),
            "the shared hash is redacted — the subject's own copy is in scope"
        );

        // The subject's copy: emptied.
        let subject_prose: String =
            sqlx::query_scalar("SELECT cc.content FROM kb_chunk_content cc WHERE cc.chunk_id = $1")
                .bind(world.chunk)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(subject_prose, "", "the subject's prose is emptied");

        // THE WITNESS: the other principal's byte-identical copy is untouched — prose,
        // vector, provenance, block bytes, search vector.
        let other_prose: String =
            sqlx::query_scalar("SELECT cc.content FROM kb_chunk_content cc WHERE cc.chunk_id = $1")
                .bind(other_chunk)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            other_prose, "the secret plan prose",
            "a same-hash chunk in another governed home keeps its prose — the erasure never \
             reached beyond the subject's estate"
        );
        let (other_embedding, other_embedded_with): (Option<String>, Option<String>) =
            sqlx::query_as("SELECT embedding::text, embedded_with FROM kb_chunks WHERE id = $1")
                .bind(other_chunk)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            other_embedding.is_some() && other_embedded_with.as_deref() == Some("model-sha"),
            "the other home's chunk keeps its vector and provenance"
        );
        let other_bytes: String =
            sqlx::query_scalar("SELECT content FROM kb_block_content WHERE block_revision_id = $1")
                .bind(other_revision)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            other_bytes, "the secret plan bytes",
            "the other home's block bytes survive"
        );
        let other_fts: String = sqlx::query_scalar(
            "SELECT search_vector::text FROM kb_resource_search_index WHERE resource_id = $1",
        )
        .bind(other_resource)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            !other_fts.is_empty(),
            "the other home's search vector survives"
        );

        // CUSTODY CLOSURE (arm 13): the subject's governed context is retired — the
        // subject-liveness floor (20260902000010) then denies every live principal's access
        // into the estate, grants and all — while the other principal's context stays live.
        let (subject_ctx_active,): (bool,) =
            sqlx::query_as("SELECT is_active FROM kb_contexts WHERE id = $1")
                .bind(world.context)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            !subject_ctx_active,
            "the wiped estate's context is retired — custody over it is closed"
        );
        let (other_ctx_active,): (bool,) =
            sqlx::query_as("SELECT is_active FROM kb_contexts WHERE id = $1")
                .bind(other_context)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            other_ctx_active,
            "an ungoverned context is nobody's retirement target"
        );
        let retirement: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_events e \
               JOIN kb_event_types t ON t.id = e.event_type_id \
              WHERE t.name = 'principal_erased' \
                AND e.payload->'targets' @> '[{\"target\": \"kb_contexts.is_active\"}]'::jsonb",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            retirement, 1,
            "the retirement rides the record as a per-target outcome"
        );
        let _ = world.event;
    }

    /// ── WITNESS: the text content ───────────────────────────────────────────────────────
    /// FAILS IF the emptied shape is incomplete: chunk AND block prose emptied with hashes
    /// retained, embedding AND provenance nulled together, the search vector emptied, the
    /// erased-content set holding every redacted hash — and the formation watermark on the
    /// affected anchor nulled (the centroid recompute mark, D5).
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn text_content_is_emptied_hashes_kept_and_the_set_holds_every_hash(pool: sqlx::PgPool) {
        let (subject, _) = insert_profile(&pool).await;
        let (operator, _) = insert_profile(&pool).await;
        test_support::grant_governance(&pool, operator).await;
        let world = seed_content(&pool, subject).await;

        let outcome = execute_erasure(
            &pool,
            ProfileId::from(operator),
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("completes");
        let ErasureOutcome::Completed(completion) = outcome else {
            panic!("must complete, got {outcome:?}");
        };

        let (content, hash): (String, String) = sqlx::query_as(
            "SELECT cc.content, c.content_hash FROM kb_chunk_content cc \
               JOIN kb_chunks c ON c.id = cc.chunk_id WHERE cc.chunk_id = $1",
        )
        .bind(world.chunk)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(content, "", "the prose is emptied");
        assert_eq!(hash, world.chunk_hash, "the chunk hash is retained");

        let (emb, prov): (Option<String>, Option<String>) =
            sqlx::query_as("SELECT embedding::text, embedded_with FROM kb_chunks WHERE id = $1")
                .bind(world.chunk)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            (emb, prov),
            (None, None),
            "embedding + provenance null together"
        );

        let (bc, bh): (String, String) = sqlx::query_as(
            "SELECT content, content_hash FROM kb_block_content WHERE block_revision_id = $1",
        )
        .bind(world.revision)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(bc, "", "the verbatim bytes are emptied");
        assert_eq!(bh, world.block_hash, "the block hash is retained");

        // D3: body_hash is computed over hashes, never bytes — the act does not touch it.
        let (body_hash,): (String,) =
            sqlx::query_as("SELECT block_body_hash FROM kb_block_revisions WHERE id = $1")
                .bind(world.revision)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(body_hash, world.block_hash, "body_hash never changes");

        let empty_vector: bool = sqlx::query_scalar(
            "SELECT search_vector = ''::tsvector FROM kb_resource_search_index \
              WHERE resource_id = $1",
        )
        .bind(world.resource)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(empty_vector, "the search vector is emptied");

        for h in [&world.chunk_hash, &world.block_hash] {
            assert!(
                completion.redacted_hashes.contains(h),
                "every text hash is in the redacted set ({h})"
            );
            assert_eq!(
                count_hash(
                    &pool,
                    "SELECT count(*) FROM kb_erased_content WHERE content_hash = $1",
                    h.to_string(),
                )
                .await,
                1,
                "kb_erased_content holds {h}"
            );
        }

        // The centroid mark: the governed context's formation watermark is GONE.
        let watermark: Option<Uuid> =
            sqlx::query_scalar("SELECT shape_materialized_event_id FROM kb_contexts WHERE id = $1")
                .bind(world.context)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(watermark.is_none(), "the anchor is marked for recompute");
    }

    /// ── WITNESS: the centroid mark reaches maps the subject does not own ────────────────
    /// FAILS IF the recompute mark only covers the subject's own homes: a region on a
    /// STRANGER's context holding the subject's resource as a member must lose ITS
    /// formation watermark too — the centroid aggregating the erased member is the leak,
    /// regardless of whose map it lives on (D5).
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn the_region_member_arm_marks_anchors_the_subject_does_not_own(pool: sqlx::PgPool) {
        let (subject, _) = insert_profile(&pool).await;
        let (operator, _) = insert_profile(&pool).await;
        let (stranger, _) = insert_profile(&pool).await;
        test_support::grant_governance(&pool, operator).await;
        let world = seed_content(&pool, subject).await;

        // The stranger's own context, watermarked like any materialized anchor.
        let strangers_context = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name, \
                     shape_materialized_event_id) \
                     VALUES ($1, 'kb_profiles', $2, 'theirs', 'Theirs', $3)",
        )
        .bind(strangers_context)
        .bind(stranger)
        .bind(world.event)
        .execute(&pool)
        .await
        .expect("seed the stranger's context");

        // A region on it (global lens), holding the subject's resource as a member.
        let lens: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_cogmap_lenses (name, w_express, w_contains, w_leads_to, w_near, \
                w_prop, s_telos, s_ref, s_central, resolution, asserted_by_event_id) \
             VALUES ('probe', 1, 1, 1, 1, 1, 1, 1, 1, 1, $1) RETURNING id",
        )
        .bind(world.event)
        .fetch_one(&pool)
        .await
        .expect("seed lens");
        let region: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_cogmap_regions (home_anchor_table, home_anchor_id, lens_id, centroid, \
                salience, member_count, asserted_by_event_id, last_event_id) \
             VALUES ('kb_contexts', $1, $2, $3::vector, 0.5, 1, $4, $4) RETURNING id",
        )
        .bind(strangers_context)
        .bind(lens)
        .bind(embedding_literal())
        .bind(world.event)
        .fetch_one(&pool)
        .await
        .expect("seed region");
        sqlx::query(
            "INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id) \
                     VALUES ($1, 'kb_resources', $2)",
        )
        .bind(region)
        .bind(world.resource)
        .execute(&pool)
        .await
        .expect("seed member");

        let outcome = execute_erasure(
            &pool,
            ProfileId::from(operator),
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("completes");
        let ErasureOutcome::Completed(completion) = outcome else {
            panic!("must complete");
        };
        assert!(
            completion
                .targets
                .iter()
                .any(|t| t.target == "kb_contexts.shape_materialized_event_id"
                    && t.outcome == "recompute-marked"),
            "the mark is recorded in the payload"
        );

        for (label, ctx) in [
            ("the stranger's", strangers_context),
            ("the subject's", world.context),
        ] {
            let watermark: Option<Uuid> = sqlx::query_scalar(
                "SELECT shape_materialized_event_id FROM kb_contexts WHERE id = $1",
            )
            .bind(ctx)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert!(
                watermark.is_none(),
                "{label} anchor carrying the erased member is marked for recompute"
            );
        }
    }

    /// ── WITNESS: the profile tombstone ──────────────────────────────────────────────────
    /// FAILS IF the pseudonym break is incomplete: identifiers gone with the UUID kept, the
    /// tombstone timestamped by the EVENT (replay-stable), the personal-team denormalization
    /// scrubbed to the sentinel derivation, and the two no-FK Slack stores emptied — while
    /// the FK-reachable auth-link rows survive (ceiling: pseudonym).
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn the_profile_is_tombstoned_and_the_denormalization_and_external_ids_scrubbed(
        pool: sqlx::PgPool,
    ) {
        let (subject, _) = insert_profile(&pool).await;
        let (operator, _) = insert_profile(&pool).await;
        test_support::grant_governance(&pool, operator).await;
        let _ = seed_content(&pool, subject).await;
        seed_slack(&pool, subject, "slack:T123:Uerasure").await;

        let outcome = execute_erasure(
            &pool,
            ProfileId::from(operator),
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("completes");
        let ErasureOutcome::Completed(completion) = outcome else {
            panic!("must complete, got {outcome:?}");
        };

        let (handle, display, email, prefs, tombstoned_at, id): (
            String,
            String,
            Option<String>,
            serde_json::Value,
            Option<chrono::DateTime<chrono::Utc>>,
            Uuid,
        ) = sqlx::query_as(
            "SELECT handle, display_name, email, preferences, tombstoned_at, id \
               FROM kb_profiles WHERE id = $1",
        )
        .bind(subject)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(id, subject, "THE UUID STAYS — it is the pseudonym");
        assert_eq!(handle, format!("erased-{subject}"), "the handle sentinel");
        assert_eq!(display, format!("erased-{subject}"), "the display sentinel");
        assert_eq!(email, None, "the email is gone");
        assert_eq!(prefs, serde_json::json!({}), "preferences are empty");
        assert!(tombstoned_at.is_some(), "tombstoned_at is set");

        // The tombstone is the EVENT's clock, never now() — the replay-stable rule.
        let (occurred,): (chrono::DateTime<chrono::Utc>,) =
            sqlx::query_as("SELECT occurred_at FROM kb_events WHERE id = $1")
                .bind(completion.event_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            tombstoned_at,
            Some(occurred),
            "tombstoned_at == occurred_at"
        );

        // The sync_personal_team denormalization, scrubbed to the sentinel derivation.
        let (name,): (String,) = sqlx::query_as("SELECT name FROM kb_teams WHERE slug = $1")
            .bind(format!("personal-erased-{subject}"))
            .fetch_one(&pool)
            .await
            .expect("the personal team carries the sentinel slug");
        assert_eq!(
            name,
            format!("erased-{subject} (personal)"),
            "the team name is the sentinel derivation"
        );

        // The no-FK external identifiers are DELETED; the auth-link rows stay (FK-reachable,
        // pseudonym ceiling — the break covers them).
        assert_eq!(
            count(
                &pool,
                "SELECT count(*) FROM kb_slack_grant_vault WHERE slack_principal_id = 'slack:T123:Uerasure'",
            )
            .await,
            0,
            "the vault row is deleted"
        );
        assert_eq!(
            count(
                &pool,
                "SELECT count(*) FROM kb_slack_link_intents WHERE slack_principal_id = 'slack:T123:Uerasure'",
            )
            .await,
            0,
            "the intent row is deleted"
        );
        assert_eq!(
            count_with(
                &pool,
                "SELECT count(*) FROM kb_profile_auth_links WHERE profile_id = $1",
                subject,
            )
            .await,
            1,
            "the auth-link row survives — the pseudonym break covers it"
        );
        let _ = completion;
    }

    /// ── WITNESS: idempotence (spec §5) ──────────────────────────────────────────────────
    /// FAILS IF a re-erase refuses, double-strikes, or re-admits: it records a NO-OP
    /// COMPLETION whose targets all report already-erased, no new strike event fires, and
    /// kb_erased_content's first-admit attribution is untouched.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn re_erasing_a_tombstoned_subject_records_a_no_op_completion(pool: sqlx::PgPool) {
        let (subject, _) = insert_profile(&pool).await;
        let (operator, _) = insert_profile(&pool).await;
        test_support::grant_governance(&pool, operator).await;
        let world = seed_content(&pool, subject).await;
        let (blob, _, _) = seed_blob(&pool, subject, "kb_contexts", world.context, "aa").await;

        let first = execute_erasure(
            &pool,
            ProfileId::from(operator),
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("first act completes");
        let ErasureOutcome::Completed(first) = first else {
            panic!("first act must complete");
        };

        let second = execute_erasure(
            &pool,
            ProfileId::from(operator),
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("re-erase completes");
        let ErasureOutcome::Completed(second) = second else {
            panic!("a re-erase is a completion, never a refusal (spec §5)");
        };
        assert!(second.already_erased, "the no-op completion says so");
        assert_ne!(first.event_id, second.event_id, "a new event is recorded");
        assert!(
            !second.targets.is_empty()
                && second.targets.iter().all(|t| t.outcome == "already-erased"),
            "targets ALL report already-erased, got {:?}",
            second.targets
        );

        // No second strike fired: the wrapper refuses already-struck rows and the act never
        // asks it to.
        assert_eq!(
            count(
                &pool,
                "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
                 WHERE t.name = 'blob_erased'",
            )
            .await,
            1,
            "exactly one strike event for the one blob"
        );
        let _ = blob;

        // The erased-content set's attribution is the FIRST admitting event (rebuildable).
        let (admits, distinct_events): (i64, i64) = sqlx::query_as(
            "SELECT count(*), count(DISTINCT erased_by_event_id) FROM kb_erased_content",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(admits, 3, "two text hashes + one blob hash");
        assert_eq!(distinct_events, 1, "every admit cites the FIRST event");
        assert_eq!(
            count_with(
                &pool,
                "SELECT count(*) FROM kb_erased_content WHERE erased_by_event_id = $1",
                first.event_id,
            )
            .await,
            3,
            "the first completion is the attributed one"
        );
    }

    /// ── WITNESS: the refusal faces ──────────────────────────────────────────────────────
    /// FAILS IF any reason code fails to record, or a refusal mutates anything beyond its one
    /// event: each D6 reason records with its detail and leaves every site untouched.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn each_refusal_reason_records_and_mutates_nothing_else(pool: sqlx::PgPool) {
        let (subject, _) = insert_profile(&pool).await;
        let (operator, _) = insert_profile(&pool).await;
        let world = seed_content(&pool, subject).await;

        let cases = [
            (ErasureRefusalReason::Unauthorized, None),
            (
                ErasureRefusalReason::UnhonourableScope,
                Some("the subject's content in team homes".to_string()),
            ),
            (
                ErasureRefusalReason::IndependentObligation,
                Some("legal hold case 42".to_string()),
            ),
        ];
        for (reason, detail) in cases {
            let refusal = refuse_erasure(
                &pool,
                ProfileId::from(subject),
                ProfileId::from(operator),
                Uuid::now_v7(),
                reason,
                detail.clone(),
                Surface::ApiHttp,
            )
            .await
            .expect("the refusal records");
            let (stored_reason, stored_detail, subject_id): (String, Option<String>, Uuid) =
                sqlx::query_as(
                    "SELECT payload->>'reason', payload->>'detail', (payload->>'subject_id')::uuid \
                       FROM kb_events WHERE id = $1",
                )
                .bind(refusal.event_id)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(
                stored_reason,
                serde_json::to_value(reason)
                    .unwrap()
                    .as_str()
                    .expect("a refusal reason serializes to a string")
            );
            assert_eq!(stored_detail, detail, "the reason's evidence rides along");
            assert_eq!(subject_id, subject, "both types spell the subject");
        }
        assert_eq!(
            count(
                &pool,
                "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
                 WHERE t.name = 'principal_erasure_refused'",
            )
            .await,
            3,
            "one event per reason code"
        );

        // Nothing else mutated: the world is exactly as seeded.
        assert_eq!(
            count(
                &pool,
                "SELECT count(*) FROM kb_chunk_content WHERE content <> ''"
            )
            .await,
            1
        );
        let (tomb,): (Option<i32>,) = sqlx::query_as(
            "SELECT (tombstoned_at IS NOT NULL)::int FROM kb_profiles WHERE id = $1",
        )
        .bind(subject)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(tomb, Some(0), "a refusal never tombstones");
        let _ = world;
    }

    /// ── WITNESS: the payload shape (D2 by construction) ─────────────────────────────────
    /// FAILS IF the completion payload grows any key the trail functions could join on: the
    /// top-level key set is exactly the PrincipalErased shape, `redacted_hashes` is the ONLY
    /// content key-set, and the per-target outcomes carry no id-shaped keys.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn the_payload_carries_only_the_d2_shape(pool: sqlx::PgPool) {
        let (subject, handle) = insert_profile(&pool).await;
        let (operator, _) = insert_profile(&pool).await;
        test_support::grant_governance(&pool, operator).await;
        let world = seed_content(&pool, subject).await;
        let team_context = seed_team_context(&pool, &handle).await;
        let _ = seed_blob(&pool, subject, "kb_contexts", world.context, "aa").await;
        let _ = seed_blob(&pool, subject, "kb_contexts", team_context, "bb").await;

        let outcome = execute_erasure(
            &pool,
            ProfileId::from(operator),
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("completes");
        let ErasureOutcome::Completed(completion) = outcome else {
            panic!("must complete");
        };

        let (payload,): (serde_json::Value,) =
            sqlx::query_as("SELECT payload FROM kb_events WHERE id = $1")
                .bind(completion.event_id)
                .fetch_one(&pool)
                .await
                .unwrap();

        let obj = payload.as_object().expect("the payload is an object");
        for key in obj.keys() {
            assert!(
                matches!(
                    key.as_str(),
                    "subject_table" | "subject_id" | "actor" | "redacted_hashes" | "targets"
                        | "propagated_to_clients"
                ),
                "unexpected payload key {key:?} — the payload must never carry a trail join-key shape"
            );
        }
        assert_eq!(obj["subject_table"], "kb_profiles");
        assert_eq!(obj["subject_id"], serde_json::json!(subject.to_string()));
        assert_eq!(obj["actor"], serde_json::json!(operator.to_string()));
        assert_eq!(
            obj["propagated_to_clients"], false,
            "gone from the SERVER only (D3)"
        );

        // redacted_hashes is the only content key-set: hashes, and nothing id-shaped anywhere.
        let hashes = obj["redacted_hashes"].as_array().unwrap();
        assert_eq!(hashes.len(), 3, "two text hashes + the struck blob hash");
        for h in hashes {
            let s = h.as_str().unwrap();
            assert_eq!(
                s.len(),
                64,
                "bare sha256 hex, exactly as content_hash carries it"
            );
            assert!(!s.contains('-'), "a uuid would be an id-shaped key");
        }
        let payload_str = payload.to_string();
        for forbidden in [
            "resource_id",
            "block_id",
            "owner_profile_id",
            "edge_id",
            "blob_id",
        ] {
            assert!(
                !payload_str.contains(forbidden),
                "the payload must never carry {forbidden}"
            );
        }
        for t in obj["targets"].as_array().unwrap() {
            let keys: Vec<&str> = t.as_object().unwrap().keys().map(|s| s.as_str()).collect();
            assert_eq!(
                keys.len(),
                2,
                "a target names itself and its outcome, got {keys:?}"
            );
        }
    }
}
