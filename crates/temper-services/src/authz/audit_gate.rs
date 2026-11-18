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
//! **readability**, of the finding and of the cited source — a deliberate widening (a write
//! authorized by a read predicate), spec §7.
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
//! * `--team <ref>:member` writes exactly one `kb_team_members` row per team (`apply_reach`,
//!   through `team_membership::membership_write`). It writes **no** `kb_access_grants` row; grants
//!   come only from the separate `--cogmap` reach (`apply_reach`'s `insert_grant` loop).
//! * `profile_effective_teams` is **role-agnostic** — any `kb_team_members` row on an active team
//!   counts, `member` included (`20260703000001_team_metadata_soft_delete.sql:38-44`).
//! * `resources_visible_to` carries a cogmap-membership arm: *"resources homed in a cognitive map
//!   joined to a REACHABLE team"* — `kb_team_cogmaps` ⋈ `reachable_teams` ⋈ `kb_resource_homes`
//!   on `anchor_table = 'kb_cogmaps'` (`20260715000040_demote_originator_from_access.sql:38-43`,
//!   the live body).
//!
//! Findings are cogmap-homed by spec §6.2. So a machine in a team joined to the finding's cogmap
//! **is** returned by `resources_visible_to`, and the read arm of this gate admits it. Standing
//! does not interfere: a machine is born `denied` (`machine_registration_service::provision`'s
//! `principal_standing_apply` call), but standing is read by `has_system_access`, which
//! `resources_visible_to` never consults, and machine admission checks registration, revocation and
//! issuer only (`profile_service::resolve_machine_from_claims`).
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
//! CONFORM: the `ScopedAuthority` trait + sealed proof (`super::ScopedAuthority`,
//! `super::Authorized`); the `NotFound`-dialect impls (`super::read_gates`). EXTEND: spec §7,
//! discharged here; spec §6.5 (job completion), discharged by [`AuditorJobAuthority`].

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::{BlockId, CogmapId, ProfileId, ResourceId};
use temper_substrate::readback;

use super::{Principal, ScopedAuthority};
use crate::error::{ApiError, ApiResult};
use crate::services::machine_client_service;

/// **The one refusal string every finding-shaped 404 renders**, across both the audit gate and the
/// two reads beside it.
///
/// Five sites must be indistinguishable to a caller: an unknown block ([`finding_of_block`]), an
/// unreadable or absent finding or cited source ([`AuditAuthority::denial`]), the path/block
/// transposition guard
/// and the audit-trail read (`citation_audit_service`), and the evidence read
/// (`evidential_standing_service`). If any one of them names its own cause, the set stops being an
/// equivalence class and a prober can diff the refusals to learn which of "no such finding",
/// "exists but you may not read it", "you may read it but not its source", and "you may read it
/// but you wrote it" applies — the exact disclosure the 404-in-place-of-403 choice exists to
/// prevent.
///
/// Deliberately a shared constant rather than five string literals. Five literals is not the same
/// defence written five times; it is five defences that will drift, silently, the first time one
/// of them is "improved". `citation_audit_trail_test` asserts the equality directly.
///
/// The wording covers every cause without electing between them — it is ambiguous *by
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
/// `NotFound` (`ScopedAuthority::denial`'s doc) does not apply where nothing is named.
/// Takes the crate-internal [`Principal`] rather than a bare id: the db_backend seam's
/// `principal()` routes here arm-for-arm. The probe stays even though the classification already
/// knows a machine from a person, because it asks a narrower question: an **unrevoked** client row
/// (`is_registered_principal`'s definition) — a revoked machine is still a machine, but may not
/// run the tick.
pub(crate) async fn require_machine_principal(
    pool: &PgPool,
    caller: Principal<'_>,
) -> ApiResult<()> {
    if is_machine_principal(pool, caller.profile_id()).await? {
        return Ok(());
    }
    Err(ApiError::Forbidden)
}

