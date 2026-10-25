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
//!    `artifact_ask_counts`;
//! 4. the page's rows are held to `ARTIFACT_ASK_PAGE_BYTES` on the bytes they write
//!    (`within_budget`), since the SQL's charge is an estimate of them.
//!
//! The continuation carries position and never authority: readability is re-decided at step 1 of
//! every page, so a page after access is revoked answers as an absent shape does. Before any of
//! it, a position the store cannot hold is refused ([`refuse_unstorable_position`]).

use axum::http::StatusCode;
use sqlx::PgPool;

use crate::backend::substrate_read::artifact_view;
use crate::error::{ApiError, ApiResult};
use crate::services::shape_loader;
use crate::transport::{
    ARTIFACT_ASK_PAGE_BYTES, ARTIFACT_ASK_READ_BYTES, ARTIFACT_READ_CANDIDATE_BYTES,
    ARTIFACT_ROW_FIXED_BYTES,
};
use temper_core::error::TemperError;
use temper_core::types::artifact_ask::{
    AskOrder, AskRequest, AskResponse, AskSession, AskedShape, OversizedArtifact, ShapeContext,
    ShapeSuccessor, ValidatedAsk,
};
use temper_core::types::data_artifact::{ArtifactCursor, ArtifactView};
use temper_core::types::ids::ProfileId;
use temper_substrate::readback;

/// One page of the artifacts matching `request.ask`, for `principal`.
///
/// # Errors
/// `ApiError::UnreadablePlan` for an `after` the store cannot hold, before anything is read
/// ([`refuse_unstorable_position`]); `ApiError::PlanRefused` carrying every refusal — the ask's
/// and the request's — when any is raised; a database fault otherwise.
pub async fn ask_page(
    pool: &PgPool,
    principal: ProfileId,
    request: &AskRequest,
) -> ApiResult<AskResponse> {
    refuse_unstorable_position(request)?;
    let (shapes, amended_since) = load(pool, principal, request).await?;
    let validated = validate(&shapes, request)?;
    read(pool, principal, request, &validated, amended_since).await
}

/// What a caller is told when its `after` names an instant the store cannot hold. Fixed, so it
/// quotes nothing back.
const UNSTORABLE_POSITION: &str = "`after.created` is outside the instants an artifact can be \
    created at (4714-11-24 BC to 294276 AD, UTC), so it is not a position any page returned";

/// Refuse an `after` the store cannot hold (`ArtifactCursor::is_storable`) as a body that cannot be
/// read: `ApiError::UnreadablePlan`, status `422`, with a fixed message, which the API renders
/// under the code `UNREADABLE_PLAN`. The API door runs this as it reads the body, and
/// [`ask_page`] runs it again before it reads anything, so a caller of the service sees the same
/// refusal as an `Err`, never a database fault from binding the position.
///
/// # Errors
/// `ApiError::UnreadablePlan` when `request.after` is not storable.
pub fn refuse_unstorable_position(request: &AskRequest) -> ApiResult<()> {
    if request.after.is_some_and(|after| !after.is_storable()) {
        return Err(ApiError::UnreadablePlan {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: UNSTORABLE_POSITION.to_string(),
        });
    }
    Ok(())
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

/// Steps 3 and 4: the page, held to its byte budget as written, and on the first page the
/// population, both from the validated bind.
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
            page_budget: ARTIFACT_ASK_PAGE_BYTES,
            row_bytes: ARTIFACT_ROW_FIXED_BYTES,
            read_budget: ARTIFACT_ASK_READ_BYTES,
            read_charge: ARTIFACT_READ_CANDIDATE_BYTES,
        },
    )
    .await
    .map_err(read_fault)?;
    let page = within_budget(page, ARTIFACT_ASK_PAGE_BYTES)?;
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
        artifacts: page.artifacts,
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

/// One page's rows as the response writes them, and where the next page starts.
#[derive(Debug)]
struct WrittenPage {
    artifacts: Vec<ArtifactView>,
    oversized: Vec<OversizedArtifact>,
    next: Option<ArtifactCursor>,
}

/// A row the SQL examined, served or oversized, before it is written.
#[derive(Debug)]
enum Examined {
    Served(readback::RetrievedArtifact),
    Oversized(readback::RetrievedOversized),
}

