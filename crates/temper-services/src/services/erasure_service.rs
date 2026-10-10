//! The erasure act's service layer — execute + survey (spec 2026-08-31, task 01a0577c Beat 2).
//!
//! THE DOOR IS OPERATOR-ONLY (ruled 2026-09-09): the instance-level operator — the
//! `is_system_admin` standing — is the ONLY executor; self-serve is a non-goal of this build.
//! Both functions take the sealed [`SystemAdmin`] proof, so the gate is the signature: a caller
//! who is not a system admin cannot reach them, and the surface that mints the proof rejects that
//! caller before dispatch with no ledger event (`handlers::erasure`, ruled 2026-09-30). The act is
//! attributed to `admin.actor()`.
//!
//! SQL commits, it does not decide legality (`principal_standing_apply`'s shape,
//! migrations/20260720000030): the scope computation, the per-row governed-home strikes, the
//! tombstone machinery and the completion event live in `principal_erasure_execute` /
//! `_erasure_apply_redaction` (migration 20260909000025; the computation itself moved whole into
//! `principal_erasure_survey_plan`, 20260913000010, which the act consumes and the survey door
//! serves). The admin surface's HTTP doors (`handlers::erasure::execute`,
//! `handlers::erasure::survey`) call straight into [`execute_erasure`] / [`survey_erasure`] —
//! this module stays the service layer and carries no HTTP types.
//!
//! The request reference is an opaque UUID supplied by the caller: it rides
//! `kb_events."references"` (rel `request`) and the act's correlation id — never the payload —
//! so the request-to-person mapping stays in the operator's DSAR records, outside the ledger.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::erasure::{
    EstateCounts, EstateErasureKind, EstateResourceErasure, EstateResourcePlan,
};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_substrate::payloads::ErasureTargetOutcome;
use temper_substrate::writes::resolve_emitter;
use temper_workflow::operations::Surface;

use crate::auth::SystemAdmin;
use crate::error::{ApiError, ApiResult};
use crate::services::resource_erasure_service::{
    is_retryable_act_error, queue_region_settling, MAX_ACT_RETRIES,
};

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
/// `already_erased`). The no-op's targets are whatever live rows the estate still holds:
/// post-act re-commits into a retired home report strikes, not `already-erased`, so an
/// empty target set means an empty estate, not merely a tombstoned subject. Either way ONE
/// `principal_erased` event stands behind these values. This is the act's only answer: no door
/// raises a principal refusal (a non-admin is rejected at the wire, and a re-erase is this
/// no-op completion).
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
    /// The resource erasures the act ran over the estate, in the order it ran them (D4).
    pub resource_erasures: Vec<EstateResourceErasure>,
    /// Live resources and live blobs still homed in the estate once the act committed (D9, ruled
    /// Q3). `None` when the count could not be read: unknown is never reported as zero.
    pub estate_stragglers: Option<u32>,
}

/// The jsonb `principal_erasure_execute` returns, before mapping onto the typed outcome.
#[derive(Debug, serde::Deserialize)]
struct ExecuteOutcomeWire {
    event_id: Uuid,
    redacted_hashes: Vec<String>,
    targets: Vec<ErasureTargetOutcome>,
    already_erased: bool,
    estate_contexts: Vec<Uuid>,
    resource_erasures: Vec<ResourceErasureWire>,
}

/// One entry of the act's `resource_erasures`, as the ledger spells it: `resource` and `event`,
/// never `resource_id`, because the completion payload carries no trail join key.
#[derive(Debug, serde::Deserialize)]
struct ResourceErasureWire {
    resource: Uuid,
    event: Uuid,
    kind: EstateErasureKind,
}

/// One blob strike's PREDICTED verdict from the survey — the plan's `released_would_be`:
/// the `blob_delete` refcount's own predicate, run at survey time. THE STRIKE-TIME VERDICT
/// IS AUTHORITATIVE (the `strike_verdicts` honesty rule): this speaks for the moment the
/// survey ran, and a commit or a sibling strike in between can flip the act's actual
/// verdict; neither overrules the other.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct BlobStrikeVerdict {
    pub blob_id: Uuid,
    pub released: bool,
}

