//! The write floor (resource erasure spec D13, F4): the one check a write on a resource passes,
//! run INSIDE the write's own transaction so the check and the mutation cannot be separated.
//!
//! Each entry point, in order (after an unlocked admission that refuses without locking — see "A
//! refused caller takes no lock" below):
//!
//! 1. takes `FOR KEY SHARE` on the `kb_resources` row — the lock `_resource_write_guard` takes
//!    (migration `20260929040730`, re-commented in `20260930000070`). It conflicts with the erasure
//!    act's `FOR UPDATE`, so a write that races the act either lands before it or refuses after it.
//!    It does not conflict with the `FOR NO KEY UPDATE` a write's own projector takes on the same
//!    row, which `FOR SHARE` would (two writers each holding SHARE would deadlock on the upgrade);
//! 2. evaluates its admission — [`modify_floor_in_tx`]: `can_modify_resource` (migration
//!    `20260804000020`), called, never restated; [`liveness_floor_in_tx`]: `kb_resources.is_active`
//!    alone, for reassign, whose authority (owner or admin reach) stays its own;
//! 3. on deny only, classifies through `resource_husk_held_by` (migration `20260930000060`) — the
//!    probe the read side's `substrate_read::erased_or` asks: [`TemperError::ResourceErased`]
//!    (`410 RESOURCE_ERASED`) to a caller who holds the husk, else [`TemperError::Forbidden`]
//!    (`403`). An admitted write pays the locked SELECT and the admission, nothing more.
//!
//! **Why the lock and the admission are two statements.** Under READ COMMITTED (the transaction
//! default here) each statement takes a fresh snapshot. A lock that waited behind the act's
//! `FOR UPDATE` returns only once the act committed, so the admission that follows sees
//! `is_active = false` and the classification sees `erased_at`. A predicate folded into the
//! locking statement would evaluate its subqueries against the snapshot taken BEFORE the wait.
//! (Under REPEATABLE READ a lock that waits on a concurrent update raises a serialization failure
//! instead, which surfaces as an error, never as an admission.)
//!
//! **A missing row is not its own answer.** The lock on an unknown id locks nothing;
//! `can_modify_resource` is false, `resource_husk_held_by` is false, and the caller gets the same
//! `Forbidden` a live resource it may not modify gets.
//!
//! **A tombstone is never an erasure.** A soft-deleted resource (`is_active = false`, `erased_at`
//! NULL) is denied by both floors and classified `Forbidden`: `resource_husk_held_by` reads
//! `erased_at`, never `is_active`.
//!
//! **A check that is not a write floor takes the same lock.** A write that names a resource it does
//! not modify (an edge's target, a blob relation's peer, a grant's subject) still must not land on
//! a husk; `lock_resource_key_share` takes the lock alone, ahead of that write's own check.
//!
//! The connection is the caller's transaction (`&mut tx`), the shape every `writes::*_in_tx` takes.
//! Called on a bare pool connection it still answers, but the lock is released at once and the
//! floor is a pre-check again — the gap this module exists to close.
//!
//! **A refused caller takes no lock.** A caller the door refuses must never queue on, or hold, a
//! lock the erasure act waits on. So every entry point that locks a row on a caller's behalf asks
//! its admission (or read check) unlocked first, on the same connection, and locks only a caller
//! that passes; the check under the lock then decides. An unlocked refusal is the answer the write
//! would have had ordered before a concurrent act — never a wrong one. The one exception is a goal
//! patch's CURRENT goal rows (`DbBackend::lock_goal_rows`): rows the caller's own resource already
//! links to, locked without a read check because the update folds their edges. How long the act
//! waits on the locks admitted callers hold is a lock-timeout question, not an admission one.
//!
//! **An admission without the lock** — [`modify_admission_unlocked`] — is the same admission and
//! the same classification on the pool, for a door that must not let a refused caller take a row
//! lock at all (the delete door's `FOR UPDATE`). It binds nothing; the door still floors inside its
//! transaction.
//!
//! **A transaction that lost a race is the incumbent `500`.** A deadlock (`40P01`) or
//! serialization failure (`40001`) inside a floored write answers as every database fault does:
//! `500 INTERNAL_ERROR`, logged at error level. A `409` would collide with "already exists" in the
//! shipped clients (ruled 2026-10-01). Every shipped client classifies a 5xx as transient, but
//! none of them (Rust, TypeScript, Python, Ruby) auto-retries an unkeyed write, so the caller sees
//! it. Two updates of one resource no longer deadlock: they serialize on its row (the head of
//! `update_resource_in_tx`).
//!
//! **A write that waited past the lock bound is `503 RESOURCE_BUSY`** (ruled 2026-10-09): the floor
//! sets [`WRITE_LOCK_TIMEOUT_MS`] for the rest of the transaction (`bound_lock_waits`), and a
//! `55P03` from any statement after it answers [`TemperError::ResourceBusy`] with `Retry-After`. It
//! rolled back having applied nothing, so unlike a `500` it is always safe to send again, and
//! `temper-client` does (`ClientError::ResourceBusy`).
//!
//! **That promise is per request, so a door keeps it only if no transaction of the request has
//! committed a write before the one that hit the bound.** `db_backend`'s `api_err` classifies a
//! `55P03` wherever it surfaces, so a door that commits a write and then opens a floored
//! transaction would answer `RESOURCE_BUSY` over a committed write, and `temper-client` would
//! re-send it. The create door did exactly that (its goal edge, and segmented begin's ingestion
//! record, ran in a second transaction) until both moved into the create's own. A door that must
//! commit before a later bounded step maps that step's [`TemperError::ResourceBusy`] to the `500`
//! class. Audited 2026-10-09: no door or surface does; the pool writes ahead of a floored
//! transaction are emitter resolves (idempotent upserts), and post-commit steps log, never answer.
//!
//! **Writers queue behind an erasure act** (`queue_behind_acts`, migration `20261013100010`): a
//! shared advisory lock on R that the acts take exclusive, so writers that arrive while an act
//! waits for R cannot keep it waiting.
//!
//! **A refusal rolls back before it is answered.** `rollback_with` ends the transaction a floor
//! (or any in-transaction gate) refused, so the row lock is released before the door answers
//! rather than whenever the dropped connection's queued `ROLLBACK` reaches the server.