impl Examined {
    fn position(&self) -> ArtifactCursor {
        match self {
            Examined::Served(a) => ArtifactCursor {
                created: a.created,
                artifact_id: a.artifact_id,
            },
            Examined::Oversized(o) => o.position(),
        }
    }
}

/// An examined row as the page writes it, with what it costs the page.
#[derive(Debug)]
enum Written {
    /// Served whole; it costs its compact JSON and the comma that separates it from the next row.
    /// Boxed: a whole row is far larger than an entry.
    Served(Box<ArtifactView>, usize),
    /// Named in `oversized`, at `ARTIFACT_ROW_FIXED_BYTES`, which bounds the entry with its comma.
    Oversized(OversizedArtifact),
}

impl Written {
    fn cost(&self) -> usize {
        match self {
            Written::Served(_, bytes) => *bytes,
            Written::Oversized(_) => ARTIFACT_ROW_FIXED_BYTES,
        }
    }
}

/// The page's examined rows in `(created, artifact_id)` order. The SQL returns them in that order
/// and the page splits them into two lists, each still in it; uuids order bytewise here as in
/// Postgres, so sorting their union restores the SQL's order.
fn in_order(page_rows: readback::RetrievedAskPage) -> Vec<Examined> {
    let mut rows: Vec<Examined> = page_rows
        .artifacts
        .into_iter()
        .map(Examined::Served)
        .chain(page_rows.oversized.into_iter().map(Examined::Oversized))
        .collect();
    rows.sort_by_key(|row| {
        let position = row.position();
        (position.created, position.artifact_id.uuid())
    });
    rows
}

/// `row` as the page writes it: a served row at its measured length, or, when that length alone
/// is over `budget`, an oversized entry carrying its stored `content_bytes`.
fn written(row: Examined, budget: usize) -> ApiResult<Written> {
    match row {
        Examined::Served(artifact) => {
            let view = artifact_view(artifact);
            let bytes = serde_json::to_vec(&view)
                .map_err(|e| read_fault(e.into()))?
                .len()
                + 1;
            if bytes > budget {
                Ok(Written::Oversized(OversizedArtifact {
                    artifact_id: view.artifact_id,
                    resource_id: view.resource_id,
                    content_bytes: view.content_bytes,
                }))
            } else {
                Ok(Written::Served(Box::new(view), bytes))
            }
        }
        Examined::Oversized(entry) => Ok(Written::Oversized(entry.entry())),
    }
}

