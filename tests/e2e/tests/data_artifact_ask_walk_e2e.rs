#![cfg(feature = "test-db")]

//! E2E: an adversarial walk of the batch artifact ask, which no single writer can stall (spec
//! `temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md` §7).
//!
//! A page that fails for every reader of a home — one a client cannot parse, one past the
//! platform's response cap, one whose read faults — stops every walk of the ask at the artifact
//! that caused it, because no `limit` or `next` steps past a page nobody receives. The walk's
//! structure guarantees progress (every matching row is served or named in `oversized`, and `next`
//! moves on) only while its test of what a page can carry is right, and that test is a list of the
//! causes found so far: a row too long for a page, a number storage prints longer than it was
//! committed, content nested past a client's parse limit. So this test names no cause. It commits
//! a hostile corpus through the real commit door, walks four asks of it to `next: null` (`tag`
//! present and no predicates at four limits: no `limit`, 1, 7 and `MAX_ARTIFACT_PAGE_ROWS`; a
//! `contains` and a `compare` over `tag` at two: no `limit` and 1), and asserts on every page what
//! a stall breaks:
//!
//! 1. **it parses**, through `DataArtifactsClient::ask` (serde_json at its default limits, as the
//!    MCP relay and the CLI parse) and from the raw body's bytes as an `AskResponse`, and it nests
//!    within the generated Ruby client's limit, a stated constant because Ruby cannot run here;
//! 2. **it fits** `VERCEL_RESPONSE_BODY_CAP_BYTES` on both doors that answer it: the API's body,
//!    and the MCP tool's answer (`temper_mcp::tools::data_artifacts::ask_result`, serialized, with
//!    the JSON-RPC envelope's allowance); and the body is within the bound that answer is sized
//!    for (`ARTIFACT_ASK_PAGE_BYTES` + `ARTIFACT_PAGE_ENVELOPE_BYTES`);
//! 3. **the walk completes**: `next: null` within one page per row of the family and one more, each
//!    `next` past the last, and every matching artifact exactly once, served or named in
//!    `oversized` as the corpus expects it, and nothing else;
//! 4. **served content is whole**: what was committed, value for value, so a number storage
//!    reprints (a float from 1e16 up to the 64-bit integer limits, served as an integer) compares
//!    by value, not by text;
//! 5. **it answers in time**: each read of a page within [`PAGE_TIMEOUT`], and the walks within
//!    [`WALK_BUDGET`]. Once a page has timed out or the budget is spent, no further walk or page
//!    starts, and each one skipped is a finding, so a slow run reports what it found rather than
//!    being killed by the test runner with every finding lost.
//!
//! The read budget is a constant the test cannot shrink, so the corpus opens with three artifacts
//! whose read charge spends it, and every walk that may examine three rows a page must stop its
//! first page on them, at the read frontier. The probes the corpus offers the doors (U+0000, the
//! deepest content, the longest family name) are each recorded with their answer. The U+0000
//! probes must be refused by name, and the deepest probe's refusals must not be faults (a 5xx);
//! the long family's answers are recorded, not held. Every failure is collected, naming the walk,
//! the page and the item, and all of them are reported together, with every probe's answer.
//!
//! **Cost.** About ninety-five artifacts holding about 18 MB of content, sixteen walks and several
//! hundred requests, each page asked twice (raw, and through the client). It runs for 34–41
//! seconds locally and was measured at 77–140 seconds in CI, heavier than this suite's usual test,
//! so `.config/nextest.toml` gives this binary a measured timeout override; [`WALK_BUDGET`] bounds
//! it inside that. The suite gates its
//! tests by what they need (`test-db`, `test-embed`), not by their weight, so this runs under
//! `test-db` with the rest.

mod common;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use temper_core::error::DATA_ARTIFACT_REFUSAL_CODE;
use temper_core::types::artifact_ask::{reference, ArtifactPredicate, AskResponse};
use temper_core::types::data_artifact::{
    ArtifactCommitRequest, ArtifactCursor, ARTIFACT_ASK_CONTENT_DEPTH, MAX_ARTIFACT_KIND_BYTES,
    MAX_ARTIFACT_PAGE_ROWS,
};
use temper_core::types::data_artifact_shape::{EnforcementMode, ShapeDeclareRequest};
use temper_core::types::ids::DataArtifactId;
use temper_services::transport::{
    ARTIFACT_ASK_PAGE_BYTES, ARTIFACT_ASK_READ_BYTES, ARTIFACT_PAGE_ENVELOPE_BYTES,
    ARTIFACT_READ_CANDIDATE_BYTES, ARTIFACT_ROW_FIXED_BYTES, VERCEL_RESPONSE_BODY_CAP_BYTES,
};
use temper_workflow::operations::{Surface, SURFACE_HEADER};
use temper_workflow::types::resource::ResourceCreateRequest;
use uuid::Uuid;

/// The nesting limit of the strictest first-party client, the generated Ruby SDK: Ruby's default
/// `max_nesting`, which `clients/temper-rb` does not override, counting the `[ ]` it parses a body
/// inside (see `ARTIFACT_ASK_CONTENT_DEPTH`). Ruby's parser cannot run in a Rust test, so a page is
/// held to it here as a stated constant.
const RUBY_MAX_NESTING: usize = 100;

/// The deepest content a Rust client reads on an ask page: serde_json's 127 levels, less the three
/// an ask page puts above each row's content (see `ARTIFACT_ASK_CONTENT_DEPTH`). The deepest
/// content the commit door admits must be past it, or no item here breaks that parse.
const RUST_CLIENT_CONTENT_DEPTH: usize = 124;

/// How long each read of a page may take: the raw request, and the client's. A request takes tens
/// of milliseconds locally on average; this allows a page hundreds of times that.
const PAGE_TIMEOUT: Duration = Duration::from_secs(15);

/// How long the test body may run, counted from the body's start, building the world included.
/// The test runner kills this binary at 300 seconds (its override in `.config/nextest.toml`), and
/// every collected finding is lost with it. So once this is spent no further walk or page starts,
/// and each one skipped is a finding. One page already in flight can still take [`PAGE_TIMEOUT`]
/// twice (raw, and through the client), so the body ends by 270 seconds, inside the kill, however
/// much of the budget building the world takes, up to the whole of it. Everything before the body
/// (process start, the harness creating the test database and applying the migrations) shares the
/// 30 seconds the body leaves.
const WALK_BUDGET: Duration = Duration::from_secs(240);

/// The page limits the `tag`-present ask, the ask with no predicates and the long family are
/// walked at: absent (the drain default), 1, a small limit that splits the corpus unevenly, and
/// the ceiling.
const LIMITS: [Option<u32>; 4] = [None, Some(1), Some(7), Some(MAX_ARTIFACT_PAGE_ROWS)];

/// The page limits the `contains` and `compare` asks are walked at: absent, whose first page stops
/// on the read frontier, and 1, which puts a page boundary after every row. Each walk reads the
/// whole corpus whatever its limit, and these asks match only a few small rows, so further limits
/// would cost whole reads of the corpus to split those few rows differently.
const MATCHER_LIMITS: [Option<u32>; 2] = [None, Some(1)];

/// The hostile family's bare name.
const KIND: &str = "hostile";