use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use temper_core::error::TemperError;
use temper_core::types::ids::{ProfileId, ResourceId};

use crate::backend::substrate_read::husk_held_by;

/// The modify floor: `profile` may modify `resource`, checked under the row lock in the caller's
/// transaction. `Ok(())` admits; a deny is [`TemperError::ResourceErased`] when `profile` holds
/// the erased husk, else [`TemperError::Forbidden`].
pub async fn modify_floor_in_tx(
    conn: &mut PgConnection,
    profile: ProfileId,
    resource: ResourceId,
) -> Result<(), TemperError> {
    // Unlocked first, so a caller this floor refuses never takes the row lock (the module's
    // "a refused caller takes no lock"). It binds nothing; the locked admission below decides.
    modify_admission(conn, profile, resource).await?;
    // The lock is all this step is for; whether a row came back is the admission's question.
    lock_resource_row(conn, resource).await?;
    modify_admission(conn, profile, resource).await
}

/// [`modify_floor_in_tx`]'s admission and classification WITHOUT the row lock, on one pool
/// connection: `Ok(())` admits; a deny is [`TemperError::ResourceErased`] to a holder of the
/// husk, else [`TemperError::Forbidden`] — the same answer the floor gives, from the same two
/// calls. For a door that takes a stronger lock than the floor's (the delete door's `FOR UPDATE`):
/// run this BEFORE the transaction opens, so a caller the floor would refuse never queues for, or
/// holds, that lock. It binds nothing — between this answer and the write the resource may change
/// — so the door still runs [`modify_floor_in_tx`] inside its transaction, and that call decides.
pub async fn modify_admission_unlocked(
    pool: &PgPool,
    profile: ProfileId,
    resource: ResourceId,
) -> Result<(), TemperError> {
    let mut conn = pool.acquire().await.map_err(floor_err)?;
    modify_admission(&mut conn, profile, resource).await
}

