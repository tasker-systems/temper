use super::*;
use crate::authz::authorize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::test_support::profile;

/// A finding with one block citing one source, and the five principals whose answers these
/// gates must separate: the citer that contributed the citation, a registered machine auditor,
/// a human reader that did not contribute, a reader of the finding alone, and an outsider.
struct Seeded {
    /// Owner of the finding's home AND emitter of the citation → refused as `Author`.
    author: Uuid,
    /// Direct profile-anchored `can_read` grant, registered machine, contributed nothing →
    /// admitted.
    auditor: Uuid,
    /// Same grant as `auditor`, but NOT registered in `kb_machine_clients` — admitted anyway,
    /// since the machine conjunct is deliberately not enforced (see the module doc).
    human_reader: Uuid,
    /// Registered machine with a read grant on the finding and none on the cited source →
    /// refused as `UnreadableSource`.
    finding_only_reader: Uuid,
    /// No home, no grant, no team → not visible at all.
    outsider: Uuid,
    finding: Uuid,
    block: Uuid,
    source: Uuid,
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
    let author = profile::insert(pool, "citer").await;
    let auditor = profile::insert(pool, "auditor").await;
    let human_reader = profile::insert(pool, "human-reader").await;
    let outsider = profile::insert(pool, "stranger").await;
    let finding_only_reader = profile::insert(pool, "finding-only-reader").await;
    register_machine(pool, auditor, "auditor@clients").await;
    register_machine(pool, finding_only_reader, "finding-only@clients").await;

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
    for resource in [finding, source] {
        sqlx::query(
            "INSERT INTO kb_resource_homes \
               (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES ($1, 'kb_contexts', $2, $3, $3)",
        )
        .bind(resource)
        .bind(ctx)
        .bind(author)
        .execute(pool)
        .await
        .unwrap();
    }

    // Read WITHOUT write — the shape the whole feature depends on existing. The table's
    // coherence CHECK ((can_write OR can_delete OR can_grant) <= can_read) permits it. The
    // auditors read both ends of the citation; `finding_only_reader` reads only the finding.
    let read_grants = [
        (finding, auditor),
        (source, auditor),
        (finding, human_reader),
        (source, human_reader),
        (finding, finding_only_reader),
    ];
    for (resource, reader) in read_grants {
        sqlx::query(
            "INSERT INTO kb_access_grants \
               (subject_table, subject_id, principal_table, principal_id, can_read, can_write, \
                granted_by_profile_id) \
             VALUES ('kb_resources', $1, 'kb_profiles', $2, true, false, $3)",
        )
        .bind(resource)
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
        finding_only_reader,
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

    let proof =
        authorize::<AuditAuthority>(&pool, Principal::Bare(ProfileId::from(s.auditor)), subject)
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
        AuditAuthority::resolve(&pool, Principal::Bare(author), subject)
            .await
            .unwrap(),
        AuditAuthority::Author,
        "the citer must not be able to grade its own work"
    );
    assert!(
        authorize::<AuditAuthority>(&pool, Principal::Bare(author), subject)
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
        AuditAuthority::resolve(&pool, Principal::Bare(author), subject)
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
    // Readable by the auditor, so only authorship separates the two citations.
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, can_read, can_write, \
            granted_by_profile_id) \
         VALUES ('kb_resources', $1, 'kb_profiles', $2, true, false, $3)",
    )
    .bind(other_source)
    .bind(s.auditor)
    .bind(s.author)
    .execute(&pool)
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
        AuditAuthority::resolve(&pool, Principal::Bare(auditor), subject_of(&pool, &s).await)
            .await
            .unwrap(),
        AuditAuthority::Auditor,
        "the citation it did NOT contribute is still auditable by it"
    );
    let own = citation_subject(&pool, BlockId::from(s.block), other_source)
        .await
        .unwrap();
    assert_eq!(
        AuditAuthority::resolve(&pool, Principal::Bare(auditor), own)
            .await
            .unwrap(),
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
        AuditAuthority::resolve(&pool, Principal::Bare(reader), subject)
            .await
            .unwrap(),
        AuditAuthority::Auditor,
        "a reader who did not contribute the citation may assess it, registered machine or not"
    );
    assert!(
        authorize::<AuditAuthority>(&pool, Principal::Bare(reader), subject)
            .await
            .is_ok()
    );
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
        AuditAuthority::resolve(&pool, Principal::Bare(ProfileId::from(s.auditor)), subject)
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
        AuditAuthority::resolve(&pool, Principal::Bare(ProfileId::from(s.outsider)), subject)
            .await
            .unwrap(),
        AuditAuthority::Unreadable
    );
    assert!(authorize::<AuditAuthority>(
        &pool,
        Principal::Bare(ProfileId::from(s.outsider)),
        subject
    )
    .await
    .is_err());
}

/// A verdict weighs the cited source, so reading the finding is not enough: a registered
/// machine with a read grant on the finding and none on the source is refused. Asserts the arm,
/// and the precondition that the finding alone would have admitted it.
#[sqlx::test(migrations = "../../migrations")]
async fn a_reader_of_the_finding_who_cannot_read_the_source_is_refused(pool: PgPool) {
    let s = seed(&pool).await;
    let subject = subject_of(&pool, &s).await;
    let caller = ProfileId::from(s.finding_only_reader);

    assert!(
        readback::is_resource_visible(&pool, caller, subject.finding())
            .await
            .unwrap(),
        "the caller CAN read the finding — so the finding check alone would have admitted it"
    );
    assert_eq!(
        AuditAuthority::resolve(&pool, Principal::Bare(caller), subject)
            .await
            .unwrap(),
        AuditAuthority::UnreadableSource
    );
    let err = authorize::<AuditAuthority>(&pool, Principal::Bare(caller), subject)
        .await
        .expect_err("a caller who cannot read the cited source may not weigh it");
    assert!(matches!(err, ApiError::NotFound(_)));
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
    assert!(AuditAuthority::UnreadableSource.is_denial());
    assert!(!AuditAuthority::Auditor.is_denial());

    // `human reader` is deliberately NOT in this list: with the machine conjunct gone it is an
    // ADMITTED principal, covered by `a_human_reader_who_is_not_the_author_may_audit`. The
    // three arms that remain are the three that deny.
    for (label, caller) in [
        ("author", s.author),
        ("outsider", s.outsider),
        ("finding-only reader", s.finding_only_reader),
    ] {
        let err =
            authorize::<AuditAuthority>(&pool, Principal::Bare(ProfileId::from(caller)), subject)
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
        require_machine_principal(&pool, Principal::Bare(ProfileId::from(s.auditor)))
            .await
            .is_ok(),
        "the registered auditor ticks"
    );
    let err = require_machine_principal(&pool, Principal::Bare(ProfileId::from(s.human_reader)))
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
        Principal::Bare(ProfileId::from(s.auditor)),
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
            Principal::Bare(ProfileId::from(s.human_reader)),
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
                Principal::Bare(ProfileId::from(caller)),
                CogmapId::from(subject)
            )
            .await
            .unwrap(),
            AuditorJobAuthority::Unreadable,
            "{label}"
        );
        let err = authorize::<AuditorJobAuthority>(
            &pool,
            Principal::Bare(ProfileId::from(caller)),
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
