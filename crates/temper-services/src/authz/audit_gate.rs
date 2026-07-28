//! Citation-audit authorization — may this caller record an audit on *this* citation, run the
//! auditor's dispatch tick, and complete the job it was handed?
//!
//! This is the gate the whole adversarial premise rests on. Set 5's auditor reads a finding's
//! citations and emits a signed defensibility verdict per source; if the gate is wrong, a citer
//! grades its own work, and "assessed by another party" is enforced nowhere.
//!
//! # The widening, and the two things that stop it going too far
//!
//! Every other authored write in temper gates on `can_modify_resource`. An audit must **not**: an
//! auditor that may only assess findings it owns is not an auditor. So the admitting predicate is
//! **readability** — a deliberate widening (a write authorized by a read predicate), spec §7.
//!
//! Readability alone is not sufficient, and spec §7 names **both** of the conjuncts that narrow it
//! back: *"can read the finding — the full canonical visibility predicate — plus being a registered,
//! unrevoked machine principal with reach"*, and a **self-audit denial arm**. Both are here:
//!
//! * The machine conjunct is **deliberately NOT enforced here.** Spec §7 ¶1 describes the auditor
//!   as a registered machine principal, and an earlier pass read that as a third denial arm. It was
//!   removed by an explicit product decision: a **human** audit is not a degraded machine audit but
//!   a distinct and arguably stronger signal, and human-expressible audits are wanted as a
//!   **promotion mechanic**. §7's registration sentence describes who the *auditor persona* is; it
//!   is not a restriction on who may assess a citation.
//!
//!   **OPEN RISK, recorded rather than hidden.** Dropping it means any principal holding read on a
//!   finding may post `-1.0` on each of its citations and pin it to `disputed` **for as long as
//!   nobody outweighs it**: the trail is append-only by design
//!   (`20260724000110_citation_audits.sql:12-17`), decay means the most recent writer dominates,
//!   and the evidence read surfaces **no attribution**, so there is no retraction and no visible
//!   culprit. The two mitigations that would close it — surfacing per-audit attribution on the
//!   evidence read, and rate-limiting audits per principal per finding — are NOT built. Whoever
//!   picks this up should treat them as the price of the promotion mechanic, not as polish. The
//!   `Author` arm below is unaffected and still holds: the citer may never grade its own work.
//! * [`AuditAuthority::Author`] — the self-audit arm. The projection favours recency and the sweep
//!   clears a finding once its citations are covered (spec §4.1, §6.3), so a citer who audits its own
//!   work both inflates its own standing *and* removes the finding from the adversary's queue.
//!
//! # The self-audit arm asks a HISTORICAL question, not a current-capability one
//!
//! The first cut used `can_modify_resource` as a *sufficient proxy* for "authored the citation", on
//! the argument that it only ever over-refuses. That is true in one direction and false in the
//! other. `can_modify_resource` answers *"can you write this finding **now**?"*; authorship is a
//! fact about the past. They come apart — and the citer is readmitted to grading its own work —
//! the moment write is removed while read is kept:
//!
//! * a direct `can_write` grant is revoked while a team/context/cogmap read path remains;
//! * the finding's home is reassigned to another owner (`reassign_resource`), and
//!   `originator_profile_id` confers nothing —
//!   `20260715000040_demote_originator_from_access.sql` says so in as many words: *"originator is
//!   provenance only, not access"*;
//! * a team role is demoted below the authoring roles `context_authorable_by_profile` /
//!   `cogmap_authorable_by_profile` require, while role-agnostic `profile_effective_teams` keeps the
//!   read.
//!
//! So the arm now asks the exact question spec §7 asks — *"did this principal emit the event that
//! contributed this citation?"* — through `citation_contributed_by_profile`
//! (`20260724000110_citation_audits.sql`), which reads `kb_block_provenance.contributed_by_event_id`
//! → `kb_events.emitter_entity_id` → `kb_entities.profile_id`. That row is immutable, so no later
//! access change moves the answer. The `can_modify_resource` proxy is **kept as a second probe**,
//! belt and braces: it still correctly refuses a present co-editor who wrote none of the citation,
//! which is the safe direction to be wrong in.
//!
//! **Residual, stated rather than papered over.** The historical probe is exact for citations whose
//! contributing event carries an emitter that resolves to a profile — every citation any temper
//! write path produces, since `writes::resolve_emitter` mints the entity from the authoring
//! principal. It cannot attribute a citation contributed under a *shared* credential (two agents on
//! one `client_id` are one `emitter_entity_id`, which is exactly the failure
//! `docs/auth/machine-token-contract.md` §C tells operators to avoid), nor one whose contributing
//! event predates the entity it names. In both cases the arm falls back to the `can_modify_resource`
//! proxy, i.e. to the pre-fix behavior — never to admission-by-default.
//!
//! # Machine reach — grounded, not assumed (spec §7 decision 5)
//!
//! §5.2 requires the auditor to be a registered machine principal, provisioned with
//! `--team <ref>:member`. The failure this note guards is silent: everything compiles, every test
//! passes, and every audit 404s in production because the machine cannot see the corpus. What was
//! established, from the code:
//!
//! * `--team <ref>:member` writes exactly one row per team — `INSERT INTO kb_team_members
//!   (team_id, profile_id, role)` (`machine_registration_service.rs:120-131`, `apply_reach`). It
//!   writes **no** `kb_access_grants` row; grants come only from the separate `--cogmap` reach
//!   (`:133-153`).
//! * `profile_effective_teams` is **role-agnostic** — any `kb_team_members` row on an active team
//!   counts, `member` included (`20260703000001_team_metadata_soft_delete.sql:38-44`).
//! * `resources_visible_to` carries a cogmap-membership arm: *"resources homed in a cognitive map
//!   joined to a REACHABLE team"* — `kb_team_cogmaps` ⋈ `reachable_teams` ⋈ `kb_resource_homes`
//!   on `anchor_table = 'kb_cogmaps'` (`20260715000040_demote_originator_from_access.sql:38-43`,
//!   the live body).
//!
//! Findings are cogmap-homed by spec §6.2. So a machine in a team joined to the finding's cogmap
//! **is** returned by `resources_visible_to`, and the read arm of this gate admits it. Standing
//! does not interfere: a machine is born `denied` (`machine_registration_service.rs:275-280`), but
//! standing is read by `has_system_access`, which `resources_visible_to` never consults, and
//! machine admission checks registration + revocation only (`profile_service.rs:230-249`).
//!
//! **The hazard is on the other side.** `can_modify_resource`'s container-write cascade calls
//! `cogmap_authorable_by_profile` (`20260715000040_demote_originator_from_access.sql:87-94`),
//! which is `profile_explicit_grant(p, 'write', 'kb_cogmaps', map)`
//! (`20260701000001_cogmap_write_tightening.sql:36-39`) — a **write grant**, direct or via a
//! reachable team. Team membership alone does not confer it, which is exactly why the
//! `--team <ref>:member` shape works. But `provision --cogmap <ref>` **defaults to write** (`:ro`
//! is the opt-out; `apply_reach` sets `can_write: grant.can_write()`), so provisioning the auditor
//! with a writable cogmap reach — or joining it to a team that holds a team-anchored write grant on
//! that cogmap — makes it [`AuditAuthority::Author`] for *every* finding homed there and refuses
//! *every* audit. Provision the auditor with team membership, and cogmap reach only as `:ro`.
//!
//! CONFORM: the `ScopedAuthority` trait + sealed proof (`super`, `mod.rs:54-133`); the
//! `NotFound`-dialect impls (`super::read_gates`). EXTEND: spec §7, discharged here; spec §6.5
//! (job completion), discharged by [`AuditorJobAuthority`].

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::{BlockId, CogmapId, ProfileId, ResourceId};
use temper_substrate::readback;