/// Each read-spending artifact's text: over a third of the read budget, so the third one read
/// spends it, and longer than a page, so each is named oversized and its content never sent.
const READ_SPENDING_CHARS: usize = ARTIFACT_ASK_READ_BYTES / 3 + 50_000;

/// What one copy of `1e19` costs an array as storage serves it, with its comma: twenty digits,
/// though it is committed as the five bytes `1e+19`.
const REPRINTED_COPY_BYTES: usize = "10000000000000000000,".len();

/// The copies of `1e19` in the artifact whose charge fits a page and whose served form does not:
/// charged six bytes a copy and served at [`REPRINTED_COPY_BYTES`], which puts it past the
/// platform's response cap as well as past the page.
const PAST_BUDGET_COPIES: usize = 300_000;

/// The copies of `1e19` in each of two artifacts that fit a page alone as served, but not together,
/// although their charges fit together.
const PAIR_COPIES: usize = 70_000;

/// Each straddling artifact's text: under half a page, so two fit one page and a third does not fit
/// what they leave.
const STRADDLE_CHARS: usize = ARTIFACT_ASK_PAGE_BYTES * 45 / 100;

/// The `"` and `\` pairs in the artifact the MCP relay escapes most. Each character is written as
/// two bytes, and the relay, escaping the page into one JSON string, writes each as four, so its
/// answer for this artifact is about twice the page's body: within the cap once, past it twice.
const QUOTE_PAIRS: usize = 450_000;

/// The zeros in the artifact the MCP relay's answer would grow most if it were pretty-printed: each
/// is two bytes written compactly, and pretty-printed it gains a line break and its indentation.
const ZEROS: usize = 700_000;

/// The run of small artifacts that carry the asked path.
const TINY_ROWS: usize = 30;

/// The label prefix of the small artifacts whose `tag` is a value at an extreme.
const EXTREME_TAG: &str = "extreme-tag-";

/// The run of artifacts committed with no content.
const CONTENTLESS_ROWS: usize = 20;

/// The deepest content offered to the commit door, past the 127 levels serde_json reads, so the
/// door's own limit is found from above.
const DEEPEST_PROBED: usize = 130;

/// The family-name lengths offered to the doors, longest first; the first one both the shape door
/// and the commit door admit is the long family. Each name is one control character repeated, so
/// it is written six bytes a character, and compresses as far as storage can compress a name. Both
/// doors refuse a name past `MAX_ARTIFACT_KIND_BYTES`, so the longest they admit is that length.
const LONG_FAMILY_CHARS: [usize; 4] = [
    1_000_000,
    300_000,
    MAX_ARTIFACT_KIND_BYTES + 1,
    MAX_ARTIFACT_KIND_BYTES,
];

/// Where the corpus expects an artifact to land on a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Served whole: it fits a page as written and nests within `ARTIFACT_ASK_CONTENT_DEPTH`.
    Served,
    /// Named in `oversized`: too long for any page as written, or nested deeper than a page holds.
    Oversized,
    /// Either; only where the corpus cannot know in advance (a family name the doors decide).
    Either,
}

/// One committed artifact: what was committed, where a page should put it, and which asks match.
#[derive(Debug)]
struct Item {
    id: Uuid,
    label: String,
    /// The committed content; `Value::Null` for an artifact committed with none.
    content: Value,
    expect: Expect,
    /// Matched by an ask with no predicates: live, and of an intent an ask admits by default.
    whole: bool,
    /// Matched by an ask for `tag` present: as `whole`, and the content an object holding the key
    /// `tag` (any value, JSON `null` included; `_temper_artifact_get`, migration
    /// `20261103000000`, steps object keys only).
    tagged: bool,
}

/// A family's committed artifacts, in commit order, folded ones included.
#[derive(Debug, Default)]
struct Family {
    items: Vec<Item>,
    index: HashMap<Uuid, usize>,
}

impl Family {
    fn get(&self, id: Uuid) -> Option<&Item> {
        self.index.get(&id).map(|&i| &self.items[i])
    }

    /// Every row committed to the family: an upper bound on the candidates a walk can read.
    fn rows(&self) -> usize {
        self.items.len()
    }

    /// Commit `entry` as `kind`, or fail the test: every item but the probes is part of the
    /// corpus by construction.
    async fn put(&mut self, app: &common::E2eTestApp, kind: &str, entry: Entry) -> CommittedRow {
        let label = entry.label.clone();
        self.try_put(app, kind, entry)
            .await
            .unwrap_or_else(|refused| panic!("the corpus item `{label}` was refused: {refused}"))
    }

    /// Commit `entry` as `kind` and record it; anything it supersedes is folded, so no ask
    /// matches it. A refused commit records nothing.
    async fn try_put(
        &mut self,
        app: &common::E2eTestApp,
        kind: &str,
        entry: Entry,
    ) -> Result<CommittedRow, Refused> {
        let row = commit(app, kind, &entry).await?;
        for folded in &entry.supersedes {
            if let Some(&i) = self.index.get(folded) {
                self.items[i].whole = false;
                self.items[i].tagged = false;
            }
        }
        let whole = entry.intent != "pinned";
        let tagged = whole
            && entry
                .content
                .as_object()
                .is_some_and(|o| o.contains_key("tag"));
        self.index.insert(row.artifact_id, self.items.len());
        self.items.push(Item {
            id: row.artifact_id,
            label: entry.label,
            content: entry.content,
            expect: entry.expect,
            whole,
            tagged,
        });
        Ok(row)
    }
}

/// One artifact to commit.
struct Entry {
    label: String,
    resource: Uuid,
    intent: &'static str,
    precedence: f64,
    content: Value,
    supersedes: Vec<Uuid>,
    expect: Expect,
}

impl Entry {
    /// A `member` artifact at precedence 0, superseding nothing.
    fn member(label: impl Into<String>, resource: Uuid, content: Value, expect: Expect) -> Entry {
        Entry {
            label: label.into(),
            resource,
            intent: "member",
            precedence: 0.0,
            content,
            supersedes: Vec::new(),
            expect,
        }
    }
}

/// A door's refusal: its status and its body.
#[derive(Debug)]
struct Refused {
    status: u16,
    body: Vec<u8>,
}

impl Refused {
    /// The error body's `code`, when the body is temper's error envelope.
    fn code(&self) -> Option<String> {
        let body: Value = serde_json::from_slice(&self.body).ok()?;
        body.get("error")?.get("code")?.as_str().map(str::to_string)
    }

    /// A refusal the door answered for the caller's request (a 4xx), not a fault (a 5xx).
    fn is_the_callers(&self) -> bool {
        (400..500).contains(&self.status)
    }
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, excerpt(&self.body))
    }
}

/// What each probe offered to a door was answered, in order, and the findings among them.
#[derive(Debug, Default)]
struct Probes {
    /// Every probe and its answer, for the failure message.
    answers: Vec<String>,
    findings: Vec<String>,
}

impl Probes {
    /// Record what `probe` was answered.
    fn record<T>(&mut self, probe: &str, answer: &Result<T, Refused>) {
        self.answers.push(match answer {
            Ok(_) => format!("{probe}: admitted"),
            Err(refused) => format!("{probe}: refused {refused}"),
        });
    }

