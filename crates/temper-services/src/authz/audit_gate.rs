//! Citation-audit authorization — may this caller record an audit on *this* finding?
//!
//! This is the gate the whole adversarial premise rests on. Set 5's auditor reads a finding's
//! citations and emits a signed defensibility verdict per source; if the gate is wrong, a citer
//! grades its own work, and "assessed by another party" is enforced nowhere.
//!
//! # The widening, and the thing that stops it going too far
//!
//! Every other authored write in temper gates on `can_modify_resource`. An audit must **not**: an
//! auditor that may only assess findings it owns is not an auditor. So the admitting predicate is
//! **readability** — a deliberate widening (a write authorized by a read predicate), spec §7.
//!
//! Readability alone is not sufficient, and that half is load-bearing. The projection favours
//! recency and the sweep clears a finding once its citations are covered (spec §4.1, §6.3), so a
//! citer who audits their own work both inflates its own standing *and* removes the finding from
//! the adversary's queue. Hence the second denial arm: [`AuditAuthority::Author`].
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
//! `NotFound`-dialect impls (`super::read_gates`). EXTEND: spec §7, discharged here.

use async_trait::async_trait;
use sqlx::PgPool;

use temper_core::types::ids::{BlockId, ProfileId, ResourceId};
use temper_substrate::readback;

use super::ScopedAuthority;
use crate::error::{ApiError, ApiResult};

/// The finding a block belongs to — **the only way an audit's subject may be named.**
///
/// An audit write lands on a block; the authorization subject is the block's owning resource.
/// Letting a caller pass a finding id alongside a block id would let it authorize over a finding
/// it can read while writing onto a block of one it cannot — the transposition the sealed
/// [`super::Authorized`] proof exists to stop (`mod.rs:95-117`). Both Task 7's backend command and
/// Task 8's HTTP surface derive the subject here so there is one spelling of the lookup.
///
/// Mirrors the resolution the SQL entry function already performs — `SELECT resource_id INTO
/// v_resource FROM kb_content_blocks WHERE id = v_block`
/// (`20260723000010_citation_audits.sql:119`) — and refuses an unknown block with
/// [`ApiError::NotFound`], the same dialect [`AuditAuthority::denial`] uses, so "no such block" and
/// "a block on a finding you may not audit" are indistinguishable to the caller.
#[cfg_attr(
    not(all(test, feature = "test-db")),
    expect(
        dead_code,
        reason = "Task 6 ships the gate; Task 7's backend command and Task 8's handler are its \
                  only callers and land next. Delete this attribute when the first one wires it."
    )
)]
pub(crate) async fn finding_of_block(pool: &PgPool, block: BlockId) -> ApiResult<ResourceId> {
    let resource = sqlx::query_scalar!(
        "SELECT resource_id FROM kb_content_blocks WHERE id = $1",
        block.uuid(),
    )
    .fetch_optional(pool)
    .await?;

    resource.map(ResourceId::from).ok_or(ApiError::NotFound)
}