use super::ScopedAuthority;
use crate::error::{ApiError, ApiResult};
use crate::services::machine_client_service;

/// **The one refusal string every finding-shaped 404 renders**, across both the audit gate and the
/// two reads beside it.
///
/// Five sites must be indistinguishable to a caller: an unknown block ([`finding_of_block`]), an
/// unreadable or absent finding ([`AuditAuthority::denial`]), the path/block transposition guard
/// and the audit-trail read (`citation_audit_service`), and the evidence read
/// (`evidential_standing_service`). If any one of them names its own cause, the set stops being an
/// equivalence class and a prober can diff the refusals to learn which of "no such finding",
/// "exists but you may not read it", and "you may read it but you wrote it" applies — the exact
/// disclosure the 404-in-place-of-403 choice exists to prevent.
///
/// Deliberately a shared constant rather than five string literals. Five literals is not the same
/// defence written five times; it is five defences that will drift, silently, the first time one
/// of them is "improved". `citation_audit_trail_test` asserts the equality directly.
///
/// The wording covers all three causes without electing between them — it is ambiguous *by
/// construction*, which is the point.
pub(crate) const FINDING_REFUSAL: &str = "finding not found or not readable";

/// Is `caller` a registered, unrevoked machine principal?
///
/// The conjunct spec §7 names, in ONE spelling shared by all three Set 5 gates — the audit write
/// ([`AuditAuthority`]), the job completion ([`AuditorJobAuthority`]), and the dispatch tick
/// ([`require_machine_principal`]). Three copies of a registry lookup is three places for a future
/// "is this still needed?" to be answered differently.
async fn is_machine_principal(pool: &PgPool, caller: ProfileId) -> ApiResult<bool> {
    machine_client_service::is_registered_principal(pool, caller).await
}

/// The auditor's **dispatch tick** gate — `POST /api/auditor/dispatch`.
///
/// Not a [`ScopedAuthority`], deliberately: a dispatch tick names no subject. The caller passes a
/// cap and nothing else, so there is no scope to seal into a proof and nothing for a transposition
/// to transpose. What it *is* is the endpoint half of the queue's principal scoping — the SQL half
/// lives in `workflow_job_claim`'s reach constraint
/// (`migrations/20260724000130_audit_drift_sweep.sql`), and neither half is sufficient alone:
///
/// * without the reach constraint, one registered machine could claim another tenant's jobs and
///   read their `cogmap_id` + finding-id payloads;
/// * without this gate, *any* authenticated principal could run the tick, and — inside whatever
///   reach it does hold — take the auditor's jobs `in_progress` and never complete them, which
///   after `max_attempts` reap cycles kills them (`20260705000001_workflow_jobs.sql:110-128`).
///
/// `Forbidden`, not `NotFound`: there is no subject whose existence a refusal could confirm, and a
/// 404 on a fixed route would just be a lie. The dialect argument that makes the other two gates
/// `NotFound` (`ScopedAuthority::denial`, `mod.rs:86-95`) does not apply where nothing is named.
pub(crate) async fn require_machine_principal(pool: &PgPool, caller: ProfileId) -> ApiResult<()> {
    if is_machine_principal(pool, caller).await? {
        return Ok(());
    }
    Err(ApiError::Forbidden)
}