/// What the read-only survey predicts the act would do (task 01a09628 item 2): the full
/// record the act would write — targets as prose, exactly, in the act's own order — the
/// redacted set, the tombstone state it would report, and the typed strike predictions.
/// Writes nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct ErasureSurvey {
    pub subject: ProfileId,
    pub already_erased: bool,
    /// The redacted set (D2) the act would admit: the governed-scope text hashes plus every
    /// hash it would actually strike.
    pub redacted_hashes: Vec<String>,
    pub targets: Vec<ErasureTargetOutcome>,
    pub blob_strikes: Vec<BlobStrikeVerdict>,
    /// The estate's size by disposition (D6): the survey's first answer.
    pub estate: EstateCounts,
    /// Every estate resource, in the order the act would take it, with counts from its own survey.
    pub resources: Vec<EstateResourcePlan>,
}

/// The jsonb `principal_erasure_survey` returns, before mapping onto the typed survey.
#[derive(Debug, serde::Deserialize)]
struct SurveyOutcomeWire {
    redacted_hashes: Vec<String>,
    targets: Vec<ErasureTargetOutcome>,
    already_erased: bool,
    blob_strikes: Vec<BlobStrikeVerdict>,
    estate: EstateCounts,
    resources: Vec<EstateResourcePlan>,
}

/// Execute the erasure act for `subject`, as the operator `admin` names.
///
/// The [`SystemAdmin`] proof is the authority: the gate ran where the proof was minted, before
/// anything here, so the existence of the subject is disclosed only to an operator.
///
/// The request reference tolerates retries: a retried call with the SAME reference re-executes
/// as a no-op completion on an already-erased subject (`already_erased`, spec §5). Correlation
/// is INDEXED, never unique (20260624000001_canonical_schema.sql:491) — the reference pairs
/// the act's events, it does not deduplicate the door.
pub async fn execute_erasure(
    pool: &PgPool,
    admin: &SystemAdmin,
    subject: ProfileId,
    request_reference: Uuid,
    surface: Surface,
) -> ApiResult<ErasureCompletion> {
    let operator = admin.actor();
    // The emitter resolves on the pool, before the transaction opens (a read, not part of the
    // mutation) — the operator's entity on the door's surface. Hard precondition, the
    // `slack_disconnect_service` shape: an unattributable authority act is worse than a failed
    // one. The surface rides from the door (`Surface::ApiHttp` from the HTTP door, the one that
    // exists today) so the ledger says where the act came from, never a hard-wired guess that
    // outlives its accuracy — the blob commit's S5 correction.
    let emitter = resolve_emitter(pool, operator, surface.marker())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Operator-only from here: existence may be disclosed (and as an error, not a ledger row).
    let exists: Option<Uuid> = sqlx::query_scalar!(
        r#"SELECT id FROM kb_profiles WHERE id = $1"#,
        subject.uuid()
    )
    .fetch_optional(pool)
    .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound("profile not found".to_string()));
    }

    // The act runs resource erasure once per estate resource (20261021100000), so it fails
    // retryably where that act does: a deadlock, a raced edge fold, a remote source that gained or
    // lost a citer, a collision on the redaction rows (D8). The act is one statement and one
    // transaction, so a failed attempt commits nothing and each retry is a fresh statement against
    // the post-conflict state.
    let mut retries = 0;
    let raw = loop {
        let result = sqlx::query_scalar!(
            r#"SELECT principal_erasure_execute($1, $2, $3, $4) AS "outcome: serde_json::Value""#,
            subject.uuid(),
            operator.uuid(),
            emitter.uuid(),
            request_reference,
        )
        .fetch_one(pool)
        .await;
        match result {
            Ok(raw) => break raw,
            Err(err) if is_retryable_act_error(&err) && retries < MAX_ACT_RETRIES => retries += 1,
            // Scrubbed, as the resource door does: an exhausted retry on a redaction-row
            // collision is a unique violation, which the generic mapping would render as a 409
            // "already exists" that misdescribes it.
            Err(err) => {
                return Err(ApiError::internal_scrubbed(
                    "principal erasure act failed",
                    err,
                ))
            }
        }
    }
    .ok_or_else(|| ApiError::Internal("principal_erasure_execute returned no row".to_string()))?;
    let wire: ExecuteOutcomeWire = serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("erasure outcome shape: {e}")))?;

    // COMMITTED. Nothing below may answer as a failure: the operator would re-run an act that
    // succeeded. Region settling for every erased resource's anchors, as the resource door does
    // after its own act (each erasure nulled those watermarks and recomputed the live centroids;
    // the settling re-forms the regions without the husk). Never fails the act.
    for e in &wire.resource_erasures {
        queue_region_settling(pool, ResourceId::from(e.resource), emitter).await;
    }
    let estate_stragglers = estate_stragglers(pool, &wire.estate_contexts).await;

    // The per-row verdicts live in the payload's targets; the strikes are re-matched out of
    // them so the typed outcome can carry what a fence needs without re-deriving from prose.
    // The SQL folded `blob_delete`'s verdict into each strike's outcome string; the
    // authoritative per-row record is the `blob_erased` events, which share the completion's
    // correlation id.
    let blob_strikes = strike_verdicts(pool, wire.event_id).await?;

    Ok(ErasureCompletion {
        event_id: wire.event_id,
        redacted_hashes: wire.redacted_hashes,
        targets: wire.targets,
        blob_strikes,
        already_erased: wire.already_erased,
        resource_erasures: wire
            .resource_erasures
            .into_iter()
            .map(|e| EstateResourceErasure {
                resource_id: e.resource,
                event_id: e.event,
                kind: e.kind,
            })
            .collect(),
        estate_stragglers,
    })
}