/// Who may record a citation audit on a finding.
///
/// One admitting arm and two denials. The denials are named arms, never an absence and never an
/// `Err` out of `resolve` — an error short-circuits `authorize` before [`ScopedAuthority::denial`]
/// runs, which would bypass this domain's chosen refusal dialect (`mod.rs:69-74`).
#[cfg_attr(
    not(all(test, feature = "test-db")),
    expect(
        dead_code,
        reason = "Task 6 ships the gate; Task 7's backend command and Task 8's handler are its \
                  only callers and land next. Delete this attribute when the first one wires it."
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuditAuthority {
    /// Can read the finding, and did not author it. **The only arm that admits an audit.**
    Auditor,
    /// DENIAL — the caller can modify the finding, so it is (by the proxy below) the citer. The
    /// self-audit prohibition of spec §7: enforced here or nowhere.
    Author,
    /// DENIAL — the caller cannot see the finding at all.
    Unreadable,
}

#[async_trait]
impl ScopedAuthority for AuditAuthority {
    /// The **finding**, never the block. See [`finding_of_block`] for why the caller may not name
    /// it and where it must come from instead.
    type Subject = ResourceId;

    /// Two probes, in the only order that can short-circuit: a principal who cannot read the
    /// finding is refused without ever paying for the authorship probe. The admitting arm needs
    /// both answers, so nothing cheaper is available for it — this is a sequence, not a
    /// strongest-first cascade like `grant.rs`'s.
    ///
    /// Both probes are **calls into the incumbent SQL predicates, never restatements** of them
    /// (`mod.rs:65-66`):
    ///
    /// * Readability is [`readback::is_resource_visible`], extracted by Task 4 for exactly this
    ///   reason — the standing read this gate sits beside asks the same question through the same
    ///   function, so the two cannot drift (`readback/mod.rs:133-162`).
    /// * Authorship is `can_modify_resource`, the same call and the same query text as
    ///   production's write gate (`db_backend.rs:469-483`).
    async fn resolve(pool: &PgPool, caller: ProfileId, finding: ResourceId) -> ApiResult<Self> {
        if !readback::is_resource_visible(pool, caller, finding)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
        {
            return Ok(AuditAuthority::Unreadable);
        }

        // `can_modify_resource` is a **sufficient proxy** for "authored the citation", not an exact
        // one. The exact question is "did this principal emit the block's contributing
        // `block_mutated`/`block_annotated` event?"; the proxy over-refuses everyone else who may
        // write the finding — a co-editor holding a write grant, the owner of the home container
        // (`can_modify_resource`'s container-write cascade). Spec §7 names it *"the cheaper
        // sufficient proxy"* and accepts that: an audit is a claim of independence, so refusing a
        // party who could have edited the citation is the safe direction to be wrong in, and the
        // auditor this gate exists for (a machine with team membership and no cogmap write grant —
        // see the module doc) is untouched by the over-refusal.
        let can_modify: Option<bool> =
            sqlx::query_scalar!("SELECT can_modify_resource($1, $2)", *caller, *finding,)
                .fetch_one(pool)
                .await?;

        Ok(if can_modify.unwrap_or(false) {
            AuditAuthority::Author
        } else {
            AuditAuthority::Auditor
        })
    }

    /// **Both** non-admitting arms. `Author` is as much a refusal as `Unreadable`; collapsing it
    /// into the admitting side — or forgetting it here — would silently restore self-grading.
    fn is_denial(&self) -> bool {
        matches!(self, AuditAuthority::Author | AuditAuthority::Unreadable)
    }

    /// `NotFound`, not `Forbidden` — and deliberately so on both arms.
    ///
    /// The evidence **read** over this same subject is already leak-safe by returning no row: the
    /// `gated` CTE in `resource_standing_shape` yields zero rows to a principal who cannot read the
    /// finding (`20260723000020_standing_citation_components.sql:326`), which
    /// `evidential_standing_service.rs:43` turns into `ApiError::NotFound`. Refusing the audit
    /// **write** with `Forbidden` would confirm that a guessed finding id names a real finding —
    /// an existence oracle standing directly beside a read built to avoid one.
    ///
    /// It matters for `Author` too, for a second reason: a `Forbidden` distinguishable from the
    /// unreadable case would tell a prober "you may see this finding but you wrote it", leaking the
    /// authorship relation the audit trail otherwise only exposes to readers.
    fn denial() -> ApiError {
        ApiError::NotFound
    }
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use crate::authz::authorize;
    use sqlx::PgPool;
    use uuid::Uuid;

    /// A finding with one block, and the three principals whose answers this gate must separate:
    /// its author (owns the home), a reader (a read-only grant and nothing more), and an outsider.
    struct Seeded {
        /// Owner of the finding's home → `can_modify_resource` true → refused as `Author`.
        author: Uuid,
        /// Direct profile-anchored `can_read` grant, no write → visible, not modifiable → admitted.
        reader: Uuid,
        /// No home, no grant, no team → not visible at all.
        outsider: Uuid,
        finding: Uuid,
        block: Uuid,
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

    async fn seed(pool: &PgPool) -> Seeded {
        let author = insert_profile(pool, "citer").await;
        let reader = insert_profile(pool, "auditor").await;
        let outsider = insert_profile(pool, "stranger").await;

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

        // The block's genesis/last event FKs borrow a migration-seeded event — the same fixture
        // shortcut `embed_service.rs:637-653` takes, since no event content is read here.
        let ev: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
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

        Seeded {
            author,
            reader,
            outsider,
            finding,
            block,
        }
    }

    /// The point of the whole feature: readability, not authorship, admits an audit. Also proves
    /// decision 1 end to end — the subject is derived from the block by [`finding_of_block`] and
    /// travels sealed inside the proof, so the act cannot name a different one.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_reader_who_is_not_the_author_may_audit(pool: PgPool) {
        let s = seed(&pool).await;

        let subject = finding_of_block(&pool, BlockId::from(s.block))
            .await
            .expect("the block resolves to its owning finding");
        assert_eq!(subject, ResourceId::from(s.finding));

        let proof = authorize::<AuditAuthority>(&pool, ProfileId::from(s.reader), subject)
            .await
            .expect("a reader who did not author the finding may audit it");
        assert_eq!(proof.authority(), AuditAuthority::Auditor);
        assert_eq!(
            proof.subject(),
            ResourceId::from(s.finding),
            "the proof carries the finding derived from the block, not a caller-named id"
        );
    }

    /// The self-audit denial. Asserts the *arm*, not merely the refusal: without this the test
    /// would still pass if the author were misclassified `Unreadable`, and the arm is what a later
    /// reader will reason about.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_author_of_the_finding_is_refused(pool: PgPool) {
        let s = seed(&pool).await;
        let subject = ResourceId::from(s.finding);
        let author = ProfileId::from(s.author);

        assert!(
            readback::is_resource_visible(&pool, author, subject)
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

    /// The ordinary read denial — the arm that keeps the write no wider than the read.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_principal_who_cannot_read_is_refused(pool: PgPool) {
        let s = seed(&pool).await;
        let subject = ResourceId::from(s.finding);

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

    /// Both denial arms refuse in the same dialect, and both are denials at all.
    ///
    /// This guards a specific future change: a "let's make the denials consistent" pass that
    /// converts one arm to `Forbidden`, or an `is_denial` "simplification" that matches only
    /// `Unreadable`. The first turns the gate into an existence oracle beside a read deliberately
    /// built to avoid one; the second silently restores self-grading. Neither would fail any other
    /// test in this file.
    #[sqlx::test(migrations = "../../migrations")]
    async fn both_denials_render_not_found(pool: PgPool) {
        let s = seed(&pool).await;
        let subject = ResourceId::from(s.finding);

        assert!(AuditAuthority::Author.is_denial());
        assert!(AuditAuthority::Unreadable.is_denial());
        assert!(!AuditAuthority::Auditor.is_denial());

        for (label, caller) in [("author", s.author), ("outsider", s.outsider)] {
            let err = authorize::<AuditAuthority>(&pool, ProfileId::from(caller), subject)
                .await
                .expect_err("both arms deny");
            assert!(
                matches!(err, ApiError::NotFound),
                "{label} must be refused with NotFound, never Forbidden — a distinguishable \
                 refusal is an existence oracle (and, for the author, an authorship oracle)"
            );
        }
    }

    /// An unknown block refuses in the gate's own dialect rather than surfacing as a 500 — so a
    /// probe cannot tell a nonexistent block from one on a finding it may not audit.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unknown_block_is_not_found(pool: PgPool) {
        let err = finding_of_block(&pool, BlockId::new())
            .await
            .expect_err("no such block");
        assert!(matches!(err, ApiError::NotFound));
    }
}