/// The finding a block belongs to — **the only way an audit's subject may be named.**
///
/// An audit write lands on a block; the authorization subject is the block's owning resource.
/// Letting a caller pass a finding id alongside a block id would let it authorize over a finding
/// it can read while writing onto a block of one it cannot — the transposition the sealed
/// [`super::Authorized`] proof exists to stop (`mod.rs:95-117`). One spelling of the lookup, two
/// callers: [`citation_subject`] (which the backend command builds its sealed subject from) and
/// `citation_audit_service`'s path/block transposition check — the latter a pure lookup, never an
/// authorization decision.
///
/// Mirrors the resolution the SQL entry function already performs — `SELECT resource_id INTO
/// v_resource FROM kb_content_blocks WHERE id = v_block`
/// (`20260724000110_citation_audits.sql:119`) — and refuses an unknown block with
/// [`ApiError::NotFound`], the same dialect [`AuditAuthority::denial`] uses, so "no such block" and
/// "a block on a finding you may not audit" are indistinguishable to the caller.
pub(crate) async fn finding_of_block(pool: &PgPool, block: BlockId) -> ApiResult<ResourceId> {
    let resource = sqlx::query_scalar!(
        "SELECT resource_id FROM kb_content_blocks WHERE id = $1",
        block.uuid(),
    )
    .fetch_optional(pool)
    .await?;

    resource
        .map(ResourceId::from)
        .ok_or_else(|| ApiError::NotFound(FINDING_REFUSAL.to_string()))
}

/// The scope of an audit decision: the **citation**, plus the finding it hangs on.
///
/// The finding alone is not enough. `AuditAuthority`'s self-audit arm asks *"did you contribute
/// **this citation**?"*, which is a `(block, source)` question — the same grain
/// `kb_block_provenance` and `kb_citation_audits` are keyed on — while readability is a question
/// about the finding. Carrying all three inside the sealed proof keeps the act from re-naming any
/// of them: the tick reads `finding` back out of `proof.subject()` rather than re-deriving it.
///
/// `source` is a bare `Uuid` because only **resource-kind** citations are auditable (spec §6.2), so
/// the discrimination happens once, at the command boundary, before a subject exists at all. That
/// also keeps this type `Copy`, which `ScopedAuthority::Subject` requires and `ProvenanceSource`
/// (whose `Remote` variant holds a `String`) cannot be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CitationSubject {
    finding: ResourceId,
    block: BlockId,
    source: Uuid,
}

impl CitationSubject {
    /// The finding the audited citation hangs on — the standing subject the post-write clock ticks.
    pub(crate) fn finding(&self) -> ResourceId {
        self.finding
    }
}

/// Resolve `(block, source)` into the sealed audit subject, deriving the finding server-side.
///
/// The one constructor for [`CitationSubject`] (its fields are private to this module), so no
/// caller can assemble a subject whose finding disagrees with its block.
pub(crate) async fn citation_subject(
    pool: &PgPool,
    block: BlockId,
    source: Uuid,
) -> ApiResult<CitationSubject> {
    Ok(CitationSubject {
        finding: finding_of_block(pool, block).await?,
        block,
        source,
    })
}

/// Who may record a citation audit on a finding.
///
/// One admitting arm and two denials. The denials are named arms, never an absence and never an
/// `Err` out of `resolve` — an error short-circuits `authorize` before [`ScopedAuthority::denial`]
/// runs, which would bypass this domain's chosen refusal dialect (`mod.rs:69-74`).
///
/// There is deliberately **no `NotMachine` arm**: a human may audit (see the module doc's product
/// decision and the open risk it accepts). Registration is still what the *dispatch tick* and *job
/// completion* require — [`require_machine_principal`] and [`AuditorJobAuthority`] — because those
/// speak for the queue, not for a citation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuditAuthority {
    /// A principal that can read the finding and did not contribute the citation. **The only arm
    /// that admits an audit** — machine or human alike.
    Auditor,
    /// DENIAL — the caller contributed this citation (or can still modify the finding, the
    /// belt-and-braces proxy). The self-audit prohibition of spec §7: enforced here or nowhere.
    Author,
    /// DENIAL — the caller cannot see the finding at all.
    Unreadable,
}

#[async_trait]
impl ScopedAuthority for AuditAuthority {
    /// The **citation**, never a caller-named finding. See [`CitationSubject`] for why the subject
    /// carries all three ids and [`citation_subject`] for where they come from.
    type Subject = CitationSubject;