/// **The page held to `budget` on the bytes its rows write.** The SQL charges a served row its
/// `content_bytes` and metadata, but storage can print a number longer than it was committed
/// (`1e19`, committed as the five bytes `1e+19`, is served as twenty digits), so that charge is an estimate. Here
/// each served row is measured as the response writes it, compactly, and the rows are walked in
/// order against `budget`: a served row longer than `budget` alone becomes an oversized entry in
/// its place; a row whose cost fits alone but not in what is left ends the page, with it and
/// every row after it dropped and `next` the last row kept. Otherwise the SQL's rows and `next`
/// stand. Every row a page can examine first fits `budget` alone — a served row as measured, or an
/// oversized entry at `ARTIFACT_ROW_FIXED_BYTES` — so a page that ends early has kept a row, and
/// `next` is never `None` while a row is dropped.
fn within_budget(page: readback::RetrievedAskPage, budget: usize) -> ApiResult<WrittenPage> {
    let sql_next = page.next;
    let mut artifacts = Vec::new();
    let mut oversized = Vec::new();
    let mut left = budget;
    let mut last = None;
    for row in in_order(page) {
        let position = row.position();
        let row = written(row, budget)?;
        let cost = row.cost();
        if cost > left {
            let next = last.ok_or_else(|| {
                read_fault(anyhow::anyhow!(
                    "a batch-read page's first row does not fit its budget alone"
                ))
            })?;
            return Ok(WrittenPage {
                artifacts,
                oversized,
                next: Some(next),
            });
        }
        left -= cost;
        last = Some(position);
        match row {
            Written::Served(view, _) => artifacts.push(*view),
            Written::Oversized(entry) => oversized.push(entry),
        }
    }
    Ok(WrittenPage {
        artifacts,
        oversized,
        next: sql_next,
    })
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

    /// A served row of the SQL's page, `n`th in order, holding `text`.
    fn served_row(n: u128, text: &str) -> readback::RetrievedArtifact {
        readback::RetrievedArtifact {
            artifact_id: temper_core::types::ids::DataArtifactId::from(Uuid::from_u128(n)),
            resource_id: temper_core::types::ids::ResourceId::from(Uuid::max()),
            kind_owner: temper_substrate::payloads::KindOwner::Profile(Uuid::max()),
            artifact_kind: "measurement".to_string(),
            intent: temper_substrate::payloads::ArtifactIntent::Member,
            precedence: 0.0,
            content_hash: "f".repeat(64),
            content_bytes: 1,
            shape_state: temper_substrate::payloads::ShapeState::DeclaredSatisfied,
            is_folded: false,
            created: chrono::DateTime::default(),
            content: Some(serde_json::Value::from(text)),
        }
    }

    /// An oversized entry of the SQL's page, `n`th in order.
    fn oversized_row(n: u128) -> readback::RetrievedOversized {
        readback::RetrievedOversized {
            artifact_id: temper_core::types::ids::DataArtifactId::from(Uuid::from_u128(n)),
            resource_id: temper_core::types::ids::ResourceId::from(Uuid::max()),
            content_bytes: 1,
            created: chrono::DateTime::default(),
        }
    }

    /// What a served row costs the page as written: its compact JSON and its comma.
    fn cost(row: &readback::RetrievedArtifact) -> usize {
        serde_json::to_vec(&artifact_view(row.clone()))
            .expect("a row serializes")
            .len()
            + 1
    }

    fn ids(page: &WrittenPage) -> (Vec<u128>, Vec<u128>) {
        (
            page.artifacts
                .iter()
                .map(|a| a.artifact_id.uuid().as_u128())
                .collect(),
            page.oversized
                .iter()
                .map(|o| o.artifact_id.uuid().as_u128())
                .collect(),
        )
    }

    fn at(n: u128) -> Option<ArtifactCursor> {
        Some(ArtifactCursor {
            created: chrono::DateTime::default(),
            artifact_id: temper_core::types::ids::DataArtifactId::from(Uuid::from_u128(n)),
        })
    }

    /// **A page is held to its budget on what its rows write, in order.** Rows 1 to 3, the SQL's
    /// `next` past them. (a) Row 2 is longer than the budget alone: it is named in `oversized` in
    /// its place, the rows around it served, and the SQL's `next` stands. (b) Row 3 fits alone but
    /// not after row 1: the page ends after row 1, `next` there. (c) The SQL named row 2 oversized,
    /// and its entry does not fit after row 1: the page ends after row 1 too. Bites: in
    /// `within_budget`, `if cost > left` → `if false` ((b) and (c) keep every row); in `written`,
    /// `if bytes > budget` → `if false` ((a) serves row 2).
    #[test]
    fn a_page_is_held_to_its_budget_on_what_its_rows_write() {
        let (one, three) = (served_row(1, "a"), served_row(3, "c"));
        let long = served_row(2, &"b".repeat(4_096));
        let (c1, c2, c3) = (cost(&one), cost(&long), cost(&three));
        let page = |artifacts: Vec<readback::RetrievedArtifact>,
                    oversized: Vec<readback::RetrievedOversized>| {
            readback::RetrievedAskPage {
                artifacts,
                oversized,
                next: at(9),
            }
        };

        let budget = c1 + ARTIFACT_ROW_FIXED_BYTES + c3;
        assert!(c2 > budget, "the fixture: row 2 is longer than the budget");
        let a = within_budget(page(vec![one.clone(), long, three.clone()], vec![]), budget)
            .expect("a page");
        assert_eq!(ids(&a), (vec![1, 3], vec![2]));
        assert_eq!(a.oversized[0].content_bytes, 1, "the stored content_bytes");
        assert_eq!(a.next, at(9));

        let b = within_budget(page(vec![one.clone(), three.clone()], vec![]), c1 + c3 - 1)
            .expect("a page");
        assert_eq!(ids(&b), (vec![1], vec![]));
        assert_eq!(b.next, at(1));

        let c = within_budget(
            page(vec![one, three], vec![oversized_row(2)]),
            c1 + ARTIFACT_ROW_FIXED_BYTES - 1,
        )
        .expect("a page");
        assert_eq!(ids(&c), (vec![1], vec![]));
        assert_eq!(c.next, at(1));
    }
}