/// The modify admission: `can_modify_resource` (migration `20260804000020`), called, never
/// restated; on deny, the classification. Locks nothing — the caller decides whether a lock
/// precedes it.
async fn modify_admission(
    conn: &mut PgConnection,
    profile: ProfileId,
    resource: ResourceId,
) -> Result<(), TemperError> {
    let can: Option<bool> = sqlx::query_scalar!(
        "SELECT can_modify_resource($1, $2)",
        *profile,
        resource.uuid(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(floor_err)?;
    if can.unwrap_or(false) {
        Ok(())
    } else {
        Err(erased_or_forbidden(conn, profile, resource).await)
    }
}

/// The liveness floor, for reassign: `resource` is live (`kb_resources.is_active`), checked under
/// the row lock in the caller's transaction. No authority check — the caller keeps its own. A deny
/// is classified exactly as [`modify_floor_in_tx`]'s is.
pub async fn liveness_floor_in_tx(
    conn: &mut PgConnection,
    profile: ProfileId,
    resource: ResourceId,
) -> Result<(), TemperError> {
    liveness_floor(conn, profile, resource, ActQueue::Join).await
}

/// [`liveness_floor_in_tx`] for a run that floors MANY resources in one transaction (bulk team
/// reassignment): the same lock bound, row lock and classification, without joining each
/// resource's act queue. The queue is an advisory lock, which lives in Postgres's shared lock
/// table (`max_locks_per_transaction` × connections); thousands in one transaction can exhaust it
/// ("out of shared memory") for every session. Without the queue such a run can pass an erasure
/// act waiting on one of its resources, but it never waits on the queue, so it cannot deadlock
/// with one. Ruled 2026-10-09: bulk reassign skips the queue.
pub async fn liveness_floor_bulk_in_tx(
    conn: &mut PgConnection,
    profile: ProfileId,
    resource: ResourceId,
) -> Result<(), TemperError> {
    liveness_floor(conn, profile, resource, ActQueue::Skip).await
}

/// Whether a floor joins the resource's act queue before its row lock ([`queue_behind_acts`]).
#[derive(Clone, Copy)]
enum ActQueue {
    Join,
    Skip,
}

async fn liveness_floor(
    conn: &mut PgConnection,
    profile: ProfileId,
    resource: ResourceId,
    queue: ActQueue,
) -> Result<(), TemperError> {
    match lock_resource_row_with(conn, resource, queue).await? {
        Some(true) => Ok(()),
        Some(false) | None => Err(erased_or_forbidden(conn, profile, resource).await),
    }
}

/// The lock alone, for a write whose own check on the resource is not a write floor: an edge's
/// target read clause, a blob relation's resource peer, a grant door's subject. `FOR KEY SHARE` on
/// the `kb_resources` row in the caller's transaction, held to its end, so the erasure act (which
/// takes `FOR UPDATE` on the row) cannot commit between that check and the write. The caller runs
/// its check as the NEXT statement on the same connection: a lock that waited behind the act
/// returns only after the act committed, so that statement's fresh snapshot sees the husk (this
/// module's two-statement argument). An unknown id locks nothing and is not an error; the
/// caller's check answers it.
pub(crate) async fn lock_resource_key_share(
    conn: &mut PgConnection,
    resource: ResourceId,
) -> Result<(), TemperError> {
    lock_resource_row(conn, resource).await.map(|_| ())
}

/// `FOR KEY SHARE` on the `kb_resources` row, returning its `is_active` — `None` when no row has
/// that id. One statement, so a wait on the act's `FOR UPDATE` returns the row version the act
/// committed (the READ COMMITTED re-check), the same reasoning `_resource_write_guard` states.
async fn lock_resource_row(
    conn: &mut PgConnection,
    resource: ResourceId,
) -> Result<Option<bool>, TemperError> {
    lock_resource_row_with(conn, resource, ActQueue::Join).await
}

async fn lock_resource_row_with(
    conn: &mut PgConnection,
    resource: ResourceId,
    queue: ActQueue,
) -> Result<Option<bool>, TemperError> {
    bound_lock_waits(conn).await?;
    if let ActQueue::Join = queue {
        queue_behind_acts(conn, resource).await?;
    }
    sqlx::query_scalar!(
        "SELECT is_active FROM kb_resources WHERE id = $1 FOR KEY SHARE",
        resource.uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(floor_err)
}

/// The deny's classification: `ResourceErased` when `profile` holds the husk, else `Forbidden`.
/// `substrate_read::erased_or`'s shape, with `Forbidden` as the fallback; a fault stays a fault.
async fn erased_or_forbidden(
    conn: &mut PgConnection,
    profile: ProfileId,
    resource: ResourceId,
) -> TemperError {
    match husk_held_by(&mut *conn, profile, resource).await {
        Ok(true) => TemperError::ResourceErased(resource),
        Ok(false) => TemperError::Forbidden,
        Err(e) => floor_err(e),
    }
}

/// The write-side lock bound: how long any one statement of a floored write may wait on a lock
/// before Postgres cancels it (`55P03`) and the write answers `503 RESOURCE_BUSY`, having applied
/// nothing. Chosen from measurement (task 01a0fd12-f4b7-7bd2-81d0-13c0814650d5, harness
/// `temper-substrate/tests/write_lock_measure.rs`).
pub const WRITE_LOCK_TIMEOUT_MS: u64 = 5_000;

/// Set [`WRITE_LOCK_TIMEOUT_MS`] for the rest of `conn`'s transaction. `SET LOCAL`, so it ends with
/// the transaction and never reaches another request on the pooled connection. Every floor takes
/// its row lock through [`lock_resource_row`], which calls this first, so the bound covers every
/// floored write's waits from its floor to its commit: behind the erasure act, behind a block
/// history scrub, behind another update of the same resource.
///
/// **Per transaction, never per pool or per role.** The erasure act and the scrub run on the same
/// pool and take no floor, so they stay unbounded: an act that timed out under write load would
/// roll back every time and never complete, a denial of erasure. Their functions also pin
/// `lock_timeout = 0` themselves (migration `20261013100000`), so a default set later on the pool
/// or the role cannot reach them.
///
/// Outside a transaction `SET LOCAL` does nothing (Postgres warns), so a floor run on a bare
/// connection is unbounded; `DbBackend`'s fast-fail opens a
/// transaction for this reason.
pub(crate) async fn bound_lock_waits(conn: &mut PgConnection) -> Result<(), TemperError> {
    // `set_config(.., is_local => true)` is `SET LOCAL`, with the value bound.
    sqlx::query_scalar!(
        "SELECT set_config('lock_timeout', $1, true)",
        format!("{WRITE_LOCK_TIMEOUT_MS}ms"),
    )
    .fetch_one(&mut *conn)
    .await
    .map(|_| ())
    .map_err(floor_err)
}

/// Join R's act queue, shared, for the rest of `conn`'s transaction: the advisory lock the
/// erasure act and the block history scrub take exclusive just before their `FOR UPDATE` on R
/// (`_resource_act_queue_key`, migration `20261013100010`).
///
/// Without it a writer's `FOR KEY SHARE` is granted past an act that is waiting for the row,
/// because it conflicts with no current holder, and overlapping writers can hold an act off
/// indefinitely. With it a writer arriving after the act waits for the act (under the lock bound),
/// and the act waits only for writers already in. Shared against shared, writers never wait on
/// each other here.
///
/// **It must come before the transaction's first row lock on R.** A writer holding R's row while
/// waiting here would deadlock with an act waiting on that row. [`lock_resource_row`] calls it
/// first; a door that locks R before its floor (the delete door's `FOR UPDATE`) calls it before
/// that lock. Re-entrant: a second call in one transaction is granted at once.
pub(crate) async fn queue_behind_acts(
    conn: &mut PgConnection,
    resource: ResourceId,
) -> Result<(), TemperError> {
    sqlx::query!(
        "SELECT pg_advisory_xact_lock_shared(_resource_act_queue_key($1))",
        resource.uuid(),
    )
    .execute(&mut *conn)
    .await
    .map(|_| ())
    .map_err(floor_err)
}

/// True when `e`'s chain holds the database error a lock wait past the bound raises (`55P03`).
pub(crate) fn hit_lock_bound(e: &anyhow::Error) -> bool {
    e.chain().any(|cause| {
        cause
            .downcast_ref::<sqlx::Error>()
            .and_then(|s| s.as_database_error())
            .and_then(|d| d.code())
            .as_deref()
            == Some(crate::error::LOCK_NOT_AVAILABLE)
    })
}

/// Bridge a database error into `TemperError`: [`TemperError::ResourceBusy`] when it is a lock
/// wait past the bound, else the `500` `db_backend`'s `api_err` gives.
fn floor_err(e: sqlx::Error) -> TemperError {
    let e = anyhow::Error::from(e);
    if hit_lock_bound(&e) {
        TemperError::ResourceBusy
    } else {
        TemperError::Api(e.to_string())
    }
}

/// End `tx` — refused by a floor or another in-transaction gate — with an explicit `ROLLBACK`,
/// then hand back the refusal. Dropping the transaction would roll it back too, but only when the
/// returned connection's queued `ROLLBACK` next reaches the server; until then the floor's
/// `FOR KEY SHARE` (and any lock taken before it) stays held, so a refused caller's transaction
/// could still delay the erasure act or another writer. A failed rollback does not change the
/// answer — the refusal is still the door's — and the drop is the fallback, so it is logged only.
pub(crate) async fn rollback_with<E>(tx: Transaction<'_, Postgres>, refusal: E) -> E {
    if let Err(e) = tx.rollback().await {
        tracing::warn!(
            error = %e,
            "explicit rollback of a refused write failed; the drop rolls it back"
        );
    }
    refusal
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    //! Witnesses for both floors. Every state is made by a real path: the resource by
    //! `writes::create_resource_with`, the tombstone by `SeedAction::ResourceDelete`, the husk by
    //! `execute_resource_erasure` under a minted `SystemAdmin` proof (the pattern in
    //! `resource_erasure_service`'s tests). Only the read grant is a fixture row, as in
    //! `temper-substrate/tests/resource_husk_held_by.rs`.
    use sqlx::PgPool;
    use uuid::Uuid;

    use temper_core::types::ids::EntityId;
    use temper_substrate::events::{fire, EventContext, SeedAction};
    use temper_substrate::ids::ContextId;
    use temper_substrate::payloads::AnchorRef;
    use temper_substrate::writes::{self, CreateParams};
    use temper_workflow::operations::Surface;

    use super::*;
    use crate::auth::SystemAdmin;
    use crate::services::resource_erasure_service::{
        execute_resource_erasure, ResourceErasureOutcome, ResourceErasureRequest,
    };
    use crate::test_support;

    /// A principal: profile, its `<handle>@web` emitter entity, and a personal context.
    struct Principal {
        profile: ProfileId,
        emitter: EntityId,
        home: ContextId,
    }

    /// The handle is the FULL id: two UUIDv7s minted in one millisecond share leading bytes, so a
    /// truncated handle collides on `kb_profiles_handle_key`.
    async fn principal(pool: &PgPool) -> Principal {
        let id = Uuid::now_v7();
        let handle = format!("user-{id}");
        sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)")
            .bind(id)
            .bind(&handle)
            .execute(pool)
            .await
            .expect("seed profile");
        let emitter: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2) RETURNING id",
        )
        .bind(id)
        .bind(format!("{handle}@web"))
        .fetch_one(pool)
        .await
        .expect("seed emitter entity");
        let home: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
             VALUES ('kb_profiles', $1, 'home', 'Home') RETURNING id",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("seed personal context");
        Principal {
            profile: ProfileId::from(id),
            emitter: EntityId::from(emitter),
            home: ContextId::from(home),
        }
    }

    /// An operator's sealed proof, minted through the real gate after `grant_governance`.
    async fn operator(pool: &PgPool) -> SystemAdmin {
        let op = principal(pool).await;
        test_support::grant_governance(pool, op.profile.uuid()).await;
        test_support::system_admin_proof_for(pool, op.profile.uuid()).await
    }

    /// A live resource created through the REAL create path, homed in `owner`'s context.
    async fn resource(pool: &PgPool, owner: &Principal) -> ResourceId {
        let origin = format!("test://write-floor-{}", Uuid::now_v7());
        writes::create_resource_with(
            pool,
            CreateParams {
                idempotency_key: None,
                title: "write floor subject",
                origin_uri: &origin,
                body: "body under the write floor",
                doc_type: "research",
                home: AnchorRef::context(owner.home),
                owner: owner.profile,
                originator: owner.profile,
                emitter: owner.emitter,
                properties: &[],
                chunks: None,
                sources: vec![],
            },
            EventContext::default(),
        )
        .await
        .expect("create resource through the real path")
    }

    /// Soft-delete `resource` through the real path: a tombstone, never a husk.
    async fn tombstone(pool: &PgPool, owner: &Principal, resource: ResourceId) {
        let mut tx = pool.begin().await.expect("begin");
        fire(
            &mut tx,
            SeedAction::ResourceDelete {
                resource,
                emitter: owner.emitter,
            },
        )
        .await
        .expect("soft delete through the real path");
        tx.commit().await.expect("commit");
        let (is_active, erased): (bool, bool) = sqlx::query_as(
            "SELECT is_active, erased_at IS NOT NULL FROM kb_resources WHERE id = $1",
        )
        .bind(resource.uuid())
        .fetch_one(pool)
        .await
        .expect("row");
        assert!(
            !is_active && !erased,
            "precondition: a tombstone, not a husk"
        );
    }

    /// Erase `resource` through the real act; asserts it completed and left a husk.
    async fn erase(pool: &PgPool, resource: ResourceId) {
        let op = operator(pool).await;
        let outcome = execute_resource_erasure(
            pool,
            None,
            &op,
            ResourceErasureRequest {
                resource,
                also_strike_blobs: &[],
                surface: Surface::ApiHttp,
            },
        )
        .await
        .expect("the act answers");
        assert!(
            matches!(outcome, ResourceErasureOutcome::Completed(_)),
            "the act must complete, got {outcome:?}"
        );
        let erased: bool =
            sqlx::query_scalar("SELECT erased_at IS NOT NULL FROM kb_resources WHERE id = $1")
                .bind(resource.uuid())
                .fetch_one(pool)
                .await
                .expect("row");
        assert!(erased, "precondition: the act made a husk");
    }

    /// A direct profile `can_read` grant, and nothing more.
    async fn grant_read(pool: &PgPool, resource: ResourceId, grantee: ProfileId, by: ProfileId) {
        sqlx::query(
            "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
             VALUES ('kb_resources', $1, 'kb_profiles', $2, true, $3)",
        )
        .bind(resource.uuid())
        .bind(grantee.uuid())
        .bind(by.uuid())
        .execute(pool)
        .await
        .expect("insert read grant");
    }

    /// The modify floor in a fresh transaction, rolled back after.
    async fn modify(
        pool: &PgPool,
        profile: ProfileId,
        resource: ResourceId,
    ) -> Result<(), TemperError> {
        let mut tx = pool.begin().await.expect("begin");
        let answer = modify_floor_in_tx(&mut tx, profile, resource).await;
        tx.rollback().await.expect("rollback");
        answer
    }

    /// The liveness floor in a fresh transaction, rolled back after.
    async fn liveness(
        pool: &PgPool,
        profile: ProfileId,
        resource: ResourceId,
    ) -> Result<(), TemperError> {
        let mut tx = pool.begin().await.expect("begin");
        let answer = liveness_floor_in_tx(&mut tx, profile, resource).await;
        tx.rollback().await.expect("rollback");
        answer
    }

    // ── modify_floor_in_tx ──────────────────────────────────────────────────────────────────

    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn modify_floor_admits_the_owner_of_a_live_resource(pool: PgPool) {
        let owner = principal(&pool).await;
        let r = resource(&pool, &owner).await;
        let answer = modify(&pool, owner.profile, r).await;
        assert!(answer.is_ok(), "the owner may modify: {answer:?}");
    }

    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn modify_floor_forbids_a_non_modifier_of_a_live_resource(pool: PgPool) {
        let owner = principal(&pool).await;
        let stranger = principal(&pool).await;
        let r = resource(&pool, &owner).await;
        let answer = modify(&pool, stranger.profile, r).await;
        assert!(
            matches!(answer, Err(TemperError::Forbidden)),
            "a stranger is forbidden: {answer:?}"
        );
    }

    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn modify_floor_forbids_the_owner_of_a_tombstone_never_erased(pool: PgPool) {
        let owner = principal(&pool).await;
        let r = resource(&pool, &owner).await;
        tombstone(&pool, &owner, r).await;
        let answer = modify(&pool, owner.profile, r).await;
        assert!(
            matches!(answer, Err(TemperError::Forbidden)),
            "a tombstone is Forbidden, never ResourceErased: {answer:?}"
        );
    }

    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn modify_floor_answers_erased_to_the_owner_of_a_husk(pool: PgPool) {
        let owner = principal(&pool).await;
        let r = resource(&pool, &owner).await;
        erase(&pool, r).await;
        let answer = modify(&pool, owner.profile, r).await;
        assert!(
            matches!(answer, Err(TemperError::ResourceErased(id)) if id == r),
            "the owner of a husk is told it was erased: {answer:?}"
        );
    }

    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn modify_floor_answers_erased_to_a_read_grant_holder_of_a_husk(pool: PgPool) {
        let owner = principal(&pool).await;
        let reader = principal(&pool).await;
        let r = resource(&pool, &owner).await;
        grant_read(&pool, r, reader.profile, owner.profile).await;
        assert!(
            matches!(
                modify(&pool, reader.profile, r).await,
                Err(TemperError::Forbidden)
            ),
            "precondition: a read-only grant does not admit a modify while live"
        );
        erase(&pool, r).await;
        let answer = modify(&pool, reader.profile, r).await;
        assert!(
            matches!(answer, Err(TemperError::ResourceErased(id)) if id == r),
            "a read-grant holder of a husk is told it was erased (the read population): {answer:?}"
        );
    }

    /// Not one of the plan's six: the oracle guard. A husk the caller holds no standing on answers
    /// exactly as an unknown id does.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn modify_floor_forbids_a_stranger_to_a_husk(pool: PgPool) {
        let owner = principal(&pool).await;
        let stranger = principal(&pool).await;
        let r = resource(&pool, &owner).await;
        erase(&pool, r).await;
        let answer = modify(&pool, stranger.profile, r).await;
        assert!(
            matches!(answer, Err(TemperError::Forbidden)),
            "a stranger to a husk gets Forbidden, never ResourceErased: {answer:?}"
        );
    }

    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn modify_floor_forbids_an_unknown_id(pool: PgPool) {
        let caller = principal(&pool).await;
        let answer = modify(&pool, caller.profile, ResourceId::from(Uuid::now_v7())).await;
        assert!(
            matches!(answer, Err(TemperError::Forbidden)),
            "an unknown id is Forbidden, not a distinct answer: {answer:?}"
        );
    }

    // ── liveness_floor_in_tx ────────────────────────────────────────────────────────────────

    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn liveness_floor_admits_a_non_owner_of_a_live_resource(pool: PgPool) {
        let owner = principal(&pool).await;
        let stranger = principal(&pool).await;
        let r = resource(&pool, &owner).await;
        let answer = liveness(&pool, stranger.profile, r).await;
        assert!(
            answer.is_ok(),
            "liveness checks no authority, so a non-owner passes on a live resource: {answer:?}"
        );
    }

    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn liveness_floor_forbids_a_tombstone(pool: PgPool) {
        let owner = principal(&pool).await;
        let r = resource(&pool, &owner).await;
        tombstone(&pool, &owner, r).await;
        let answer = liveness(&pool, owner.profile, r).await;
        assert!(
            matches!(answer, Err(TemperError::Forbidden)),
            "a tombstone fails liveness as Forbidden, never ResourceErased: {answer:?}"
        );
    }

    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn liveness_floor_answers_erased_to_the_holder_of_a_husk(pool: PgPool) {
        let owner = principal(&pool).await;
        let r = resource(&pool, &owner).await;
        erase(&pool, r).await;
        let answer = liveness(&pool, owner.profile, r).await;
        assert!(
            matches!(answer, Err(TemperError::ResourceErased(id)) if id == r),
            "the holder of a husk is told it was erased: {answer:?}"
        );
    }
}