    /// Three probes, in the only order that can short-circuit safely.
    ///
    /// Readability runs FIRST and that placement is a leak decision, not a cost one: every later
    /// arm refuses in the same `NotFound` dialect, so ordering them differently would not leak — but
    /// a principal who cannot read the finding must not pay for, or be distinguishable by, probes
    /// about a subject it may not know exists. The two authorship probes are last, and both must run
    /// for the admitting arm.
    ///
    /// Every probe is a **call into a SQL predicate, never a restatement** of one (`mod.rs:65-66`):
    ///
    /// * Readability is [`readback::is_resource_visible`], extracted by Task 4 for exactly this
    ///   reason — the standing read this gate sits beside asks the same question through the same
    ///   function, so the two cannot drift (`readback/mod.rs:133-162`).
    /// * Citation authorship is `citation_contributed_by_profile`
    ///   (`20260724000110_citation_audits.sql`) — the exact historical fact, see the module doc.
    /// * The authorship proxy is `can_modify_resource`, the same call and the same query text as
    ///   production's write gate (`db_backend.rs:469-483`).
    async fn resolve(
        pool: &PgPool,
        caller: ProfileId,
        subject: CitationSubject,
    ) -> ApiResult<Self> {
        if !readback::is_resource_visible(pool, caller, subject.finding)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
        {
            return Ok(AuditAuthority::Unreadable);
        }

        // The exact question (historical): did this principal emit the event that contributed this
        // citation? Keyed on the citation, not the finding — a multi-source block has one author per
        // source, and refusing an auditor for a source it did not contribute would be wrong in the
        // costly direction.
        let contributed: Option<bool> = sqlx::query_scalar!(
            "SELECT citation_contributed_by_profile($1, 'resource'::provenance_source_kind, $2, $3)",
            subject.block.uuid(),
            subject.source,
            *caller,
        )
        .fetch_one(pool)
        .await?;
        if contributed.unwrap_or(false) {
            return Ok(AuditAuthority::Author);
        }

        // The proxy, kept as a second probe. It over-refuses a present co-editor who wrote none of
        // the citation — the safe direction — and the auditor this gate exists for (a machine with
        // team membership and no cogmap write grant; see the module doc) is untouched by it.
        let can_modify: Option<bool> = sqlx::query_scalar!(
            "SELECT can_modify_resource($1, $2)",
            *caller,
            *subject.finding,
        )
        .fetch_one(pool)
        .await?;

        Ok(if can_modify.unwrap_or(false) {
            AuditAuthority::Author
        } else {
            AuditAuthority::Auditor
        })
    }

    /// **Both** non-admitting arms. `Author` is as much a refusal as `Unreadable`; collapsing
    /// either into the admitting side — or forgetting one here — silently restores self-grading or
    /// admits a principal that cannot even read the finding.
    fn is_denial(&self) -> bool {
        matches!(self, AuditAuthority::Author | AuditAuthority::Unreadable)
    }

    /// `NotFound`, not `Forbidden` — and deliberately so on every arm.
    ///
    /// The evidence **read** over this same subject is already leak-safe by returning no row: the
    /// `gated` CTE in `resource_standing_shape` yields zero rows to a principal who cannot read the
    /// finding (`20260724000120_standing_citation_components.sql:326`), which
    /// `evidential_standing_service.rs:43` turns into `ApiError::NotFound`. Refusing the audit
    /// **write** with `Forbidden` would confirm that a guessed finding id names a real finding —
    /// an existence oracle standing directly beside a read built to avoid one.
    ///
    /// It matters for `Author` too, for a second reason: a `Forbidden` distinguishable from the
    /// unreadable case would tell a prober "you may see this finding but you wrote it", leaking the
    /// authorship relation the audit trail otherwise only exposes to readers.
    fn denial() -> ApiError {
        ApiError::NotFound(FINDING_REFUSAL.to_string())
    }
}

/// Who may complete a cognitive map's active citation-audit job — `POST /api/auditor/{cogmap}/complete`.
///
/// Spec §6.5. The endpoint exists because the auditor advances no watermark, so without it the
/// reaper re-dispatches every audited cogmap and appends duplicate verdicts to a trail that cannot
/// retract them.
///
/// **The gate and the effect are two different narrowings, and both are needed.** This authority
/// answers *"may you speak about this cogmap's auditor queue at all?"* — readable cogmap, registered
/// machine principal. It deliberately does **not** answer *"is this your job?"*, because that
/// question has a third answer — *"there is no job"* — which is a legitimate no-op (a manual audit
/// outside the dispatch loop, an already-completed job, a reaped lease) and not a refusal. So the
/// claim check lives in the SQL effect instead: `workflow_job_complete_claimed` transitions only a
/// row that is `in_progress` **and** `claimed_by_profile_id = caller`
/// (`migrations/20260724000130_audit_drift_sweep.sql`), and matches nothing otherwise. A caller past
/// this gate that holds no such job therefore performs a guaranteed no-op — it can neither terminate
/// a never-dispatched `pending` job (indefinite suppression of a cogmap's auditing) nor free another
/// session's in-flight slot (duplicate concurrent audit sessions on one finding list).
///
/// **Readability, not writability** — and that is not laziness. §C of
/// `docs/auth/machine-token-contract.md` establishes that the auditor is provisioned *without*
/// cogmap write precisely so it is not classified [`AuditAuthority::Author`], so a write gate here
/// would 403 the one caller this endpoint exists for. Readability is also exactly the predicate that
/// put the job in this principal's queue: `audit_drift_sweep` and `workflow_job_claim`'s reach
/// constraint both route through `steward_candidate_cogmaps`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuditorJobAuthority {
    /// A registered machine principal that can read the cogmap. **The only admitting arm.**
    Auditor,
    /// DENIAL — not a registered, unrevoked machine principal.
    NotMachine,
    /// DENIAL — the cogmap does not exist, or the caller cannot read it. One arm for both, because
    /// the refusal must not tell them apart.
    Unreadable,
}

#[async_trait]
impl ScopedAuthority for AuditorJobAuthority {
    /// The cognitive map whose auditor queue slot is being completed.
    type Subject = CogmapId;

