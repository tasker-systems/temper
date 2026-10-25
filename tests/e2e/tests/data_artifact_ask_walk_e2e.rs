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
//! a hostile corpus through the real commit door, walks the ask to `next: null` at four limits (no
//! `limit`, 1, 7 and `MAX_ARTIFACT_PAGE_ROWS`), and asserts on every page what a stall breaks:
//!
//! 1. **it parses**, through `DataArtifactsClient::ask` (serde_json at its default limits, as the
//!    MCP relay and the CLI parse) and from the raw body's bytes as an `AskResponse`, and it nests
//!    within the generated Ruby client's limit, a stated constant because Ruby cannot run here;
//! 2. **it fits** `VERCEL_RESPONSE_BODY_CAP_BYTES`, and the body bound the MCP relay's re-encoding
//!    is sized for (`ARTIFACT_ASK_PAGE_BYTES` + `ARTIFACT_PAGE_ENVELOPE_BYTES`);
//! 3. **the walk completes**: `next: null` within one page per row of the family and one more, each
//!    `next` past the last, and every matching artifact exactly once, served or named in
//!    `oversized` as the corpus expects it, and nothing else;
//! 4. **served content is whole**: what was committed, value for value, so a number storage
//!    reprints (a float from 1e16 up to the 64-bit integer limits, served as an integer) compares
//!    by value, not by text.
//!
//! The read budget is a constant the test cannot shrink, so the corpus opens with three artifacts
//! whose read charge spends it, and every walk that may examine three rows a page must stop its
//! first page on them, at the read frontier. Every failure is collected, naming the walk, the page
//! and the item, and all of them are reported together.
//!
//! **Cost.** About ninety artifacts holding about 15 MB of content, twelve walks and about 500
//! requests, each page asked twice (raw, and through the client). It is expected to run for tens
//! of seconds, heavier than this suite's usual test. The suite gates its tests by what they need
//! (`test-db`, `test-embed`), not by their weight, so this runs under `test-db` with the rest.

mod common;

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use temper_core::types::artifact_ask::AskResponse;
use temper_core::types::data_artifact::{
    ArtifactCommitRequest, ArtifactCursor, MAX_ARTIFACT_PAGE_ROWS,
};
use temper_core::types::data_artifact_shape::{EnforcementMode, ShapeDeclareRequest};
use temper_core::types::ids::DataArtifactId;
use temper_services::transport::{
    ARTIFACT_ASK_CONTENT_DEPTH, ARTIFACT_ASK_PAGE_BYTES, ARTIFACT_ASK_READ_BYTES,
    ARTIFACT_PAGE_ENVELOPE_BYTES, ARTIFACT_READ_CANDIDATE_BYTES, ARTIFACT_ROW_FIXED_BYTES,
    VERCEL_RESPONSE_BODY_CAP_BYTES,
};
use temper_workflow::operations::{Surface, SURFACE_HEADER};
use temper_workflow::types::resource::ResourceCreateRequest;
use uuid::Uuid;

/// The nesting limit of the strictest first-party client, the generated Ruby SDK: Ruby's default
/// `max_nesting`, which `clients/temper-rb` does not override, counting the `[ ]` it parses a body
/// inside (see `ARTIFACT_ASK_CONTENT_DEPTH` in temper-services). Ruby's parser cannot run in a Rust
/// test, so a page is held to it here as a stated constant.
const RUBY_MAX_NESTING: usize = 100;

/// The page limits each ask is walked at: absent (the drain default), 1, a small limit that splits
/// the corpus unevenly, and the ceiling.
const LIMITS: [Option<u32>; 4] = [None, Some(1), Some(7), Some(MAX_ARTIFACT_PAGE_ROWS)];

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

/// The run of small artifacts that carry the asked path.
const TINY_ROWS: usize = 30;

/// The run of artifacts committed with no content.
const CONTENTLESS_ROWS: usize = 20;

/// The deepest content offered to the commit door, past the 127 levels serde_json reads, so the
/// door's own limit is found from above.
const DEEPEST_PROBED: usize = 130;

/// The family-name lengths offered to the doors, longest first; the first one both the shape door
/// and the commit door admit is the long family. Each name is one control character repeated, so
/// it is written six bytes a character, and compresses as far as storage can compress a name.
const LONG_FAMILY_CHARS: [usize; 5] = [1_000_000, 300_000, 100_000, 10_000, 2_000];

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
    ) -> Result<CommittedRow, String> {
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
/// iteratively). On a refusal, the status and the start of the body.
async fn commit(
    app: &common::E2eTestApp,
    kind: &str,
    entry: &Entry,
) -> Result<CommittedRow, String> {
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
        return Err(format!("{status}: {}", excerpt(&body)));
    }
    let committed: Committed = serde_json::from_slice(&body)
        .unwrap_or_else(|e| panic!("the commit door's answer names the row ({e})"));
    Ok(committed.artifact)
}