    /// Record what `probe` was answered, and a finding when it was refused by a fault: a door
    /// refuses what it will not take with a 4xx, so a 5xx is the server failing, not refusing.
    fn record_refusal<T>(&mut self, probe: &str, answer: &Result<T, Refused>) {
        self.record(probe, answer);
        if let Err(refused) = answer {
            if !refused.is_the_callers() {
                self.findings.push(format!(
                    "{probe}: answered {refused}, a fault, not a refusal"
                ));
            }
        }
    }
}

/// The commit door's answer, read for the row's id, position and stored size only.
#[derive(Debug, Deserialize)]
struct Committed {
    artifact: CommittedRow,
}

#[derive(Debug, Deserialize)]
struct CommittedRow {
    artifact_id: Uuid,
    created: DateTime<Utc>,
    content_bytes: i64,
}

impl CommittedRow {
    fn position(&self) -> ArtifactCursor {
        ArtifactCursor {
            created: self.created,
            artifact_id: DataArtifactId::from(self.artifact_id),
        }
    }

    fn content_bytes(&self) -> usize {
        usize::try_from(self.content_bytes).expect("a stored size is not negative")
    }
}

/// POST `body` to `path` as the app's principal, through the real listener, as the CLI's surface:
/// the status, and the body's bytes unread.
async fn post(
    app: &common::E2eTestApp,
    path: &str,
    body: &impl serde::Serialize,
) -> (u16, Vec<u8>) {
    let resp = app
        .reqwest_client
        .post(app.url(path))
        .bearer_auth(&app.token)
        .header(SURFACE_HEADER, Surface::CliCloud.marker())
        .json(body)
        .send()
        .await
        .expect("the request reaches the server");
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.expect("the body is read");
    (status, bytes.to_vec())
}

/// The first bytes of a body, for a message.
fn excerpt(body: &[u8]) -> String {
    String::from_utf8_lossy(&body[..body.len().min(512)]).into_owned()
}

/// Commit `entry` as `kind` through the real commit door (`POST /api/resources/{id}/artifacts`),
/// raw. The door answers with the content, which a client reading the answer whole cannot parse
/// once the content nests past its limit, so only the row's id, position and size are read, by a
/// reader that skips every other field without nesting into it (serde_json skips an unread field
/// iteratively). On a refusal, the status and the body.
async fn commit(
    app: &common::E2eTestApp,
    kind: &str,
    entry: &Entry,
) -> Result<CommittedRow, Refused> {
    let request = ArtifactCommitRequest {
        kind: kind.to_string(),
        kind_owner: None,
        intent: entry.intent.to_string(),
        precedence: entry.precedence,
        content: entry.content.clone(),
        supersedes: entry
            .supersedes
            .iter()
            .copied()
            .map(DataArtifactId::from)
            .collect(),
        act: Default::default(),
    };
    let path = format!("/api/resources/{}/artifacts", entry.resource);
    let (status, body) = post(app, &path, &request).await;
    if !(200..300).contains(&status) {
        return Err(Refused { status, body });
    }
    let committed: Committed = serde_json::from_slice(&body)
        .unwrap_or_else(|e| panic!("the commit door's answer names the row ({e})"));
    Ok(committed.artifact)
}

/// The shape door's answer, read for the shape's id only.
#[derive(Debug, Deserialize)]
struct Declared {
    shape_id: Uuid,
}

/// Declare an advisory shape for `kind` in the context through the real shape door
/// (`POST /api/contexts/{id}/shapes`), raw, declaring the asked path `tag` with no constraint, so
/// content of any form commits under it. On a refusal, the status and the body.
async fn declare(app: &common::E2eTestApp, context: Uuid, kind: &str) -> Result<Uuid, Refused> {
    let request = ShapeDeclareRequest {
        kind: kind.to_string(),
        kind_owner: None,
        schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": { "tag": {} }
        }),
        enforcement: EnforcementMode::Advisory,
        act: Default::default(),
    };
    let (status, body) = post(app, &format!("/api/contexts/{context}/shapes"), &request).await;
    if !(200..300).contains(&status) {
        return Err(Refused { status, body });
    }
    let declared: Declared = serde_json::from_slice(&body)
        .unwrap_or_else(|e| panic!("the shape door's answer names the shape ({e})"));
    Ok(declared.shape_id)
}

/// `{"tag": tag, "payload": payload}`: an object carrying the asked path.
fn with_tag(tag: &str, payload: Value) -> Value {
    json!({ "tag": tag, "payload": payload })
}

/// Content `depth` deep as `ARTIFACT_ASK_CONTENT_DEPTH` counts it: the tagged object at 1, then
/// arrays down to `depth`. `depth` is at least 2.
fn nested(depth: usize) -> Value {
    let mut payload = json!([]);
    for _ in 2..depth {
        payload = json!([payload]);
    }
    with_tag(&format!("depth-{depth}"), payload)
}

/// How deeply `value` nests: each array or object level counts one, with the root at 1, as
/// `ARTIFACT_ASK_CONTENT_DEPTH` counts it. Walked from an explicit stack.
fn nesting(value: &Value) -> usize {
    let mut deepest = 0;
    let mut stack = vec![(value, 1)];
    while let Some((value, level)) = stack.pop() {
        match value {
            Value::Array(items) => {
                deepest = deepest.max(level);
                stack.extend(items.iter().map(|child| (child, level + 1)));
            }
            Value::Object(map) => {
                deepest = deepest.max(level);
                stack.extend(map.values().map(|child| (child, level + 1)));
            }
            _ => {}
        }
    }
    deepest
}

/// Numbers storage reprints, and their neighbours: floats from 1e16 up to the 64-bit integer
/// limits (served as integers), either side of that band, integers at the 64-bit limits, the
/// float extremes, long mantissas, and negative zero.
fn numbers() -> Value {
    let two_63 = 2.0_f64.powi(63);
    let two_64 = 2.0_f64.powi(64);
    with_tag(
        "numbers",
        json!([
            1e16,
            1.5e17,
            1e19,
            two_63,
            two_64,
            -1e16,
            -two_63,
            1e15,
            1e20,
            -1e19,
            i64::MIN,
            i64::MAX,
            u64::MAX,
            9_007_199_254_740_993_u64,
            5e-324,
            f64::MIN_POSITIVE,
            f64::MAX,
            f64::MIN,
            f64::EPSILON,
            0.1 + 0.2,
            1.0 / 3.0,
            std::f64::consts::PI,
            -0.0,
            0.0,
            0,
            -1
        ]),
    )
}

/// Every C0 control character but NUL, with `"`, `\` and DEL, as text and as object keys.
fn c0() -> Value {
    let controls: String = (1..0x20_u32)
        .filter_map(char::from_u32)
        .chain(['"', '\\', '\u{7f}'])
        .collect();
    let keys: serde_json::Map<String, Value> = controls
        .chars()
        .map(|c| (c.to_string(), json!(c.to_string())))
        .collect();
    with_tag("c0", json!({ "text": controls, "keys": keys }))
}

/// A long run of C0 control characters, each written as a six-byte escape or a two-byte one.
fn c0_run() -> Value {
    let run: String = (1..0x20_u32)
        .filter_map(char::from_u32)
        .flat_map(|c| std::iter::repeat_n(c, 1_000))
        .collect();
    with_tag("c0-run", json!(run))
}

/// A long run of three-byte characters and one of four-byte characters.
fn multibyte_run() -> Value {
    with_tag(
        "multibyte-run",
        json!(["\u{20ac}".repeat(100_000), "\u{1F600}".repeat(50_000)]),
    )
}