    async fn resolve(pool: &PgPool, caller: ProfileId, cogmap: CogmapId) -> ApiResult<Self> {
        // Existence AND readability in one gated lookup — `anchor_readable_by_profile` is the same
        // predicate `steward_candidate_cogmaps` gates on, so this admits no principal the dispatch
        // would not already have handed work to. An absent row is either "no such cogmap" or "not
        // yours", and the single arm is what keeps them indistinguishable.
        let readable = sqlx::query_scalar!(
            r#"
            SELECT id AS "id!: Uuid"
              FROM kb_cogmaps
             WHERE id = $1
               AND anchor_readable_by_profile($2, 'kb_cogmaps', $1)
            "#,
            *cogmap,
            *caller,
        )
        .fetch_optional(pool)
        .await?;
        if readable.is_none() {
            return Ok(AuditorJobAuthority::Unreadable);
        }

        Ok(if is_machine_principal(pool, caller).await? {
            AuditorJobAuthority::Auditor
        } else {
            AuditorJobAuthority::NotMachine
        })
    }

    fn is_denial(&self) -> bool {
        matches!(
            self,
            AuditorJobAuthority::NotMachine | AuditorJobAuthority::Unreadable
        )
    }

    /// `NotFound` on both arms, matching [`AuditAuthority::denial`] and the 404 this endpoint's
    /// OpenAPI response already documents: cogmap ids travel in share flows, so a `Forbidden` would
    /// confirm that a guessed id names a real map.
    fn denial() -> ApiError {
        ApiError::NotFound("cognitive map not found or not readable".to_string())
    }
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use crate::authz::authorize;
    use sqlx::PgPool;
    use uuid::Uuid;

    /// A finding with one block citing one source, and the four principals whose answers these
    /// gates must separate: the citer that contributed the citation, a registered machine auditor,
    /// a registered machine principal that has read but did not contribute, and an outsider.
    struct Seeded {
        /// Owner of the finding's home AND emitter of the citation → refused as `Author`.
        author: Uuid,
        /// Direct profile-anchored `can_read` grant, registered machine, contributed nothing →
        /// admitted.
        auditor: Uuid,
        /// Same grant as `auditor`, but NOT registered in `kb_machine_clients` — admitted anyway,
        /// since the machine conjunct is deliberately not enforced (see the module doc).
        human_reader: Uuid,
        /// No home, no grant, no team → not visible at all.
        outsider: Uuid,
        finding: Uuid,
        block: Uuid,
        source: Uuid,
    }

