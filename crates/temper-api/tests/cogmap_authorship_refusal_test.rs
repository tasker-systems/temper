#![cfg(feature = "test-db")]
//! One question — *"may this principal author this cognitive map?"* — is answered at three places in
//! `DbBackend`, and this suite is what keeps them answering in **one voice**.
//!
//! `check_cogmap_authorable` establishes the answer with a standalone predicate; `advance_steward_
//! watermark` and `materialize_on_threshold` establish it from a combined auth+data lookup whose
//! `WHERE` already carried `anchor_readable_by_profile`. Different routes, same refusal — and the
//! two combined lookups deliberately skip the read probe the standalone gate performs, because their
//! `WHERE` has already proven what that probe would ask. That divergence is correct and invisible,
//! which is exactly why it needs a test that compares the *outputs* rather than the paths.
//!
//! **`advance_steward_watermark` is the production case, not a hypothetical.** The steward agent met
//! this refusal once per run on the L0 kernel map for eight days — L0 is joined to the root team, so
//! every approved principal reads it, and nobody holds a write grant on it by design. Refused
//! opaquely, the agent could not form a corrective hypothesis and probed instead until it found a
//! genuine authorization bypass. That is what a message costs when it is withheld from someone who
//! already has standing to hear it.
//!
//! The negative pole — a principal who cannot read the subject keeps the argument-free refusal —
//! lives in `invocation_handler_test::open_on_unreadable_cogmap_is_forbidden`, against the same
//! gate. Both poles are needed: a gate that disclosed to everyone would pass this file alone.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{
    AdvanceStewardWatermark, Backend, MaterializeOnThreshold, OpenInvocation, Surface,
};

mod common;

/// The L0 kernel map (`20260625000001`): root-team-joined, so readable by any approved profile, and
/// authorable by nobody. The production shape of "reads it, cannot author it", for free.
const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// A principal that READS L0 and cannot author it — the whole fixture, because L0 supplies both
/// halves. Deliberately grants nothing: a fixture that pre-decided the write grant would make every
/// assertion below vacuous.
async fn reader_of_l0(pool: &PgPool, email: &str) -> ProfileId {
    let profile = common::fixtures::create_test_profile(pool, email).await;
    common::fixtures::approve_standing(pool, profile).await;
    ProfileId::from(profile)
}

/// Assert the refusal is the reader's dialect, and that it carries both halves a caller needs to act
/// on it: which capability was missing, and on what.
fn assert_names_the_missing_capability<T: std::fmt::Debug>(
    result: &Result<T, TemperError>,
    site: &str,
) {
    let Err(TemperError::ForbiddenDetail(msg)) = result else {
        panic!("{site}: a principal who reads the map must be refused in the disclosing dialect, not the argument-free one: {result:?}");
    };
    assert!(
        msg.contains("write grant"),
        "{site}: the refusal must name the capability that was missing: {msg:?}"
    );
    assert!(
        msg.contains(&L0_COGMAP.to_string()),
        "{site}: the refusal must name the subject it refused: {msg:?}"
    );
}

/// The production refusal, at the site that produced it.
///
/// `advance_steward_watermark`'s gate is the `can_write` column of a row whose `WHERE` already
/// required `anchor_readable_by_profile` — so reaching the refusal at all *proves* the caller reads
/// the map, and the disclosing dialect is owed unconditionally. No read probe runs here, and none
/// should: asserting the output rather than the mechanism is what lets that stay true.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn advance_watermark_on_a_readable_unauthorable_map_names_the_grant(pool: PgPool) {
    let profile = reader_of_l0(&pool, "steward-shaped@example.com").await;
    let backend = DbBackend::new(pool.clone(), profile);

    let denied = backend
        .advance_steward_watermark(AdvanceStewardWatermark {
            cogmap: CogmapId::from(L0_COGMAP),
            event_id: None,
            boundary_fingerprint: None,
            origin: Surface::ApiHttp,
        })
        .await;

    assert_names_the_missing_capability(&denied, "advance_steward_watermark");
}

/// The second combined lookup, which reaches the refusal by the same route and must not have drifted
/// from it. Its `HomeAnchor` match is where a copy could plausibly diverge — the context arm
/// deliberately keeps the argument-free refusal, so the cogmap arm is one edit away from inheriting
/// it by accident.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn materialize_on_a_readable_unauthorable_map_names_the_grant(pool: PgPool) {
    let profile = reader_of_l0(&pool, "materializer@example.com").await;
    let backend = DbBackend::new(pool.clone(), profile);

    let denied = backend
        .materialize_on_threshold(MaterializeOnThreshold {
            anchor: HomeAnchor::Cogmap(CogmapId::from(L0_COGMAP)),
            threshold: None,
            origin: Surface::ApiHttp,
        })
        .await;

    assert_names_the_missing_capability(&denied, "materialize_on_threshold");
}

/// The standalone gate, reached through `open_invocation` — the third route, and the only one that
/// pays for a read probe. Present here beside the other two so the file compares all three outputs
/// in one place; `invocation_handler_test` exercises the same site in its own narrative.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn check_cogmap_authorable_on_a_readable_unauthorable_map_names_the_grant(pool: PgPool) {
    let profile = reader_of_l0(&pool, "opener@example.com").await;
    let backend = DbBackend::new(pool.clone(), profile);

    let denied = backend
        .open_invocation(OpenInvocation {
            trigger_kind: "manual".to_string(),
            originating_cogmap: CogmapId::from(L0_COGMAP),
            parent_cogmap: None,
            origin: Surface::ApiHttp,
        })
        .await;

    assert_names_the_missing_capability(&denied, "check_cogmap_authorable");
}