/// An object of many keys.
fn wide_object() -> Value {
    let keys: serde_json::Map<String, Value> = (0..10_000)
        .map(|i| (format!("k{i:05}"), json!(i)))
        .collect();
    with_tag("wide-object", Value::Object(keys))
}

/// `copies` copies of `1e19`, committed as five bytes each and served as twenty.
fn reprinted(label: &str, copies: usize) -> Value {
    with_tag(label, json!(vec![1e19_f64; copies]))
}

/// The family the corpus commits under one name the doors admit, as long as they admit, and the
/// characters in its name.
struct LongFamily {
    shape: Uuid,
    chars: usize,
    family: Family,
}

/// The world: one context, two resources homed in it, the hostile family and the long family.
struct World {
    app: common::E2eTestApp,
    shape: Uuid,
    family: Family,
    /// The three artifacts that spend the read budget, in order, and the last one's position.
    trio: Vec<Uuid>,
    trio_end: ArtifactCursor,
    long: LongFamily,
    probes: Probes,
}

async fn world(pool: PgPool) -> World {
    let app = common::setup(pool).await;
    let context = app
        .client
        .contexts()
        .create("e2e-artifact-ask-walk", None)
        .await
        .expect("context create");
    let mut resources = Vec::new();
    for n in 1..=2 {
        let created = app
            .client
            .resources()
            .create(&ResourceCreateRequest {
                kb_context_id: context.id.into(),
                idempotency_key: None,
                doc_type: "research".to_string(),
                origin_uri: format!("test://e2e/artifact-ask-walk/{n}"),
                title: format!("artifact-ask walk subject {n}"),
                act: Default::default(),
            })
            .await
            .expect("resource create");
        let id: Uuid = created.id.into();
        resources.push(id);
    }
    let (one, two) = (resources[0], resources[1]);
    let shape = declare(&app, *context.id, KIND)
        .await
        .expect("the hostile family's shape is declared");
    let mut family = Family::default();

    // The read frontier, first in the family, so every walk meets it on its first page.
    let mut trio = Vec::new();
    let mut charges = Vec::new();
    let mut trio_end = None;
    for n in 1..=3 {
        let label = format!("read-spending-{n}");
        let content = with_tag(&label, json!("x".repeat(READ_SPENDING_CHARS)));
        let row = family
            .put(
                &app,
                KIND,
                Entry::member(label, one, content, Expect::Oversized),
            )
            .await;
        assert!(
            row.content_bytes() > ARTIFACT_ASK_PAGE_BYTES,
            "the fixture: each read-spending artifact is longer than a page"
        );
        trio.push(row.artifact_id);
        charges.push(row.content_bytes() + ARTIFACT_READ_CANDIDATE_BYTES);
        trio_end = Some(row.position());
    }
    assert!(
        charges[0] + charges[1] < ARTIFACT_ASK_READ_BYTES
            && charges.iter().sum::<usize>() >= ARTIFACT_ASK_READ_BYTES,
        "the fixture: the third read-spending artifact is read, and spends the read budget"
    );
    let after = family
        .put(
            &app,
            KIND,
            Entry::member(
                "after-the-read-budget",
                two,
                json!({ "tag": "small" }),
                Expect::Served,
            ),
        )
        .await;
    assert!(
        after.content_bytes() < ARTIFACT_ROW_FIXED_BYTES,
        "the fixture: the row after the read frontier is small, so a page budget could hold it"
    );

    // Numbers, text, width and depth.
    let shaped = [
        ("numbers", numbers()),
        ("c0", c0()),
        ("c0-run", c0_run()),
        (
            "line-separators",
            with_tag(
                "line-separators",
                json!({ "text": "\u{2028}\u{2029}", "\u{2028}": "\u{2029}" }),
            ),
        ),
        (
            "non-bmp",
            with_tag(
                "non-bmp",
                json!({ "text": "\u{1F600}\u{1D11E}\u{10FFFF}", "\u{1F600}": "\u{10000}" }),
            ),
        ),
        ("multibyte-run", multibyte_run()),
        ("wide-object", wide_object()),
        (
            "wide-array",
            with_tag("wide-array", json!((0..100_000).collect::<Vec<u32>>())),
        ),
        ("depth-at-limit", nested(ARTIFACT_ASK_CONTENT_DEPTH)),
    ];
    for (n, (label, content)) in shaped.into_iter().enumerate() {
        let entry = Entry::member(label, resources[n % 2], content, Expect::Served);
        family.put(&app, KIND, entry).await;
    }
    let one_past = Entry::member(
        "depth-one-past-the-limit",
        two,
        nested(ARTIFACT_ASK_CONTENT_DEPTH + 1),
        Expect::Oversized,
    );
    family.put(&app, KIND, one_past).await;

    // The MCP relay escapes a page into one JSON string: what it doubles, and what pretty-printing
    // would grow.
    let quotes = with_tag("quotes", json!("\"\\".repeat(QUOTE_PAIRS)));
    let relayed_once = serde_json::to_string(&serde_json::to_string(&quotes).expect("serializes"))
        .expect("serializes")
        .len();
    let row = family
        .put(
            &app,
            KIND,
            Entry::member("quotes", one, quotes, Expect::Served),
        )
        .await;
    assert!(
        row.content_bytes() + 4 * ARTIFACT_ROW_FIXED_BYTES <= ARTIFACT_ASK_PAGE_BYTES
            && relayed_once + ARTIFACT_PAGE_ENVELOPE_BYTES <= VERCEL_RESPONSE_BODY_CAP_BYTES
            && 2 * relayed_once > VERCEL_RESPONSE_BODY_CAP_BYTES,
        "the fixture: the quotes fit a page, and escaped as the relay escapes its text they fit \
         the platform's response cap once and not twice"
    );
    let zeros = with_tag("zeros", json!(vec![0; ZEROS]));
    let pretty = serde_json::to_string_pretty(&zeros).expect("serializes");
    let relayed_pretty = serde_json::to_string(&pretty).expect("serializes").len();
    let row = family
        .put(
            &app,
            KIND,
            Entry::member("zeros", two, zeros, Expect::Served),
        )
        .await;
    assert!(
        row.content_bytes() + 4 * ARTIFACT_ROW_FIXED_BYTES <= ARTIFACT_ASK_PAGE_BYTES
            && relayed_pretty > VERCEL_RESPONSE_BODY_CAP_BYTES,
        "the fixture: the zeros fit a page, and pretty-printed and escaped as the relay escapes \
         its text, they alone are past the platform's response cap"
    );

    // Size: what storage serves can outgrow what was charged.
    let past = family
        .put(
            &app,
            KIND,
            Entry::member(
                "served-past-the-page",
                one,
                reprinted("served-past-the-page", PAST_BUDGET_COPIES),
                Expect::Oversized,
            ),
        )
        .await;
    assert!(
        past.content_bytes() + 4 * ARTIFACT_ROW_FIXED_BYTES <= ARTIFACT_ASK_PAGE_BYTES
            && PAST_BUDGET_COPIES * REPRINTED_COPY_BYTES > VERCEL_RESPONSE_BODY_CAP_BYTES,
        "the fixture: charged within a page, and served past the platform's response cap"
    );
    for n in 1..=2 {
        let label = format!("reprinted-pair-{n}");
        let content = reprinted(&label, PAIR_COPIES);
        let row = family
            .put(
                &app,
                KIND,
                Entry::member(label, two, content, Expect::Served),
            )
            .await;
        assert!(
            2 * (row.content_bytes() + 4 * ARTIFACT_ROW_FIXED_BYTES) <= ARTIFACT_ASK_PAGE_BYTES
                && PAIR_COPIES * REPRINTED_COPY_BYTES + 4 * ARTIFACT_ROW_FIXED_BYTES
                    <= ARTIFACT_ASK_PAGE_BYTES
                && 2 * PAIR_COPIES * REPRINTED_COPY_BYTES > ARTIFACT_ASK_PAGE_BYTES,
            "the fixture: the pair's charges fit one page together, and as served each fits a \
             page alone and the two do not"
        );
    }
    for n in 1..=3 {
        let label = format!("straddle-{n}");
        let content = with_tag(&label, json!("y".repeat(STRADDLE_CHARS)));
        family
            .put(
                &app,
                KIND,
                Entry::member(label, one, content, Expect::Served),
            )
            .await;
    }

    // Many tiny rows, carrying the asked path in every JSON form, at extreme precedences.
    let tags = [
        json!("t"),
        json!(0),
        Value::Null,
        json!(false),
        json!({}),
        json!([]),
    ];
    let precedences = [0.0, f64::MAX, f64::MIN, -0.0, 1e-300];
    for n in 0..TINY_ROWS {
        let entry = Entry {
            label: format!("tiny-{n}"),
            resource: resources[n % 2],
            intent: "member",
            precedence: precedences[n % precedences.len()],
            content: json!({ "tag": tags[n % tags.len()].clone(), "n": n }),
            supersedes: Vec::new(),
            expect: Expect::Served,
        };
        family.put(&app, KIND, entry).await;
    }

    // Small rows whose `tag` is a value at an extreme: numbers past `bigint`, or that storage
    // prints longer than they were committed, negative zero, and an array as deep as a page holds
    // under the tag. `compare` (at least 0) admits the four numbers and `contains` none of the
    // five, so a matcher arm that faults on a value it reads, not only on a type, stops a walk.
    let mut deep_tag = json!([]);
    for _ in 2..ARTIFACT_ASK_CONTENT_DEPTH {
        deep_tag = json!([deep_tag]);
    }
    let extremes = [
        ("1e19", json!(1e19)),
        ("u64-max", json!(u64::MAX)),
        ("f64-max", json!(f64::MAX)),
        ("negative-zero", json!(-0.0)),
        ("deep-array", deep_tag),
    ];
    for (n, (label, tag)) in extremes.into_iter().enumerate() {
        let entry = Entry::member(
            format!("{EXTREME_TAG}{label}"),
            resources[n % 2],
            json!({ "tag": tag, "n": TINY_ROWS + n }),
            Expect::Served,
        );
        family.put(&app, KIND, entry).await;
    }

    // A run of artifacts with no content: matched by the ask with no predicates only.
    for n in 0..CONTENTLESS_ROWS {
        let entry = Entry::member(
            format!("contentless-{n}"),
            resources[n % 2],
            Value::Null,
            Expect::Served,
        );
        family.put(&app, KIND, entry).await;
    }

    // Artifacts the ask for `tag` must not match: no `tag` at the root, or no object root.
    let unmatched = [
        ("no-tag", json!({ "payload": 1 })),
        ("array-root", json!(["tag"])),
        ("string-root", json!("tag")),
        ("number-root", json!(7)),
        ("bool-root", json!(true)),
        ("empty-object", json!({})),
        ("other-case", json!({ "Tag": 1 })),
        ("nested-tag", json!({ "payload": { "tag": 1 } })),
    ];
    for (n, (label, content)) in unmatched.into_iter().enumerate() {
        let entry = Entry::member(label, resources[n % 2], content, Expect::Served);
        family.put(&app, KIND, entry).await;
    }

    // Artifacts no ask here matches: an intent no ask admits by default, and a folded row.
    let pinned = Entry {
        intent: "pinned",
        ..Entry::member("pinned", one, json!({ "tag": "pinned" }), Expect::Served)
    };
    family.put(&app, KIND, pinned).await;
    let folded = Entry {
        intent: "current",
        ..Entry::member("folded", two, json!({ "tag": "folded" }), Expect::Served)
    };
    let folded = family.put(&app, KIND, folded).await;
    let folds = Entry {
        intent: "current",
        supersedes: vec![folded.artifact_id],
        ..Entry::member("folds-it", two, json!({ "tag": "folds" }), Expect::Served)
    };
    family.put(&app, KIND, folds).await;

    // Probes: in the corpus if the commit door admits them, absent if it refuses them, and each
    // answer recorded. Postgres stores no U+0000, so the commit door refuses it by name, before
    // anything is stored: a 400 under the data-artifact refusal code, never a 500.
    let mut probes = Probes::default();
    let nul_in_text = with_tag("nul-in-text", json!("a\u{0}b"));
    let nul_in_key = json!({ "tag": "nul-in-key", "k\u{0}": 1 });
    let nul_in_kind = format!("{KIND}\u{0}");
    for (label, kind, content) in [
        ("nul-in-text", KIND, nul_in_text),
        ("nul-in-key", KIND, nul_in_key),
        (
            "nul-in-kind",
            nul_in_kind.as_str(),
            json!({ "tag": "nul-in-kind" }),
        ),
    ] {
        let answer = family
            .try_put(
                &app,
                kind,
                Entry::member(label, one, content, Expect::Served),
            )
            .await;
        probes.record(label, &answer);
        let refused_by_name = answer.as_ref().is_err_and(|refused| {
            refused.status == 400 && refused.code().as_deref() == Some(DATA_ARTIFACT_REFUSAL_CODE)
        });
        if !refused_by_name {
            probes.findings.push(format!(
                "{label}: not refused as a 400 under {DATA_ARTIFACT_REFUSAL_CODE}"
            ));
        }
    }
    // The deepest content the commit door admits, found from above; past the depth a Rust client
    // reads on an ask page, so it is the corpus's one item that breaks that parse when served.
    let mut deepest = None;
    for depth in (ARTIFACT_ASK_CONTENT_DEPTH + 2..=DEEPEST_PROBED).rev() {
        let entry = Entry::member(
            format!("deepest-admitted ({depth} deep)"),
            one,
            nested(depth),
            Expect::Oversized,
        );
        let answer = family.try_put(&app, KIND, entry).await;
        probes.record_refusal(&format!("content {depth} deep"), &answer);
        if answer.is_ok() {
            deepest = Some(depth);
            break;
        }
    }
    if deepest.is_none_or(|depth| depth <= RUST_CLIENT_CONTENT_DEPTH) {
        probes.findings.push(format!(
            "the deepest content the commit door admitted is {deepest:?} deep, not past the \
             {RUST_CLIENT_CONTENT_DEPTH} a Rust client reads on an ask page"
        ));
    }

    let long = long_family(&app, *context.id, two, &mut probes).await;
    World {
        app,
        shape,
        family,
        trio,
        trio_end: trio_end.expect("three read-spending artifacts"),
        long,
        probes,
    }
}

