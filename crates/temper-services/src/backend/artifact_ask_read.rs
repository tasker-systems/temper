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
//!    (`within_budget`), since the SQL's charge is an estimate of them, and a row whose content
//!    nests deeper than `ARTIFACT_ASK_CONTENT_DEPTH` is named in `oversized`, as a row too long
//!    for the page is, so every page parses in every first-party client.
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
use temper_core::types::data_artifact::{ArtifactCursor, ArtifactView, ARTIFACT_ASK_CONTENT_DEPTH};
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

/// Steps 3 and 4: the page, held to its byte budget as written and to the content depth every
/// first-party client parses, and on the first page the population, both from the validated bind.
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
/// is over `budget` or its content nests deeper than `ARTIFACT_ASK_CONTENT_DEPTH`, an oversized
/// entry carrying its stored `content_bytes`.
fn written(row: Examined, budget: usize) -> ApiResult<Written> {
    match row {
        Examined::Served(artifact) => {
            let view = artifact_view(artifact);
            let bytes = written_len(&view)?;
            let too_deep = view
                .content
                .as_ref()
                .is_some_and(|content| depth(content) > ARTIFACT_ASK_CONTENT_DEPTH);
            if bytes > budget || too_deep {
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

/// A served row's length as the page writes it: its compact JSON, counted by a sink as it is
/// written rather than allocated, and the comma that separates it from the next row.
fn written_len(view: &ArtifactView) -> ApiResult<usize> {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, view).map_err(|e| read_fault(e.into()))?;
    Ok(count.0 + 1)
}

/// How deeply `value` nests: each array or object level counts one, with the root at 1, so a
/// string or a number is 0 deep (as `ARTIFACT_ASK_CONTENT_DEPTH` counts it). Walked from an
/// explicit stack, not by recursion, so no value is too deep for the walk; only arrays and objects
/// are pushed, so a long array of numbers adds nothing to the stack.
fn depth(value: &serde_json::Value) -> usize {
    let nests = |child: &&serde_json::Value| child.is_array() || child.is_object();
    let mut deepest = 0;
    let mut stack = vec![(value, 1)];
    while let Some((value, level)) = stack.pop() {
        match value {
            serde_json::Value::Array(items) => {
                deepest = deepest.max(level);
                stack.extend(items.iter().filter(nests).map(|child| (child, level + 1)));
            }
            serde_json::Value::Object(map) => {
                deepest = deepest.max(level);
                stack.extend(map.values().filter(nests).map(|child| (child, level + 1)));
            }
            _ => {}
        }
    }
    deepest
}

/// **The page held to `budget` on the bytes its rows write.** The SQL charges a served row its
/// `content_bytes` and metadata, but storage can print a number longer than it was committed
/// (`1e19`, committed as the five bytes `1e+19`, is served as twenty digits), so that charge is
/// an estimate. Here each served row is measured as the response writes it, compactly, and the
/// rows are walked in order against `budget`: a served row longer than `budget` alone, or whose
/// content nests deeper than `ARTIFACT_ASK_CONTENT_DEPTH`, becomes an oversized entry in its
/// place; a row whose cost fits alone but not in what is left ends the page, with it and every
/// row after it dropped and `next` the last row kept. Otherwise the SQL's rows and `next` stand.
/// Every row a page can examine first fits `budget` alone — a served row as measured, or an
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

    /// An array nested `levels` deep, as [`depth`] counts it; `levels` is at least 1.
    fn nested(levels: usize) -> serde_json::Value {
        let mut value = serde_json::Value::Array(vec![]);
        for _ in 1..levels {
            value = serde_json::Value::Array(vec![value]);
        }
        value
    }

    /// **Depth counts each array or object level, with the root at 1**, as
    /// `ARTIFACT_ASK_CONTENT_DEPTH` names it: a string or a number is 0, `{}` and `[]` are 1,
    /// `{"a":[0]}` is 2, and a value is as deep as its deepest branch, not its widest. Bites: the
    /// root pushed at level 0 (every figure one less); `deepest.max(level)` dropped from the
    /// object arm (the objects' figures fall).
    #[test]
    fn depth_counts_each_array_or_object_level_with_the_root_at_1() {
        use serde_json::json;
        assert_eq!(depth(&json!("text")), 0);
        assert_eq!(depth(&json!(1)), 0);
        assert_eq!(depth(&json!({})), 1);
        assert_eq!(depth(&json!([])), 1);
        assert_eq!(depth(&json!({ "a": [0] })), 2);
        assert_eq!(depth(&json!([[[0]], [0, 1, 2], { "b": { "c": {} } }])), 4);
        assert_eq!(depth(&nested(200)), 200);
    }

    /// **`ARTIFACT_ASK_CONTENT_DEPTH` is the strictest first-party client's nesting limit less the
    /// levels above the content, measured on an ask page.** The generated Ruby client parses a
    /// body inside one more array (`JSON.parse("[#{body}]")`, `clients/temper-rb`) under Ruby's
    /// default `max_nesting` of 100. A page whose content nests exactly at the limit is exactly 100
    /// deep inside that array, and one level more is 101. Bite: the constant raised to 97.
    #[test]
    fn the_content_depth_limit_is_what_the_strictest_client_parses_on_an_ask_page() {
        const RUBY_MAX_NESTING: usize = 100;
        let as_ruby_parses = |levels: usize| {
            let mut view = artifact_view(served_row(1, "a"));
            view.content = Some(nested(levels));
            let page = AskResponse {
                shape: AskedShape {
                    shape_id: ShapeId::from(Uuid::max()),
                    shape_version: 1,
                    home_anchor_table: "kb_contexts".to_string(),
                    home_anchor_id: Uuid::max(),
                },
                artifacts: vec![view],
                oversized: vec![],
                population: None,
                ordered_by: AskOrder::Created,
                next: None,
                amended_since: None,
            };
            let body = serde_json::to_value(&page).expect("a page serializes");
            depth(&serde_json::Value::Array(vec![body]))
        };
        assert_eq!(as_ruby_parses(ARTIFACT_ASK_CONTENT_DEPTH), RUBY_MAX_NESTING);
        assert_eq!(
            as_ruby_parses(ARTIFACT_ASK_CONTENT_DEPTH + 1),
            RUBY_MAX_NESTING + 1
        );
    }

    /// **A served row whose content nests deeper than `ARTIFACT_ASK_CONTENT_DEPTH` is named in
    /// `oversized` in its place, though it fits the budget**, and the page goes on: rows 1 to 3,
    /// row 1 nested at the limit and row 2 one level past it. Row 1 and row 3 are served, row 2 is
    /// named, and the SQL's `next` stands. Bite: in `written`, `|| too_deep` removed (row 2 is
    /// served).
    #[test]
    fn a_row_nested_past_the_content_depth_is_named_oversized_in_its_place() {
        let mut at_limit = served_row(1, "a");
        at_limit.content = Some(nested(ARTIFACT_ASK_CONTENT_DEPTH));
        let mut past = served_row(2, "b");
        past.content = Some(nested(ARTIFACT_ASK_CONTENT_DEPTH + 1));
        let page = readback::RetrievedAskPage {
            artifacts: vec![at_limit, past, served_row(3, "c")],
            oversized: vec![],
            next: at(9),
        };
        let kept = within_budget(page, ARTIFACT_ASK_PAGE_BYTES).expect("a page");
        assert_eq!(ids(&kept), (vec![1, 3], vec![2]));
        assert_eq!(
            kept.oversized[0].content_bytes, 1,
            "the stored content_bytes"
        );
        assert_eq!(kept.next, at(9));
    }
}