    async fn insert_profile(pool: &PgPool, handle: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Register `profile` as a machine principal — the `kb_machine_clients` allowlist row
    /// `resolve_machine_from_claims` requires at the door and `AuditAuthority` now requires at the
    /// gate. Fixture-shaped (raw INSERT, no provisioning side effects) because the reach this test
    /// needs is the grant seeded separately; all this row carries is "is a registered agent".
    async fn register_machine(pool: &PgPool, profile: Uuid, client_id: &str) {
        sqlx::query(
            "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
             VALUES ($1, $1, $2, $2)",
        )
        .bind(client_id)
        .bind(profile)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed(pool: &PgPool) -> Seeded {
        let author = insert_profile(pool, "citer").await;
        let auditor = insert_profile(pool, "auditor").await;
        let human_reader = insert_profile(pool, "human-reader").await;
        let outsider = insert_profile(pool, "stranger").await;
        register_machine(pool, auditor, "auditor@clients").await;

        // A profile-owned context, so `context_authorable_by_profile` (the container-write cascade
        // arm of `can_modify_resource`) admits the author and nobody else. A team-owned context
        // would hand every team member the cascade and make the reader an `Author` too.
        let ctx: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
             VALUES ('kb_profiles', $1, 'findings', 'Findings') RETURNING id",
        )
        .bind(author)
        .fetch_one(pool)
        .await
        .unwrap();

        let finding: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('a finding', '') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let source: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('a source', '') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_resource_homes \
               (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES ($1, 'kb_contexts', $2, $3, $3)",
        )
        .bind(finding)
        .bind(ctx)
        .bind(author)
        .execute(pool)
        .await
        .unwrap();

        // Read WITHOUT write — the shape the whole feature depends on existing. The table's
        // coherence CHECK ((can_write OR can_delete OR can_grant) <= can_read) permits it.
        for reader in [auditor, human_reader] {
            sqlx::query(
                "INSERT INTO kb_access_grants \
                   (subject_table, subject_id, principal_table, principal_id, can_read, can_write, \
                    granted_by_profile_id) \
                 VALUES ('kb_resources', $1, 'kb_profiles', $2, true, false, $3)",
            )
            .bind(finding)
            .bind(reader)
            .bind(author)
            .execute(pool)
            .await
            .unwrap();
        }

        // The citation's contributing event must be emitted by the AUTHOR's entity: that chain
        // (`kb_block_provenance.contributed_by_event_id` → `kb_events.emitter_entity_id` →
        // `kb_entities.profile_id`) is exactly what `citation_contributed_by_profile` walks, and it
        // is the whole point of the historical arm. A migration-seeded event would attribute the
        // citation to the `system` actor and the arm could never fire.
        let author_entity: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'citer@cli') RETURNING id",
        )
        .bind(author)
        .fetch_one(pool)
        .await
        .unwrap();
        let ev: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_events (event_type_id, emitter_entity_id) \
             VALUES ((SELECT id FROM kb_event_types WHERE name = 'block_mutated'), $1) RETURNING id",
        )
        .bind(author_entity)
        .fetch_one(pool)
        .await
        .unwrap();
        let block: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id) \
             VALUES ($1, 0, $2, $2) RETURNING id",
        )
        .bind(finding)
        .bind(ev)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_block_provenance \
               (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq) \
             VALUES ($1, 'resource', $2, $3, 0)",
        )
        .bind(block)
        .bind(source)
        .bind(ev)
        .execute(pool)
        .await
        .unwrap();

        Seeded {
            author,
            auditor,
            human_reader,
            outsider,
            finding,
            block,
            source,
        }
    }

    async fn subject_of(pool: &PgPool, s: &Seeded) -> CitationSubject {
        citation_subject(pool, BlockId::from(s.block), s.source)
            .await
            .expect("the block resolves to its owning finding")
    }

    /// The point of the whole feature: readability, not authorship, admits an audit. Also proves
    /// decision 1 end to end — the subject is derived from the block by [`citation_subject`] and
    /// travels sealed inside the proof, so the act cannot name a different one.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_registered_auditor_who_did_not_cite_may_audit(pool: PgPool) {
        let s = seed(&pool).await;
        let subject = subject_of(&pool, &s).await;
        assert_eq!(subject.finding(), ResourceId::from(s.finding));

        let proof = authorize::<AuditAuthority>(&pool, ProfileId::from(s.auditor), subject)
            .await
            .expect("a registered machine reader who did not cite may audit");
        assert_eq!(proof.authority(), AuditAuthority::Auditor);
        assert_eq!(
            proof.subject().finding(),
            ResourceId::from(s.finding),
            "the proof carries the finding derived from the block, not a caller-named id"
        );
    }

    /// The self-audit denial, on the CURRENT-capability path. Asserts the *arm*, not merely the
    /// refusal: without this the test would still pass if the author were misclassified.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_author_of_the_finding_is_refused(pool: PgPool) {
        let s = seed(&pool).await;
        let subject = subject_of(&pool, &s).await;
        let author = ProfileId::from(s.author);
        // The author is registered as a machine too. Harmless now that the machine conjunct is
        // gone, and deliberately kept: it holds this test on the `Author` arm even if a future pass
        // reinstates a registration conjunct, rather than letting it start passing because the
        // author stopped being an agent.
        register_machine(&pool, s.author, "citer@clients").await;

        assert!(
            readback::is_resource_visible(&pool, author, subject.finding())
                .await
                .unwrap(),
            "the author CAN read its own finding — so readability alone would have admitted it"
        );
        assert_eq!(
            AuditAuthority::resolve(&pool, author, subject)
                .await
                .unwrap(),
            AuditAuthority::Author,
            "the citer must not be able to grade its own work"
        );
        assert!(
            authorize::<AuditAuthority>(&pool, author, subject)
                .await
                .is_err(),
            "and the gate must actually refuse, not merely classify"
        );
    }

    /// **THE HISTORICAL-FACT TEST.** The citer keeps read and LOSES every write path — the home is
    /// reassigned to another owner and its read-only grant is all that remains — so
    /// `can_modify_resource` is now false for it. Under the shipped proxy-only arm this returned
    /// `Auditor` and the citer could grade its own citation; the arm must still say `Author`,
    /// because `kb_block_provenance.contributed_by_event_id` still names its entity.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_citer_who_lost_write_but_kept_read_is_still_refused(pool: PgPool) {
        let s = seed(&pool).await;
        let subject = subject_of(&pool, &s).await;
        let author = ProfileId::from(s.author);
        register_machine(&pool, s.author, "citer@clients").await;

        // Reassign the home to the (non-citing) auditor and give the citer a bare read grant. This
        // is `reassign_resource`'s effect on access, reduced to the rows that matter.
        //
        // `can_modify_resource` has FOUR arms, and stripping write means clearing every one of them
        // that names this principal — the first cut of this fixture moved only `owner_profile_id`
        // and the precondition assert below caught it. `originator_profile_id` is a second
        // home-arm spelling, and the container-write cascade admits whoever may author the HOME
        // (`20260712000020_can_modify_active_floor.sql:64-72`), which for a profile-owned context is
        // its owner — so the context has to move too, or the citer keeps modify through the
        // container regardless of what the resource rows say.
        sqlx::query(
            "UPDATE kb_resource_homes SET owner_profile_id = $1, originator_profile_id = $1 \
             WHERE resource_id = $2",
        )
        .bind(s.auditor)
        .bind(s.finding)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE kb_contexts SET owner_id = $1 \
              WHERE id = (SELECT anchor_id FROM kb_resource_homes \
                           WHERE resource_id = $2 AND anchor_table = 'kb_contexts')",
        )
        .bind(s.auditor)
        .bind(s.finding)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_access_grants \
               (subject_table, subject_id, principal_table, principal_id, can_read, can_write, \
                granted_by_profile_id) \
             VALUES ('kb_resources', $1, 'kb_profiles', $2, true, false, $3)",
        )
        .bind(s.finding)
        .bind(s.author)
        .bind(s.auditor)
        .execute(&pool)
        .await
        .unwrap();

        let can_modify: Option<bool> =
            sqlx::query_scalar!("SELECT can_modify_resource($1, $2)", *author, s.finding)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            can_modify,
            Some(false),
            "precondition: the proxy no longer refuses this principal — if it still did, this test \
             could pass without the historical arm existing at all"
        );
        assert!(
            readback::is_resource_visible(&pool, author, subject.finding())
                .await
                .unwrap(),
            "precondition: it kept read, which is what makes the readmission possible"
        );

        assert_eq!(
            AuditAuthority::resolve(&pool, author, subject)
                .await
                .unwrap(),
            AuditAuthority::Author,
            "authorship is a fact about the past: losing write must not readmit the citer to \
             grading its own citation"
        );
    }

    /// The other side of the same coin: the historical arm is keyed on the CITATION, so a principal
    /// that contributed a *different* citation on the same block is still an auditor for this one.
    /// Without the `source_id` conjunct this would refuse, and the gate would be unable to audit any
    /// block a co-contributor ever touched.
    #[sqlx::test(migrations = "../../migrations")]
    async fn contributing_a_different_citation_on_the_block_does_not_refuse(pool: PgPool) {
        let s = seed(&pool).await;

        // A second source on the SAME block, contributed by the auditor's own entity.
        let other_source: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('other source', '') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let auditor_entity: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'auditor@mcp') RETURNING id",
        )
        .bind(s.auditor)
        .fetch_one(&pool)
        .await
        .unwrap();
        let ev: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_events (event_type_id, emitter_entity_id) \
             VALUES ((SELECT id FROM kb_event_types WHERE name = 'block_mutated'), $1) RETURNING id",
        )
        .bind(auditor_entity)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_block_provenance \
               (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq) \
             VALUES ($1, 'resource', $2, $3, 1)",
        )
        .bind(s.block)
        .bind(other_source)
        .bind(ev)
        .execute(&pool)
        .await
        .unwrap();

        let auditor = ProfileId::from(s.auditor);
        assert_eq!(
            AuditAuthority::resolve(&pool, auditor, subject_of(&pool, &s).await)
                .await
                .unwrap(),
            AuditAuthority::Auditor,
            "the citation it did NOT contribute is still auditable by it"
        );
        let own = citation_subject(&pool, BlockId::from(s.block), other_source)
            .await
            .unwrap();
        assert_eq!(
            AuditAuthority::resolve(&pool, auditor, own).await.unwrap(),
            AuditAuthority::Author,
            "and the citation it DID contribute is not"
        );
    }

    /// **A human may audit, and that is the point.** This test used to assert the opposite: an
    /// earlier pass read spec §7 ¶1's "registered, unrevoked machine principal" as a third denial
    /// arm, and this was the test proving it. The arm was removed by an explicit product decision —
    /// a human audit is not a degraded machine audit but a distinct signal, and human-expressible
    /// audits are wanted as a promotion mechanic (see the module doc, including the open risk that
    /// decision accepts).
    ///
    /// The human reader holds exactly the grant the auditor holds and contributed nothing, so the
    /// two surviving arms both admit it. Not vacuous: it asserts readability separately, so an
    /// `Auditor` verdict reached by the gate silently admitting everything would not look like a
    /// pass here — the neighbouring `Unreadable` and `Author` tests still red in that world.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_human_reader_who_is_not_the_author_may_audit(pool: PgPool) {
        let s = seed(&pool).await;
        let subject = subject_of(&pool, &s).await;
        let reader = ProfileId::from(s.human_reader);

        assert!(
            readback::is_resource_visible(&pool, reader, subject.finding())
                .await
                .unwrap(),
            "precondition: it can read the finding"
        );
        assert_eq!(
            AuditAuthority::resolve(&pool, reader, subject)
                .await
                .unwrap(),
            AuditAuthority::Auditor,
            "a reader who did not contribute the citation may assess it, registered machine or not"
        );
        assert!(authorize::<AuditAuthority>(&pool, reader, subject)
            .await
            .is_ok());
    }

    /// A revoked machine is refused at the DOOR, not here. `resolve_machine_from_claims` is
    /// lookup-or-401 and a revoked row does not authenticate, so a decommissioned agent never
    /// reaches this gate with a token. Asserting a revocation arm HERE would be asserting a
    /// predicate this gate deliberately no longer runs — and, now that humans may audit, revocation
    /// is not what this gate is about. Kept as an explicit statement of where the check lives, so
    /// its absence reads as a decision rather than an oversight.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_revoked_machine_is_still_admitted_by_this_gate_because_authn_stops_it(pool: PgPool) {
        let s = seed(&pool).await;
        let subject = subject_of(&pool, &s).await;
        sqlx::query("UPDATE kb_machine_clients SET revoked_at = now() WHERE profile_id = $1")
            .bind(s.auditor)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            AuditAuthority::resolve(&pool, ProfileId::from(s.auditor), subject)
                .await
                .unwrap(),
            AuditAuthority::Auditor,
            "authorization asks 'may this principal audit this citation'; whether the credential is \
             still live is authentication's question, answered before this gate runs"
        );
    }

    /// The ordinary read denial — the arm that keeps the write no wider than the read.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_principal_who_cannot_read_is_refused(pool: PgPool) {
        let s = seed(&pool).await;
        let subject = subject_of(&pool, &s).await;

        assert_eq!(
            AuditAuthority::resolve(&pool, ProfileId::from(s.outsider), subject)
                .await
                .unwrap(),
            AuditAuthority::Unreadable
        );
        assert!(
            authorize::<AuditAuthority>(&pool, ProfileId::from(s.outsider), subject)
                .await
                .is_err()
        );
    }

    /// Every denial arm refuses in the same dialect, and every one is a denial at all.
    ///
    /// This guards a specific future change: a "let's make the denials consistent" pass that
    /// converts one arm to `Forbidden`, or an `is_denial` "simplification" that drops an arm. The
    /// first turns the gate into an existence oracle beside a read deliberately built to avoid one;
    /// the second silently restores self-grading or re-opens the write to every reader. Neither
    /// would fail any other test in this file.
    #[sqlx::test(migrations = "../../migrations")]
    async fn every_denial_renders_not_found(pool: PgPool) {
        let s = seed(&pool).await;
        let subject = subject_of(&pool, &s).await;
        register_machine(&pool, s.author, "citer@clients").await;

        assert!(AuditAuthority::Author.is_denial());
        assert!(AuditAuthority::Unreadable.is_denial());
        assert!(!AuditAuthority::Auditor.is_denial());

        // `human reader` is deliberately NOT in this list: with the machine conjunct gone it is an
        // ADMITTED principal, covered by `a_human_reader_who_is_not_the_author_may_audit`. The two
        // arms that remain are the two that deny.
        for (label, caller) in [("author", s.author), ("outsider", s.outsider)] {
            let err = authorize::<AuditAuthority>(&pool, ProfileId::from(caller), subject)
                .await
                .expect_err("every non-admitting arm denies");
            assert!(
                matches!(err, ApiError::NotFound(_)),
                "{label} must be refused with NotFound, never Forbidden — a distinguishable \
                 refusal is an existence oracle (and, for the author, an authorship oracle)"
            );
        }
    }

    /// An unknown block refuses in the gate's own dialect rather than surfacing as a 500 — so a
    /// probe cannot tell a nonexistent block from one on a finding it may not audit.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unknown_block_is_not_found(pool: PgPool) {
        let err = citation_subject(&pool, BlockId::new(), Uuid::now_v7())
            .await
            .expect_err("no such block");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    // ── the dispatch-tick gate ────────────────────────────────────────────────

    /// The endpoint half of the Critical fix: an ordinary authenticated principal cannot run the
    /// auditor's tick at all, so it can never be handed a claimed job's payload.
    #[sqlx::test(migrations = "../../migrations")]
    async fn only_a_registered_machine_may_run_the_dispatch_tick(pool: PgPool) {
        let s = seed(&pool).await;
        assert!(
            require_machine_principal(&pool, ProfileId::from(s.auditor))
                .await
                .is_ok(),
            "the registered auditor ticks"
        );
        let err = require_machine_principal(&pool, ProfileId::from(s.human_reader))
            .await
            .expect_err("an unregistered principal must not");
        assert!(
            matches!(err, ApiError::Forbidden),
            "Forbidden, not NotFound: the tick names no subject whose existence a 404 could hide"
        );
    }

    // ── the job-completion gate ───────────────────────────────────────────────

    async fn a_readable_cogmap(pool: &PgPool, principal: Uuid) -> Uuid {
        let telos: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('telos', '') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let cogmap: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('m', $1) RETURNING id",
        )
        .bind(telos)
        .fetch_one(pool)
        .await
        .unwrap();
        let team: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_teams (slug, name) VALUES ('job-team', 'Job Team') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
        )
        .bind(team)
        .bind(principal)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
            .bind(cogmap)
            .bind(team)
            .execute(pool)
            .await
            .unwrap();
        cogmap
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_registered_machine_that_reads_the_cogmap_may_complete(pool: PgPool) {
        let s = seed(&pool).await;
        let cogmap = a_readable_cogmap(&pool, s.auditor).await;
        let proof = authorize::<AuditorJobAuthority>(
            &pool,
            ProfileId::from(s.auditor),
            CogmapId::from(cogmap),
        )
        .await
        .expect("the provisioned auditor completes its own job");
        assert_eq!(proof.authority(), AuditorJobAuthority::Auditor);
        assert_eq!(proof.subject(), CogmapId::from(cogmap));
    }

    /// A mere reader is no longer enough — the shipped gate admitted anyone who could read the
    /// cogmap, which is how a human could suppress a map's auditing or induce duplicate sessions.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_human_reader_of_the_cogmap_may_not_complete(pool: PgPool) {
        let s = seed(&pool).await;
        let cogmap = a_readable_cogmap(&pool, s.human_reader).await;
        assert_eq!(
            AuditorJobAuthority::resolve(
                &pool,
                ProfileId::from(s.human_reader),
                CogmapId::from(cogmap)
            )
            .await
            .unwrap(),
            AuditorJobAuthority::NotMachine
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unreadable_or_absent_cogmap_is_the_same_not_found(pool: PgPool) {
        let s = seed(&pool).await;
        let cogmap = a_readable_cogmap(&pool, s.auditor).await;

        for (label, caller, subject) in [
            ("unreadable", s.outsider, cogmap),
            ("absent", s.auditor, Uuid::now_v7()),
        ] {
            assert_eq!(
                AuditorJobAuthority::resolve(
                    &pool,
                    ProfileId::from(caller),
                    CogmapId::from(subject)
                )
                .await
                .unwrap(),
                AuditorJobAuthority::Unreadable,
                "{label}"
            );
            let err = authorize::<AuditorJobAuthority>(
                &pool,
                ProfileId::from(caller),
                CogmapId::from(subject),
            )
            .await
            .expect_err("both arms deny");
            assert!(
                matches!(err, ApiError::NotFound(_)),
                "{label} must be 404 — one arm for 'no such map' and 'not yours'"
            );
        }
    }
}