/// The longest name in [`LONG_FAMILY_CHARS`] the shape door and the commit door both admit, with
/// four small artifacts and one without content committed under it. Where a page puts its rows
/// follows from the name: a row writes it once, as a JSON string, beside small content and fixed
/// fields within [`ARTIFACT_ROW_FIXED_BYTES`]. Each door's answer to each length is recorded in
/// `probes`; a longer name's refusal is witnessed by name in `data_artifact_refusal_e2e`.
async fn long_family(
    app: &common::E2eTestApp,
    context: Uuid,
    resource: Uuid,
    probes: &mut Probes,
) -> LongFamily {
    for chars in LONG_FAMILY_CHARS {
        let kind = "\u{1}".repeat(chars);
        let declared = declare(app, context, &kind).await;
        probes.record(
            &format!("a shape for a {chars}-character family"),
            &declared,
        );
        let Ok(shape) = declared else {
            continue;
        };
        let written = serde_json::to_string(&kind)
            .expect("a name serializes")
            .len();
        let expect = if written + 4 * ARTIFACT_ROW_FIXED_BYTES <= ARTIFACT_ASK_PAGE_BYTES {
            Expect::Served
        } else if written > ARTIFACT_ASK_PAGE_BYTES {
            Expect::Oversized
        } else {
            Expect::Either
        };
        let mut family = Family::default();
        let first = Entry::member("long-family-0", resource, json!({ "tag": 0 }), expect);
        let committed = family.try_put(app, &kind, first).await;
        probes.record(
            &format!("a commit to a {chars}-character family"),
            &committed,
        );
        if committed.is_err() {
            continue;
        }
        for n in 1..4 {
            let entry = Entry::member(
                format!("long-family-{n}"),
                resource,
                json!({ "tag": n }),
                expect,
            );
            family.put(app, &kind, entry).await;
        }
        let contentless = Entry::member("long-family-contentless", resource, Value::Null, expect);
        family.put(app, &kind, contentless).await;
        return LongFamily {
            shape,
            chars,
            family,
        };
    }
    panic!("the doors admit no family name of any length in {LONG_FAMILY_CHARS:?}")
}