/// Live resources and live blobs still homed in the estate contexts the act recorded, charters
/// aside: one created there after the act's plan read the estate and committed before the act
/// retired the context (D9). Ruled Q3 (2026-10-10): reported, not locked against. Re-running the
/// act erases them. A failed read is logged and answered `None`, never zero: the act has
/// committed, so it must not fail, and an unknown count must not read as a clean estate.
async fn estate_stragglers(pool: &PgPool, estate: &[Uuid]) -> Option<u32> {
    let count = sqlx::query_scalar!(
        r#"SELECT (SELECT count(*)
                     FROM kb_resource_homes h
                     JOIN kb_resources r ON r.id = h.resource_id
                    WHERE h.anchor_table = 'kb_contexts'
                      AND h.anchor_id = ANY($1)
                      AND r.erased_at IS NULL
                      AND NOT EXISTS (SELECT 1 FROM kb_cogmaps m WHERE m.telos_resource_id = r.id))
                + (SELECT count(*)
                     FROM kb_blobs b
                    WHERE b.home_table = 'kb_contexts'
                      AND b.home_id = ANY($1)
                      AND b.content_type IS NOT NULL) AS "n!""#,
        estate,
    )
    .fetch_one(pool)
    .await;
    match count {
        Ok(n) => Some(u32::try_from(n).unwrap_or(u32::MAX)),
        Err(e) => {
            tracing::warn!(error = %e, "failed to count the estate's remaining live resources and blobs after erasure; reported as unknown");
            None
        }
    }
}

