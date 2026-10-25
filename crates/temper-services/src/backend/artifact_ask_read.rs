//! The batch arm of asking data artifacts: one ask in, the matching artifacts out, whole, in
//! bounded pages (spec `temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md`
//! §7). The composable arm's read path is [`super::query_read`]; this is its sibling, and shares its
//! loader, its validation and its SQL body rather than holding copies of them.
//!
//! **Every page, in order:**
//!
//! 1. the shape the ask names is loaded for the principal — through `shape_loader::load_one` on a
//!    first page, and `shape_loader::load_one_pinned` on a later one, which skips the fold check so
//!    an amendment between pages never strands the enumeration (P17);
//! 2. the ask is validated against it through one [`AskSession`] (refusal steps 1, 3 and 4 of §3.5,
//!    every page), and beside it the refusals the request earns without its shape
//!    ([`AskRequest::refusals`]: the page bound, the request's value budget) — all of them at once,
//!    as one `ApiError::PlanRefused`;
//! 3. one page is read through the gated `artifacts_matching_ask` with the ask's validated bind,
//!    never the caller's own; on the first page, the population through the gated
//!    `artifact_ask_counts`.
//!
//! The continuation carries position and never authority: readability is re-decided at step 1 of
//! every page, so a page after access is revoked answers as an absent shape does.

use sqlx::PgPool;

use crate::backend::substrate_read::artifact_view;
use crate::error::{ApiError, ApiResult};
use crate::services::shape_loader;
use crate::transport::ARTIFACT_ASK_PAGE_CONTENT_BYTES;
use temper_core::error::TemperError;
use temper_core::types::artifact_ask::{
    AskOrder, AskRequest, AskResponse, AskSession, AskedShape, ShapeContext, ShapeSuccessor,
    ValidatedAsk,
};
use temper_core::types::ids::ProfileId;
use temper_substrate::readback;

/// One page of the artifacts matching `request.ask`, for `principal`.
///
/// # Errors
/// `ApiError::PlanRefused` carrying every refusal — the ask's and the request's — when any is
/// raised; a database fault otherwise.
pub async fn ask_page(
    pool: &PgPool,
    principal: ProfileId,
    request: &AskRequest,
) -> ApiResult<AskResponse> {
    let (shapes, amended_since) = load(pool, principal, request).await?;
    let validated = validate(&shapes, request)?;
    read(pool, principal, request, &validated, amended_since).await
}

/// Step 1: the context the ask is validated against, and — on a later page whose pinned row has
/// been amended since — the version now in force.
async fn load(
    pool: &PgPool,
    principal: ProfileId,
    request: &AskRequest,
) -> ApiResult<(ShapeContext, Option<ShapeSuccessor>)> {
    let id = request.ask.shape;
    let (load, amended_since) = match request.after {
        None => (shape_loader::load_one(pool, principal, id).await?, None),
        Some(_) => {
            let pinned = shape_loader::load_one_pinned(pool, principal, id).await?;
            (pinned.load, pinned.amended_since)
        }
    };
    Ok((shape_loader::context_of(vec![(id, load)]), amended_since))
}

/// Step 2: the ask validated through one session, or every refusal at once — the ask's own, then
/// the request's, which never consulted the shape, so an absent shape and an unreadable one are
/// still refused alike (§3.5).
fn validate(shapes: &ShapeContext, request: &AskRequest) -> ApiResult<ValidatedAsk> {
    let mut session: AskSession<'_> = shapes.session();
    let answer = session.validate(&request.ask);
    shape_loader::remember_session(shapes, &session);
    let request_refusals = request.refusals();
    match answer {
        Ok(validated) if request_refusals.is_empty() => Ok(validated),
        Ok(_) => Err(ApiError::PlanRefused {
            refusals: request_refusals,
        }),
        Err(mut refusals) => {
            refusals.extend(request_refusals);
            Err(ApiError::PlanRefused { refusals })
        }
    }
}

/// Step 3: the page, and on the first page the population, both from the validated bind.
async fn read(
    pool: &PgPool,
    principal: ProfileId,
    request: &AskRequest,
    validated: &ValidatedAsk,
    amended_since: Option<ShapeSuccessor>,
) -> ApiResult<AskResponse> {
    let bind = validated.bind();
    let page = readback::artifacts_matching_ask(
        pool,
        readback::AskPageParams {
            principal,
            ask: &bind,
            after: request.after,
            limit: request.rows(),
            content_budget: ARTIFACT_ASK_PAGE_CONTENT_BYTES,
        },
    )
    .await
    .map_err(read_fault)?;
    let population = match request.after {
        None => Some(
            readback::artifact_ask_population(pool, principal, &bind)
                .await
                .map_err(read_fault)?,
        ),
        Some(_) => None,
    };
    Ok(AskResponse {
        shape: AskedShape::from(validated.shape()),
        artifacts: page.artifacts.into_iter().map(artifact_view).collect(),
        oversized: page.oversized,
        population,
        ordered_by: AskOrder::Created,
        next: page.next,
        amended_since,
    })
}

/// A read fault, mapped as the neighbouring artifact reads map theirs (`substrate_read`).
fn read_fault(e: anyhow::Error) -> ApiError {
    ApiError::from(TemperError::Api(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::artifact_ask::ArtifactAsk;
    use temper_core::types::data_artifact::MAX_ARTIFACT_PAGE_ROWS;
    use temper_core::types::ids::ShapeId;
    use temper_core::types::query::RefusalReason;
    use uuid::Uuid;

    fn request(limit: Option<u32>) -> AskRequest {
        AskRequest {
            ask: ArtifactAsk {
                shape: ShapeId::from(Uuid::from_u128(7)),
                predicates: vec![],
                intent: vec![],
                include_folded: false,
                shape_states: vec![],
            },
            limit,
            after: None,
        }
    }

    fn refused(answer: ApiResult<ValidatedAsk>) -> Vec<RefusalReason> {
        match answer {
            Err(ApiError::PlanRefused { refusals }) => {
                refusals.into_iter().map(|r| r.reason).collect()
            }
            Err(other) => panic!("expected refusals, got {other:?}"),
            Ok(_) => panic!("expected refusals, got a validated ask"),
        }
    }

    /// **A paging refusal is returned with the ask's refusals, never instead of them**, and alone
    /// when the ask validates. An unloadable shape and `limit` 0, and a limit over the ceiling: both
    /// refusals, the ask's first. Bites: `validate` returning the ask's refusals alone (the paging
    /// one drops), or the request's alone (`shape_not_askable` drops).
    #[test]
    fn a_paging_refusal_travels_with_the_asks_refusals() {
        for limit in [0, MAX_ARTIFACT_PAGE_ROWS + 1] {
            assert_eq!(
                refused(validate(&ShapeContext::empty(), &request(Some(limit)))),
                [
                    RefusalReason::ShapeNotAskable,
                    RefusalReason::PageLimitOutOfRange
                ],
                "{limit}"
            );
        }
        assert_eq!(
            refused(validate(&ShapeContext::empty(), &request(None))),
            [RefusalReason::ShapeNotAskable]
        );
    }
}