/// Findings, collected so one run reports every page and item that failed.
#[derive(Debug, Default)]
struct Findings(Vec<String>);

impl Findings {
    fn note(&mut self, finding: String) {
        self.0.push(finding);
    }

    /// Fail with every finding, the first hundred in full, and every probe's answer.
    fn assert_none(&self, probes: &[String]) {
        let shown: Vec<&str> = self.0.iter().take(100).map(String::as_str).collect();
        assert!(
            self.0.is_empty(),
            "{} finding(s) on the hostile corpus:\n{}\n\nthe probes were answered:\n{}",
            self.0.len(),
            shown.join("\n"),
            probes.join("\n")
        );
    }
}

/// Whether a walk or a page may still start: not once [`WALK_BUDGET`] is spent, and not once a
/// page has timed out, since a server that slow is likely to be as slow for the next.
#[derive(Debug)]
struct Clock {
    deadline: Instant,
    timed_out: bool,
}

impl Clock {
    /// A clock whose budget starts now.
    fn starting_now() -> Clock {
        Clock {
            deadline: Instant::now() + WALK_BUDGET,
            timed_out: false,
        }
    }

    /// Why nothing more may start, or `None` while it may.
    fn spent(&self) -> Option<&'static str> {
        if self.timed_out {
            Some("a page has timed out")
        } else if Instant::now() >= self.deadline {
            Some("the test's time budget is spent")
        } else {
            None
        }
    }
}

/// Which corpus items an ask matches.
type Matches<'a> = &'a dyn Fn(&Item) -> bool;

/// What an ask with `predicates` matches among the corpus items, by the reference evaluator
/// (`temper_core::types::artifact_ask::reference`, the oracle the ask's SQL is held to): a live
/// item of an intent an ask admits whose content satisfies every predicate. The evaluator orders
/// strings through the comparator it is given, bytewise here; the walks' `compare` bound is a
/// number, so the type guard keeps every string out of the comparison and the database's
/// collation never enters.
fn by_reference(predicates: &Value) -> impl Fn(&Item) -> bool {
    let predicates: Vec<ArtifactPredicate> =
        serde_json::from_value(predicates.clone()).expect("the walk's predicates parse");
    move |item: &Item| {
        item.whole && reference::matches(&predicates, &item.content, &|a: &str, b: &str| a.cmp(b))
    }
}

/// One walk: an ask, a limit, and the family whose artifacts it should see.
struct Walk<'a> {
    name: String,
    ask: &'a Value,
    limit: Option<u32>,
    family: &'a Family,
    matches: Matches<'a>,
}

/// What one page of a walk held: the ids it served and named, and where it said to continue.
#[derive(Debug)]
struct PageSeen {
    served: Vec<Uuid>,
    oversized: Vec<Uuid>,
    next: Option<ArtifactCursor>,
}

/// A position as the walk orders rows: `(created, artifact_id)`, uuids bytewise as in Postgres.
fn position(cursor: &ArtifactCursor) -> (DateTime<Utc>, Uuid) {
    (cursor.created, cursor.artifact_id.uuid())
}

/// The request for one page of `ask`.
fn request(ask: &Value, limit: Option<u32>, after: Option<ArtifactCursor>) -> Value {
    let mut body = json!({ "ask": ask });
    if let Some(limit) = limit {
        body["limit"] = json!(limit);
    }
    if let Some(after) = after {
        body["after"] = serde_json::to_value(after).expect("a position serializes");
    }
    body
}