/// The per-row strike verdicts for a completion: every `blob_erased` event sharing the
/// completion's correlation id, joined to the row it emptied, each carrying the verdict the
/// wrapper decided AT THAT STRIKE'S MOMENT. A later read sees only the end state, so the
/// moment is reconstructed: live rows when the wrapper counted = live rows now + same-hash
/// strikes LATER in the same act (each emptied one live row) + the struck row itself, which
/// was still live under its own refcount — so released ⟺ live-now + later = 0. A subject's
/// two same-hash homes therefore read false then true, the act's sequential refcount, not a
/// flat end-state read. THE ORDER IS THE ACT'S OWN: the strike loop consumes the plan's
/// rows in `kb_blobs.id` order (the plan's declared, load-bearing order), so "later" is a
/// key comparison on the struck row's id — exact regardless of event-timestamp or
/// uuid-generation ordering, which tie at transaction-stable timestamps. The fence derives
/// its own pathname from the payload's per-target outcome prose — the STRUCK row's
/// `blob_pathname` is always NULL (the strike emptied it), so it is not read here.
async fn strike_verdicts(
    pool: &PgPool,
    completion_event: Uuid,
) -> ApiResult<Vec<BlobStrikeOutcome>> {
    let rows = sqlx::query!(
        r#"
        SELECT (e.payload->>'blob_id')::uuid       AS "blob_id: Uuid",
               ((SELECT count(*) FROM kb_blobs live
                  WHERE live.content_hash = b.content_hash
                    AND live.content_type IS NOT NULL)
                + (SELECT count(*) FROM kb_events f
                    JOIN kb_event_types ft ON ft.id = f.event_type_id
                          AND ft.name = 'blob_erased'
                   WHERE f.correlation_id = e.correlation_id
                     AND f.id <> e.id
                     AND (f.payload->>'blob_id')::uuid IN (
                         SELECT s.id FROM kb_blobs s
                          WHERE s.content_hash = b.content_hash
                            AND s.id > b.id))) = 0 AS "released: bool"
          FROM kb_events e
          JOIN kb_event_types t ON t.id = e.event_type_id AND t.name = 'blob_erased'
          JOIN kb_blobs b ON b.id = (e.payload->>'blob_id')::uuid
         WHERE e.correlation_id = (SELECT correlation_id FROM kb_events WHERE id = $1)
           AND e.id <> $1
         ORDER BY b.id
        "#,
        completion_event,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| BlobStrikeOutcome {
            blob_id: r.blob_id.expect("a blob_erased payload carries blob_id"),
            // The wrapper's verdict at ITS moment — the struck row was live under its own
            // refcount, so live-now + later same-hash strikes = 0 ⟺ it was the last live
            // row. A re-commit after the act (the fence's declared-open window) drifts this
            // read from the strike-time prose, and the payload's prose stays authoritative.
            released: r.released.unwrap_or(false),
        })
        .collect())
}