/// The finding a block belongs to — **the only way an audit's subject may be named.**
///
/// An audit write lands on a block; the authorization subject is the block's owning resource.
/// Letting a caller pass a finding id alongside a block id would let it authorize over a finding
/// it can read while writing onto a block of one it cannot — the transposition the sealed
/// [`super::Authorized`] proof exists to stop (see its doc). One spelling of the lookup, two
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
    // `AND NOT is_folded`: a folded block is not a live citation (the defined-dangling-state
    // design) — auditing one must be a defined refusal, never a silent success on a
    // gone-citation. The filter lands the folded row in the same zero-rows arm as an unknown
    // block, so both render NotFound: the gate's standing dialect, denying existence rather
    // than opening an oracle over which refusals are which.
    let resource = sqlx::query_scalar!(
        "SELECT resource_id FROM kb_content_blocks WHERE id = $1 AND NOT is_folded",
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
/// `kb_block_provenance` and `kb_citation_audits` are keyed on — and readability is asked of both
/// the finding and the source. Carrying all three inside the sealed proof keeps the act from re-naming any
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
/// One admitting arm and three denials. The denials are named arms, never an absence and never an
/// `Err` out of `resolve` — an error short-circuits `authorize` before [`ScopedAuthority::denial`]
/// runs, which would bypass this domain's chosen refusal dialect (`ScopedAuthority::is_denial`).
///
/// There is deliberately **no `NotMachine` arm**: a human may audit (see the module doc's product
/// decision and the open risk it accepts). Registration is still what the *dispatch tick* and *job
/// completion* require — [`require_machine_principal`] and [`AuditorJobAuthority`] — because those
/// speak for the queue, not for a citation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuditAuthority {
    /// A principal that reads the finding by membership and can read the cited source, and did not
    /// contribute the citation. **The only arm that admits an audit** — machine or human alike.
    Auditor,
    /// DENIAL — the caller contributed this citation (or can still modify the finding, the
    /// belt-and-braces proxy). The self-audit prohibition of spec §7: enforced here or nowhere.
    Author,
    /// DENIAL — the caller cannot see the finding by membership: not at all, or only through
    /// `public_within` reach, which reads and never grades (`is_resource_visible_by_membership`).
    Unreadable,
    /// DENIAL — the caller can see the finding but not the cited source. A verdict weighs a source
    /// against a claim, so a caller who cannot read the source has nothing to weigh. Same refusal
    /// as `Unreadable`: the caller already knows whether it can read the source.
    UnreadableSource,
}

#[async_trait]
impl ScopedAuthority for AuditAuthority {
    /// The **citation**, never a caller-named finding. See [`CitationSubject`] for why the subject
    /// carries all three ids and [`citation_subject`] for where they come from.
    type Subject = CitationSubject;

    /// Four probes, in the only order that can short-circuit safely.
    ///
    /// Readability runs FIRST, finding then source, and that placement is a leak decision, not a
    /// cost one: every later arm refuses in the same `NotFound` dialect, so ordering them
    /// differently would not leak — but a principal who cannot read the finding must not pay for,
    /// or be distinguishable by, probes about a subject it may not know exists. The two authorship
    /// probes are last, and both must run for the admitting arm.
    ///
    /// Every probe is a **call into a SQL predicate, never a restatement** of one
    /// (`ScopedAuthority::resolve`):
    ///
    /// * Readability of the cited source is [`readback::is_resource_visible`], extracted by Task 4
    ///   for exactly this reason — the standing read this gate sits beside asks the same question
    ///   through the same function, so the two cannot drift.
    ///   Readability of the **finding** is [`readback::is_resource_visible_by_membership`]: the
    ///   finding is acted on, so it is judged on membership, never on `public_within` reach.
    ///   The queue's SQL asks the same two sets.
    /// * Citation authorship is `citation_contributed_by_profile`
    ///   (`20260724000110_citation_audits.sql`) — the exact historical fact, see the module doc.
    /// * The authorship proxy is `can_modify_resource`, the same call and the same query text as
    ///   production's write gate (`DbBackend::check_can_modify_next`).
    async fn resolve(
        pool: &PgPool,
        caller: Principal<'_>,
        subject: CitationSubject,
    ) -> ApiResult<Self> {
        let caller = caller.profile_id();
        // The finding, on MEMBERSHIP visibility: a verdict writes about the finding, and a reader
        // who sees it only through `public_within` may read it but never act on it. The queue
        // (`audit_drift_sweep` and its two siblings) gates the finding on the same set, so it never
        // offers what this refuses.
        if !readback::is_resource_visible_by_membership(pool, caller, subject.finding)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
        {
            return Ok(AuditAuthority::Unreadable);
        }
        // The cited source, on READING visibility: it is a reference, not an act on it, so an
        // auditor may judge its own side's finding that cites a public team's material.
        if !readback::is_resource_visible(pool, caller, ResourceId::from(subject.source))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
        {
            return Ok(AuditAuthority::UnreadableSource);
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

    /// **Every** non-admitting arm. `Author` is as much a refusal as `Unreadable` and
    /// `UnreadableSource`; collapsing any of them into the admitting side — or forgetting one here —
    /// silently restores self-grading or admits a principal that cannot read what it would weigh.
    fn is_denial(&self) -> bool {
        matches!(
            self,
            AuditAuthority::Author | AuditAuthority::Unreadable | AuditAuthority::UnreadableSource
        )
    }

    /// `NotFound`, not `Forbidden` — and deliberately so on every arm.
    ///
    /// The evidence **read** over this same subject is already leak-safe by returning no row: the
    /// `gated` CTE in `resource_standing_shape` yields zero rows to a principal who cannot read the
    /// finding (`20260724000120_standing_citation_components.sql:326`), which
    /// `evidential_standing_service::resource_evidence` turns into `ApiError::NotFound`. Refusing
    /// the audit **write** with `Forbidden` would confirm that a guessed finding id names a real
    /// finding — an existence oracle standing directly beside a read built to avoid one.
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

    async fn resolve(pool: &PgPool, caller: Principal<'_>, cogmap: CogmapId) -> ApiResult<Self> {
        let caller = caller.profile_id();
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
mod tests;