/// Ask for one page twice, raw and through `DataArtifactsClient::ask`, and hold it to what every
/// page must be: answered within [`PAGE_TIMEOUT`], within the platform's response cap as the API
/// sends it and as the MCP tool answers it, and within the body that answer is sized for, parsed
/// from its bytes as an `AskResponse`, nested within the Ruby client's limit, and read alike by the
/// client. `None` when the walk cannot go on from it; a timeout also stops `clock`.
async fn read_page(
    app: &common::E2eTestApp,
    request: &Value,
    at: &str,
    clock: &mut Clock,
    findings: &mut Findings,
) -> Option<AskResponse> {
    let asked = post(app, "/api/data-artifacts/ask", request);
    let Ok((status, body)) = tokio::time::timeout(PAGE_TIMEOUT, asked).await else {
        findings.note(format!("{at}: no answer within {PAGE_TIMEOUT:?}"));
        clock.timed_out = true;
        return None;
    };
    if status != 200 {
        findings.note(format!("{at}: answered {status}: {}", excerpt(&body)));
        return None;
    }
    if body.len() > VERCEL_RESPONSE_BODY_CAP_BYTES {
        findings.note(format!(
            "{at}: the body is {} bytes, past the platform's response cap of \
             {VERCEL_RESPONSE_BODY_CAP_BYTES}",
            body.len()
        ));
    }
    let relay_sized = ARTIFACT_ASK_PAGE_BYTES + ARTIFACT_PAGE_ENVELOPE_BYTES;
    if body.len() > relay_sized {
        findings.note(format!(
            "{at}: the body is {} bytes, past the {relay_sized} the MCP relay's re-encoding is \
             sized for",
            body.len()
        ));
    }
    let raw: AskResponse = match serde_json::from_slice(&body) {
        Ok(page) => page,
        Err(e) => {
            findings.note(format!(
                "{at}: the body does not parse as an AskResponse: {e}"
            ));
            return None;
        }
    };
    match temper_mcp::tools::data_artifacts::ask_result(&raw) {
        Ok(answer) => {
            let relayed = serde_json::to_string(&answer)
                .expect("a tool answer serializes")
                .len();
            if relayed + ARTIFACT_PAGE_ENVELOPE_BYTES > VERCEL_RESPONSE_BODY_CAP_BYTES {
                findings.note(format!(
                    "{at}: the MCP tool's answer is {relayed} bytes, past the platform's response \
                     cap of {VERCEL_RESPONSE_BODY_CAP_BYTES} with the JSON-RPC envelope's \
                     {ARTIFACT_PAGE_ENVELOPE_BYTES}"
                ));
            }
        }
        Err(e) => findings.note(format!("{at}: the MCP tool cannot answer the page: {e:?}")),
    }
    let written = serde_json::to_value(&raw).expect("a page serializes");
    let as_ruby_parses = nesting(&written) + 1;
    if as_ruby_parses > RUBY_MAX_NESTING {
        findings.note(format!(
            "{at}: the page nests {as_ruby_parses} deep as the Ruby client parses it, past its \
             {RUBY_MAX_NESTING}"
        ));
    }
    let artifacts = app.client.data_artifacts();
    let read = artifacts.ask(request);
    let Ok(read) = tokio::time::timeout(PAGE_TIMEOUT, read).await else {
        findings.note(format!(
            "{at}: DataArtifactsClient::ask had no answer within {PAGE_TIMEOUT:?}"
        ));
        clock.timed_out = true;
        return None;
    };
    match read {
        Ok(page) if serde_json::to_value(&page).expect("a page serializes") == written => {}
        Ok(_) => findings.note(format!(
            "{at}: the client read a page other than the raw body"
        )),
        Err(e) => {
            findings.note(format!(
                "{at}: DataArtifactsClient::ask cannot read the page: {e}"
            ));
            return None;
        }
    }
    Some(raw)
}

/// Count `id` as seen on this walk, and return the corpus item when the walk's ask matches it; a
/// finding when it does not.
fn matched<'a>(
    at: &str,
    id: Uuid,
    walk: &Walk<'a>,
    seen: &mut HashMap<Uuid, usize>,
    findings: &mut Findings,
) -> Option<&'a Item> {
    *seen.entry(id).or_default() += 1;
    match walk.family.get(id) {
        Some(item) if (walk.matches)(item) => Some(item),
        Some(item) => {
            findings.note(format!(
                "{at}: `{}` appears, but the ask does not match it",
                item.label
            ));
            None
        }
        None => {
            findings.note(format!(
                "{at}: {id} appears, but the corpus did not commit it"
            ));
            None
        }
    }
}

/// Hold a page's rows to the corpus: each matched, served or named as expected, and served whole.
fn check_rows(
    at: &str,
    page: &AskResponse,
    walk: &Walk<'_>,
    seen: &mut HashMap<Uuid, usize>,
    findings: &mut Findings,
) {
    for row in &page.artifacts {
        let Some(item) = matched(at, row.artifact_id.uuid(), walk, seen, findings) else {
            continue;
        };
        if item.expect == Expect::Oversized {
            findings.note(format!(
                "{at}: `{}` is served, but no page can carry it",
                item.label
            ));
        }
        let whole = match (&item.content, &row.content) {
            (Value::Null, served) => served.is_none(),
            (committed, Some(served)) => same_value(served, committed),
            (_, None) => false,
        };
        if !whole {
            findings.note(format!(
                "{at}: `{}` is served with content other than was committed",
                item.label
            ));
        }
    }
    for entry in &page.oversized {
        let Some(item) = matched(at, entry.artifact_id.uuid(), walk, seen, findings) else {
            continue;
        };
        if item.expect == Expect::Served {
            findings.note(format!(
                "{at}: `{}` is named oversized, though a page can carry it whole",
                item.label
            ));
        }
    }
}

/// Whether `served` is `committed`, value for value: numbers by value, so a float storage reprints
/// as an integer is still the number committed; everything else exactly.
fn same_value(served: &Value, committed: &Value) -> bool {
    match (served, committed) {
        (Value::Number(s), Value::Number(c)) => {
            if let (Some(s), Some(c)) = (s.as_i64(), c.as_i64()) {
                s == c
            } else if let (Some(s), Some(c)) = (s.as_u64(), c.as_u64()) {
                s == c
            } else {
                s.as_f64() == c.as_f64()
            }
        }
        (Value::Array(served), Value::Array(committed)) => {
            served.len() == committed.len()
                && served.iter().zip(committed).all(|(s, c)| same_value(s, c))
        }
        (Value::Object(served), Value::Object(committed)) => {
            served.len() == committed.len()
                && committed
                    .iter()
                    .all(|(key, c)| served.get(key).is_some_and(|s| same_value(s, c)))
        }
        _ => served == committed,
    }
}

/// Walk `walk.ask` from its first page to `next: null`, holding every page to what a page must be
/// and every row to the corpus, then hold the walk to the corpus: every artifact the ask matches
/// exactly once. The pages seen, in order; `None`, and a finding, when `clock` lets the walk not
/// start. A page `clock` lets not start is a finding too, and stops the walk.
async fn walk_to_the_end(
    app: &common::E2eTestApp,
    walk: &Walk<'_>,
    clock: &mut Clock,
    findings: &mut Findings,
) -> Option<Vec<PageSeen>> {
    if let Some(reason) = clock.spent() {
        findings.note(format!("walk `{}`: not run: {reason}", walk.name));
        return None;
    }
    let max_pages = walk.family.rows() + 1;
    let mut seen: HashMap<Uuid, usize> = HashMap::new();
    let mut pages: Vec<PageSeen> = Vec::new();
    let mut after: Option<ArtifactCursor> = None;
    let ended = loop {
        let at = format!(
            "walk `{}`, page {} (after {:?})",
            walk.name,
            pages.len() + 1,
            after.map(|a| a.artifact_id.uuid())
        );
        if pages.len() == max_pages {
            findings.note(format!(
                "{at}: no `next: null` after {max_pages} pages, one per row of the family and one \
                 more"
            ));
            break false;
        }
        if let Some(reason) = clock.spent() {
            findings.note(format!("{at}: not read: {reason}"));
            break false;
        }
        let body = request(walk.ask, walk.limit, after);
        let Some(page) = read_page(app, &body, &at, clock, findings).await else {
            break false;
        };
        check_rows(&at, &page, walk, &mut seen, findings);
        pages.push(PageSeen {
            served: page
                .artifacts
                .iter()
                .map(|a| a.artifact_id.uuid())
                .collect(),
            oversized: page
                .oversized
                .iter()
                .map(|o| o.artifact_id.uuid())
                .collect(),
            next: page.next,
        });
        match page.next {
            None => break true,
            Some(next) if after.is_some_and(|after| position(&next) <= position(&after)) => {
                findings.note(format!("{at}: `next` {next:?} does not move past `after`"));
                break false;
            }
            Some(next) => after = Some(next),
        }
    };
    let expected: Vec<&Item> = walk
        .family
        .items
        .iter()
        .filter(|item| (walk.matches)(item))
        .collect();
    if ended {
        for item in expected {
            let times = seen.get(&item.id).copied().unwrap_or(0);
            if times != 1 {
                findings.note(format!(
                    "walk `{}`: `{}` appears {times} times, not once",
                    walk.name, item.label
                ));
            }
        }
    } else {
        let missed = expected
            .iter()
            .filter(|item| !seen.contains_key(&item.id))
            .count();
        findings.note(format!(
            "walk `{}`: stopped after {} page(s), with {missed} of {} matching artifacts never \
             reached",
            walk.name,
            pages.len(),
            expected.len()
        ));
    }
    Some(pages)
}