/// The read-only survey (task 01a09628 item 2): what [`execute_erasure`] WOULD record if it
/// ran now. The prediction is the act's OWN computation — `principal_erasure_survey_plan`
/// (migration 20260913000010), the exact body the act consumes — never a re-derivation, and
/// its strike verdicts simulate the act's own sequential refcount (same-hash rows earlier
/// in the strike set are already emptied when the act reaches this row). In the same state,
/// the prediction matches the record; the strike-time verdict stays authoritative for a
/// commit or sibling strike that lands after the survey (a preview that can disagree is
/// worse than no preview).
///
/// The [`SystemAdmin`] proof is the gate, as for the act. The survey writes nothing, so there is
/// deliberately no `Surface` parameter: it appends nothing, so there is nothing to attribute
/// (ruled 2026-09-12: a survey attempt is not an erasure request).
///
/// Existence is disclosed only to an operator (the execute door's order).
pub async fn survey_erasure(
    pool: &PgPool,
    _admin: &SystemAdmin,
    subject: ProfileId,
) -> ApiResult<ErasureSurvey> {
    // Operator-only from here: existence may be disclosed (and as an error, not a ledger row).
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
        r#"SELECT principal_erasure_survey($1) AS "survey: serde_json::Value""#,
        subject.uuid(),
    )
    .fetch_one(pool)
    .await?
    .ok_or_else(|| ApiError::Internal("principal_erasure_survey returned no row".to_string()))?;
    let wire: SurveyOutcomeWire = serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("erasure survey shape: {e}")))?;

    Ok(ErasureSurvey {
        subject,
        already_erased: wire.already_erased,
        redacted_hashes: wire.redacted_hashes,
        targets: wire.targets,
        blob_strikes: wire.blob_strikes,
        estate: wire.estate,
        resources: wire.resources,
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
            // A team the subject does NOT own personally: since 20261021100000 the personal
            // team's contexts are the estate (R1), so a not-governed team home is another team.
            "WITH t AS (INSERT INTO kb_teams (slug, name) VALUES ('shared-' || $2, 'Shared') \
                       RETURNING id) \
             INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
                     SELECT $1, 'kb_teams', t.id, 'shared', 'Shared' FROM t",
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
        let admin = test_support::system_admin_proof_for(&pool, operator).await;
        let world = seed_content(&pool, subject).await;

        let outcome = execute_erasure(
            &pool,
            &admin,
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::CliCloud,
        )
        .await
        .expect("the operator's act completes");
        let _ = outcome;

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

    /// ── WITNESS: the personal-homed blob strike ─────────────────────────────────────────
    /// FAILS IF the strike does not go through the wrapper as a per-row `blob_erased` event:
    /// the row lands in the D5.2 shape, the verdict rides the payload's per-target outcomes,
    /// and the strike is correlated to the completion (the pairing is a fact, D1).
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn a_personal_homed_blob_is_struck_and_its_verdict_rides_the_payload(pool: sqlx::PgPool) {
        let (subject, _) = insert_profile(&pool).await;
        let (operator, _) = insert_profile(&pool).await;
        test_support::grant_governance(&pool, operator).await;
        let admin = test_support::system_admin_proof_for(&pool, operator).await;
        let world = seed_content(&pool, subject).await;
        let (blob, hash, pathname) =
            seed_blob(&pool, subject, "kb_contexts", world.context, "aa").await;

        let outcome = execute_erasure(
            &pool,
            &admin,
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("the operator's act completes");
        let completion = outcome;
        assert_eq!(
            completion.redacted_hashes.len(),
            1,
            "the struck blob's hash; no non-charter text hash (D5)"
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
        let admin = test_support::system_admin_proof_for(&pool, operator).await;
        let world = seed_content(&pool, subject).await;
        let team_context = seed_team_context(&pool, &handle).await;
        let (blob, hash, pathname) =
            seed_blob(&pool, subject, "kb_contexts", team_context, "bb").await;

        let outcome = execute_erasure(
            &pool,
            &admin,
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("completes");
        let completion = outcome;

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
        let admin = test_support::system_admin_proof_for(&pool, operator).await;
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
            &admin,
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("completes");
        let completion = outcome;
        // Since 20261021100000 the subject's copy is emptied by resource erasure, row by row, and
        // no estate hash but a charter's is admitted (D5, ruled Q1): the record names none.
        assert!(
            !completion.redacted_hashes.contains(&world.chunk_hash),
            "a non-charter estate hash is not in the redacted set"
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
    /// retained, embedding AND provenance nulled together, the search vector emptied — and the
    /// formation watermark on the affected anchor nulled (the centroid recompute mark, D5).
    /// Since 20261021100000 resource erasure empties the estate row by row and admits no hash to
    /// the erased-content set (D5 of the person-erasure design, ruled Q1): the set gains none.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn text_content_is_emptied_hashes_kept_and_no_estate_hash_enters_the_set(
        pool: sqlx::PgPool,
    ) {
        let (subject, _) = insert_profile(&pool).await;
        let (operator, _) = insert_profile(&pool).await;
        test_support::grant_governance(&pool, operator).await;
        let admin = test_support::system_admin_proof_for(&pool, operator).await;
        let world = seed_content(&pool, subject).await;

        let outcome = execute_erasure(
            &pool,
            &admin,
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("completes");
        let completion = outcome;

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
                !completion.redacted_hashes.contains(h),
                "a non-charter text hash is not in the redacted set ({h})"
            );
            assert_eq!(
                count_hash(
                    &pool,
                    "SELECT count(*) FROM kb_erased_content WHERE content_hash = $1",
                    h.to_string(),
                )
                .await,
                0,
                "kb_erased_content does not hold {h}"
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
        let admin = test_support::system_admin_proof_for(&pool, operator).await;
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
            &admin,
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("completes");
        let completion = outcome;
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

    /// ── WITNESS: erasure finds the personal team by identity, not by slug ───────────────
    /// FAILS IF erasure recomputes `'personal-' || handle`: when the bare slug was held at
    /// genesis, the subject's own team carries a `-N` suffix, and a slug lookup would scrub the
    /// holder's team while leaving the subject's handle in their own team's slug and name.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn erasure_scrubs_the_subjects_suffixed_personal_team_not_the_slug_holder(
        pool: sqlx::PgPool,
    ) {
        let (holder, _) = insert_profile(&pool).await;
        let subject = Uuid::now_v7();
        let handle = format!("user-{subject}");
        let held: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_teams (slug, name) VALUES ($1, 'Held') RETURNING id",
        )
        .bind(format!("personal-{handle}"))
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')",
        )
        .bind(held)
        .bind(holder)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)")
            .bind(subject)
            .bind(&handle)
            .execute(&pool)
            .await
            .expect("a held personal slug never blocks a profile");
        sqlx::query("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2)")
            .bind(subject)
            .bind(format!("{handle}@web"))
            .execute(&pool)
            .await
            .unwrap();
        let (operator, _) = insert_profile(&pool).await;
        test_support::grant_governance(&pool, operator).await;
        let admin = test_support::system_admin_proof_for(&pool, operator).await;

        execute_erasure(
            &pool,
            &admin,
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("completes");

        let (slug, name): (String, String) =
            sqlx::query_as("SELECT slug, name FROM kb_teams WHERE id = $1")
                .bind(held)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            slug,
            format!("personal-{handle}"),
            "the holder's slug is untouched"
        );
        assert_eq!(name, "Held", "the holder's name is untouched");

        let (own_slug, own_name): (String, String) =
            sqlx::query_as("SELECT slug, name FROM kb_teams WHERE personal_of = $1")
                .bind(subject)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(own_slug, format!("personal-erased-{subject}"));
        assert_eq!(own_name, format!("erased-{subject} (personal)"));
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
        let admin = test_support::system_admin_proof_for(&pool, operator).await;
        let _ = seed_content(&pool, subject).await;
        seed_slack(&pool, subject, "slack:T123:Uerasure").await;

        let outcome = execute_erasure(
            &pool,
            &admin,
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("completes");
        let completion = outcome;

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
        let admin = test_support::system_admin_proof_for(&pool, operator).await;
        let world = seed_content(&pool, subject).await;
        let (blob, _, _) = seed_blob(&pool, subject, "kb_contexts", world.context, "aa").await;

        let first = execute_erasure(
            &pool,
            &admin,
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("first act completes");

        let second = execute_erasure(
            &pool,
            &admin,
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("re-erase completes");
        assert!(second.already_erased, "the no-op completion says so");
        assert_ne!(first.event_id, second.event_id, "a new event is recorded");
        // The estate's resources are husks the first act completed, so the re-erase runs no
        // resource erasure and names them skipped (2f); every other target is already-erased.
        assert!(
            second.resource_erasures.is_empty(),
            "no resource erasure runs again"
        );
        assert!(
            second.targets.iter().any(|t| t.target == "kb_resources"
                && t.outcome == "1 estate resource(s) already erased and complete; skipped"),
            "the complete husk is named skipped, got {:?}",
            second.targets
        );
        assert!(
            second
                .targets
                .iter()
                .filter(|t| t.target != "kb_resources")
                .all(|t| t.outcome == "already-erased")
                && second.targets.iter().any(|t| t.target != "kb_resources"),
            "every other target reports already-erased, got {:?}",
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
        assert_eq!(admits, 1, "the struck blob's hash alone (D5)");
        assert_eq!(distinct_events, 1, "every admit cites the FIRST event");
        assert_eq!(
            count_with(
                &pool,
                "SELECT count(*) FROM kb_erased_content WHERE erased_by_event_id = $1",
                first.event_id,
            )
            .await,
            1,
            "the first completion is the attributed one"
        );
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
        let admin = test_support::system_admin_proof_for(&pool, operator).await;
        let world = seed_content(&pool, subject).await;
        let team_context = seed_team_context(&pool, &handle).await;
        let _ = seed_blob(&pool, subject, "kb_contexts", world.context, "aa").await;
        let _ = seed_blob(&pool, subject, "kb_contexts", team_context, "bb").await;

        let outcome = execute_erasure(
            &pool,
            &admin,
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
        .expect("completes");
        let completion = outcome;

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
                    "subject_table"
                        | "subject_id"
                        | "actor"
                        | "redacted_hashes"
                        | "targets"
                        | "estate_contexts"
                        | "resource_erasures"
                        | "charters_held"
                ),
                "unexpected payload key {key:?} — the payload must never carry a trail join-key shape"
            );
        }
        assert_eq!(obj["subject_table"], "kb_profiles");
        assert_eq!(obj["subject_id"], serde_json::json!(subject.to_string()));
        assert_eq!(obj["actor"], serde_json::json!(operator.to_string()));

        // redacted_hashes is the only content key-set: hashes, and nothing id-shaped anywhere.
        let hashes = obj["redacted_hashes"].as_array().unwrap();
        assert_eq!(
            hashes.len(),
            1,
            "the struck blob hash; no charter in this estate (D5)"
        );
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