/// Declare an advisory shape for `kind` in the context, declaring the asked path `tag` with no
/// constraint, so content of any form commits under it.
async fn declare(app: &common::E2eTestApp, context: Uuid, kind: &str) -> Result<Uuid, String> {
    app.client
        .data_artifacts()
        .declare_shape(
            context,
            &ShapeDeclareRequest {
                kind: kind.to_string(),
                kind_owner: None,
                schema: json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": { "tag": {} }
                }),
                enforcement: EnforcementMode::Advisory,
                act: Default::default(),
            },
        )
        .await
        .map(|declared| declared.shape_id.uuid())
        .map_err(|e| e.to_string())
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

    // Probes: in the corpus if the commit door admits them, absent if it refuses them.
    let nul_in_text = with_tag("nul-in-text", json!("a\u{0}b"));
    let nul_in_key = json!({ "tag": "nul-in-key", "k\u{0}": 1 });
    for (label, content) in [("nul-in-text", nul_in_text), ("nul-in-key", nul_in_key)] {
        let _admitted_or_not = family
            .try_put(
                &app,
                KIND,
                Entry::member(label, one, content, Expect::Served),
            )
            .await;
    }
    for depth in (ARTIFACT_ASK_CONTENT_DEPTH + 2..=DEEPEST_PROBED).rev() {
        let entry = Entry::member(
            format!("deepest-admitted ({depth} deep)"),
            one,
            nested(depth),
            Expect::Oversized,
        );
        if family.try_put(&app, KIND, entry).await.is_ok() {
            break;
        }
    }

    let long = long_family(&app, *context.id, two).await;
    World {
        app,
        shape,
        family,
        trio,
        trio_end: trio_end.expect("three read-spending artifacts"),
        long,
    }
}

/// The longest name in [`LONG_FAMILY_CHARS`] the shape door and the commit door both admit, with
/// four small artifacts and one without content committed under it. Where a page puts its rows
/// follows from the name: a row writes it once, as a JSON string, beside small content and fixed
/// fields within [`ARTIFACT_ROW_FIXED_BYTES`].
async fn long_family(app: &common::E2eTestApp, context: Uuid, resource: Uuid) -> LongFamily {
    for chars in LONG_FAMILY_CHARS {
        let kind = "\u{1}".repeat(chars);
        let Ok(shape) = declare(app, context, &kind).await else {
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
        if family.try_put(app, &kind, first).await.is_err() {
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

    /// Fail with every finding, the first hundred in full.
    fn assert_none(&self) {
        let shown: Vec<&str> = self.0.iter().take(100).map(String::as_str).collect();
        assert!(
            self.0.is_empty(),
            "{} finding(s) on the hostile corpus:\n{}",
            self.0.len(),
            shown.join("\n")
        );
    }
}

/// Which corpus items an ask matches.
type Matches = fn(&Item) -> bool;

/// One walk: an ask, a limit, and the family whose artifacts it should see.
struct Walk<'a> {
    name: String,
    ask: &'a Value,
    limit: Option<u32>,
    family: &'a Family,
    matches: Matches,
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
/// page must be: answered, within the platform's response cap and the body the MCP relay is sized
/// for, parsed from its bytes as an `AskResponse`, nested within the Ruby client's limit, and read
/// alike by the client. `None` when the walk cannot go on from it.
async fn read_page(
    app: &common::E2eTestApp,
    request: &Value,
    at: &str,
    findings: &mut Findings,
) -> Option<AskResponse> {
    let (status, body) = post(app, "/api/data-artifacts/ask", request).await;
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
    let written = serde_json::to_value(&raw).expect("a page serializes");
    let as_ruby_parses = nesting(&written) + 1;
    if as_ruby_parses > RUBY_MAX_NESTING {
        findings.note(format!(
            "{at}: the page nests {as_ruby_parses} deep as the Ruby client parses it, past its \
             {RUBY_MAX_NESTING}"
        ));
    }
    match app.client.data_artifacts().ask(request).await {
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
/// exactly once. The pages seen, in order.
async fn walk_to_the_end(
    app: &common::E2eTestApp,
    walk: &Walk<'_>,
    findings: &mut Findings,
) -> Vec<PageSeen> {
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
        let body = request(walk.ask, walk.limit, after);
        let Some(page) = read_page(app, &body, &at, findings).await else {
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
    pages
}

/// A limit as a walk's name shows it.
fn shown(limit: Option<u32>) -> String {
    limit.map_or_else(|| "absent".to_string(), |limit| limit.to_string())
}

/// **Every walk of a hostile corpus ends, and every page parses, fits and serves whole content.**
/// The hostile family is asked two ways — `tag` present, which its non-object, untagged,
/// content-less, pinned and folded artifacts must not answer, and with no predicates, which every
/// live member and current artifact answers, content-less ones included — and the long family with
/// no predicates; each at every limit in [`LIMITS`]. A walk that may examine three rows a page
/// (no limit, 7, the ceiling) stops its first page on the read frontier: the three read-spending
/// artifacts named oversized, nothing served, and `next` at the third, though the row limit left
/// room and the page had spent three entries' worth of its budget on a page whose next row is
/// small. So only the read budget can have ended it.
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
///   frontier, answers `next: null`, so the walk ends there and every later artifact is missed.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_walk_of_a_hostile_corpus_ends_and_every_page_parses_fits_and_is_whole(pool: PgPool) {
    let w = world(pool).await;
    let mut findings = Findings::default();
    let tag_ask = json!({ "shape": w.shape, "predicates": [
        { "path": ["tag"], "op": { "op": "has_key" } }
    ] });
    let whole_ask = json!({ "shape": w.shape });
    let asks: [(&str, &Value, Matches); 2] = [
        ("`tag` present", &tag_ask, |item: &Item| item.tagged),
        ("no predicates", &whole_ask, |item: &Item| item.whole),
    ];
    for (ask_name, ask, matches) in asks {
        for limit in LIMITS {
            let walk_of = Walk {
                name: format!("{ask_name}, limit {}", shown(limit)),
                ask,
                limit,
                family: &w.family,
                matches,
            };
            let pages = walk_to_the_end(&w.app, &walk_of, &mut findings).await;
            if limit.is_none_or(|limit| limit >= 3) {
                let first = pages.first();
                let on_the_frontier = first.is_some_and(|page| {
                    page.served.is_empty()
                        && page.oversized == w.trio
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
            matches: |item: &Item| item.whole,
        };
        walk_to_the_end(&w.app, &walk_of, &mut findings).await;
    }
    findings.assert_none();
}