/// A limit as a walk's name shows it.
fn shown(limit: Option<u32>) -> String {
    limit.map_or_else(|| "absent".to_string(), |limit| limit.to_string())
}

/// **Every walk of a hostile corpus ends, and every page parses, fits and serves whole content.**
/// The hostile family is asked four ways — `tag` present, which its non-object, untagged,
/// content-less, pinned and folded artifacts must not answer; with no predicates, which every
/// live member and current artifact answers, content-less ones included; `tag` containing `"t"`,
/// `null` or `{}`, and `tag` at least 0, which only some of the small artifacts answer, their `tag`
/// taking every JSON type and some a value at an extreme (`1e19`, `u64::MAX`, `f64::MAX`, `-0.0`,
/// an array as deep as a page holds), so a matcher arm that could raise on a type or on a value
/// would show — and the long family with no predicates. The `contains` and `compare` asks are
/// walked at every limit in [`MATCHER_LIMITS`], the rest at every limit in [`LIMITS`], all within
/// [`WALK_BUDGET`]. What the `contains` and `compare` asks match is the reference evaluator's
/// answer ([`by_reference`]). A walk that may examine three rows a page (no limit, 7, the ceiling)
/// stops its first page on the read frontier: of the three read-spending artifacts, the ones its
/// ask matches named oversized, nothing served, and `next` at the third, though the row limit left
/// room and the page had spent at most three entries' worth of its budget on a page whose next
/// row is small. So only the read budget can have ended it. The probes' answers are held too, but
/// for the long family's, which are recorded only: U+0000 refused by name, the deepest probe's
/// refusals never a fault, and the deepest content admitted past what a Rust client reads on a
/// page.
///
/// Bites, each red through this test's own findings:
/// - the depth check removed (in `artifact_ask_read::written`, `if bytes > budget || too_deep`
///   becomes `if bytes > budget`): `depth-one-past-the-limit` is served, past the Ruby client's
///   limit, and the deepest admitted artifact makes a page no serde_json client parses, stopping
///   the walk;
/// - the written-size check given `usize::MAX` (in `artifact_ask_read::read`,
///   `within_budget(page, ARTIFACT_ASK_PAGE_BYTES)` becomes `within_budget(page, usize::MAX)`):
///   `served-past-the-page` is served, and its page is past the platform's response cap;
/// - `next` from `examined` alone (in temper-substrate's `readback::ask_page_of`,
///   `!r.examined || r.frontier` becomes `!r.examined`): the first page, stopped on the read
///   frontier, answers `next: null`, so the walk ends there and every later artifact is missed;
/// - the MCP tool's answer pretty-printed (in temper-mcp's `tools::data_artifacts::compact_result`,
///   which `ask_result` answers through, `serde_json::to_string(value)` becomes
///   `serde_json::to_string_pretty(value)`): every page serving `zeros` answers past the
///   platform's response cap through the tool;
/// - the commit door's U+0000 check removed (in `DbBackend::commit_data_artifact`, the
///   `unstorable_nul` check): the three U+0000 probes answer 500, "not refused as a 400".
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_walk_of_a_hostile_corpus_ends_and_every_page_parses_fits_and_is_whole(pool: PgPool) {
    // Counted from the body's start, so a slow world spends the budget and leaves fewer walks run,
    // each a named finding.
    let mut clock = Clock::starting_now();
    let w = world(pool).await;
    let mut findings = Findings::default();
    for finding in &w.probes.findings {
        findings.note(format!("probe {finding}"));
    }
    let ask_with = |predicates: &Value| json!({ "shape": w.shape, "predicates": predicates });
    let tag_ask = ask_with(&json!([{ "path": ["tag"], "op": { "op": "has_key" } }]));
    let whole_ask = json!({ "shape": w.shape });
    let contains = json!([
        { "path": ["tag"], "op": { "op": "contains", "values": ["t", null, {}] } }
    ]);
    let compare = json!([
        { "path": ["tag"], "op": { "op": "compare", "direction": "gte", "value": 0 } }
    ]);
    let (contains_ask, compare_ask) = (ask_with(&contains), ask_with(&compare));
    let tagged = |item: &Item| item.tagged;
    let whole = |item: &Item| item.whole;
    let contained = by_reference(&contains);
    let compared = by_reference(&compare);
    let extremes: Vec<&Item> = w
        .family
        .items
        .iter()
        .filter(|item| item.label.starts_with(EXTREME_TAG))
        .collect();
    assert!(
        extremes.len() == 5
            && extremes.iter().filter(|&&item| compared(item)).count() == 4
            && !extremes.iter().any(|&item| contained(item)),
        "the fixture: `compare` admits the four extreme numbers, and `contains` none of the five \
         extreme tags"
    );
    let asks: [(&str, &Value, Matches<'_>, &[Option<u32>]); 4] = [
        ("`tag` present", &tag_ask, &tagged, &LIMITS),
        ("no predicates", &whole_ask, &whole, &LIMITS),
        (
            "`tag` contains \"t\", null or {}",
            &contains_ask,
            &contained,
            &MATCHER_LIMITS,
        ),
        ("`tag` at least 0", &compare_ask, &compared, &MATCHER_LIMITS),
    ];
    for (ask_name, ask, matches, limits) in asks {
        for &limit in limits {
            let walk_of = Walk {
                name: format!("{ask_name}, limit {}", shown(limit)),
                ask,
                limit,
                family: &w.family,
                matches,
            };
            let walked = walk_to_the_end(&w.app, &walk_of, &mut clock, &mut findings).await;
            let Some(pages) = walked else {
                continue;
            };
            if limit.is_none_or(|limit| limit >= 3) {
                let named: Vec<Uuid> = w
                    .trio
                    .iter()
                    .copied()
                    .filter(|id| w.family.get(*id).is_some_and(matches))
                    .collect();
                let first = pages.first();
                let on_the_frontier = first.is_some_and(|page| {
                    page.served.is_empty()
                        && page.oversized == named
                        && page.next == Some(w.trio_end)
                });
                if !on_the_frontier {
                    findings.note(format!(
                        "walk `{}`: page 1 does not stop at the read frontier, the third \
                         read-spending artifact: {first:?}",
                        walk_of.name
                    ));
                }
            }
        }
    }
    let long_ask = json!({ "shape": w.long.shape });
    for limit in LIMITS {
        let walk_of = Walk {
            name: format!(
                "a family named by {} characters, limit {}",
                w.long.chars,
                shown(limit)
            ),
            ask: &long_ask,
            limit,
            family: &w.long.family,
            matches: &whole,
        };
        walk_to_the_end(&w.app, &walk_of, &mut clock, &mut findings).await;
    }
    findings.assert_none(&w.probes.answers);
}
