//! The transport layers every HTTP surface applies below its own root span.
//!
//! Companion to [`crate::cors`], and here for the same reason: temper-api and temper-mcp each
//! assemble their own router, so anything expressed twice is something one of them can be built
//! without. That is not hypothetical — `RequestDecompressionLayer` and the fallback handler were
//! both applied by `apply_transport_layers` (shared by the public and internal API apps) and by
//! neither of MCP's stacks, so on the agent-facing surface a gzip body reached the JSON extractor
//! still compressed, and an unmatched path was answered by the auth middleware rather than by a
//! 404 of any shape.
//!
//! The root span stays per-surface deliberately: temper-api names it `http_request` and temper-mcp
//! names it `mcp_request`, because three different things under one name is unreadable once they
//! are exported together.

use axum::extract::DefaultBodyLimit;
use axum::http::header::{self, HeaderValue};
use axum::Router;
use tower_http::decompression::RequestDecompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;

/// The response headers both public HTTP surfaces set on every response.
///
/// **These are set in-app, not verified at the edge, and that is the whole point.** The hosting
/// platform may well send some of them; it can also be reconfigured, or replaced, without any
/// change to this repository. A control that only holds while something in front of the instance
/// keeps behaving is not a control this codebase can claim.
///
/// Each is applied `if_not_present`, so the baseline is a **floor a handler can deliberately
/// raise or relax for its own content**, and every relaxation is visible next to the response it
/// belongs to rather than hidden in a wider policy here. One route needs that today: temper-api's
/// Slack callback renders HTML with an inline `<style>`, which the policy below forbids.
///
/// | Header | Value | Why this value |
/// |---|---|---|
/// | `x-content-type-options` | `nosniff` | Both surfaces answer `application/json` almost everywhere. Content sniffing can only ever disagree with a `Content-Type` these surfaces set deliberately |
/// | `x-frame-options` | `DENY` | Neither surface has a page meant to be framed. The Slack callback is a terminal page, not an embed |
/// | `referrer-policy` | `no-referrer` | The Slack callback carries state in its URL. A referrer leak from it is the one path either surface has to leaking a URL to a third party |
/// | `strict-transport-security` | `max-age=63072000; includeSubDomains` | Two years, the usual floor for preload eligibility. `preload` itself is **not** sent — it is a submission to a browser-vendor list that is painful to reverse, and that is an operator's decision, not a default |
/// | `content-security-policy` | `default-src 'none'; frame-ancestors 'none'; base-uri 'none'` | A JSON API loads nothing, so the correct policy is *nothing*. `frame-ancestors` is what actually enforces the framing rule in modern browsers; `x-frame-options` is above it for the ones that never learned |
fn security_header_layers<S>(app: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    const NOSNIFF: HeaderValue = HeaderValue::from_static("nosniff");
    const DENY: HeaderValue = HeaderValue::from_static("DENY");
    const NO_REFERRER: HeaderValue = HeaderValue::from_static("no-referrer");
    const HSTS: HeaderValue = HeaderValue::from_static("max-age=63072000; includeSubDomains");

    app.layer(SetResponseHeaderLayer::if_not_present(
        header::X_CONTENT_TYPE_OPTIONS,
        NOSNIFF,
    ))
    .layer(SetResponseHeaderLayer::if_not_present(
        header::X_FRAME_OPTIONS,
        DENY,
    ))
    .layer(SetResponseHeaderLayer::if_not_present(
        header::REFERRER_POLICY,
        NO_REFERRER,
    ))
    .layer(SetResponseHeaderLayer::if_not_present(
        header::STRICT_TRANSPORT_SECURITY,
        HSTS,
    ))
    .layer(SetResponseHeaderLayer::if_not_present(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(JSON_CONTENT_SECURITY_POLICY),
    ))
}

/// The policy for a surface that renders nothing — which is every route on both surfaces except
/// the two that answer with HTML.
pub const JSON_CONTENT_SECURITY_POLICY: &str =
    "default-src 'none'; frame-ancestors 'none'; base-uri 'none'";

/// Replace the baseline content-security policy on one sub-router.
///
/// The baseline is applied `if_not_present`, so a route that answers with HTML can set its own and
/// keep it. A handler assembling a single response does that on the response itself; this is for
/// the case where the exception covers a whole sub-router nobody hand-writes — Swagger's bundle,
/// which serves its own scripts, styles and images.
///
/// Kept here rather than layered at the call site so the surfaces do not each grow a `tower-http`
/// dependency to relax a policy this module set.
pub fn override_content_security_policy<S>(app: Router<S>, policy: &'static str) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    app.layer(SetResponseHeaderLayer::overriding(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(policy),
    ))
}

/// The hosting platform's hard request-body cap on Vercel, where temper's hosted deployments run:
/// a body past it is refused by the platform with a bare `413` before any door here reads it, so no
/// door limit above it is reachable there. Self-hosted proxies may admit more.
///
/// Two things are held under it. `BLOB_SINGLE_REQUEST_MAX_BYTES` is warned against when it exceeds
/// this (WARN, not refuse; B-C5, final-pass review). And the largest legal composition, at the most
/// expansive encoder, is measured against it by
/// `the_largest_legal_composition_fits_inside_the_declared_body_limit` in temper-api, because a
/// plan the contract calls legal must not meet that bare `413` on the deployment we run.
pub const VERCEL_REQUEST_BODY_CAP_BYTES: usize = 4_500_000;

/// The hosting platform's hard response-body cap on Vercel: a function whose response body is past
/// it fails there, whatever the door meant to send. The same figure as the request cap.
///
/// What is held under it: a batch read's page, through [`ARTIFACT_ASK_PAGE_BYTES`], on both doors
/// that answer it — the API's body and the MCP relay's.
pub const VERCEL_RESPONSE_BODY_CAP_BYTES: usize = 4_500_000;

/// The most one page of a batch read (`POST /api/data-artifacts/ask`) costs, rows and all: the sum
/// of what every row the page examines is charged. A row that fits this alone is charged its
/// `content_bytes` and its metadata; a matching row that does not is named in the page's
/// `oversized` list without its content and charged [`ARTIFACT_ROW_FIXED_BYTES`]; a row that fits
/// alone but not in what is left ends the page, and the next page starts with it. The charge is
/// made in SQL (`artifacts_matching_ask`, migration `20261106000000`). The row limit and the read
/// budget ([`ARTIFACT_ASK_READ_BYTES`]) can end a page too; the earliest of the three does.
///
/// # 2,000,000 bytes, under [`VERCEL_RESPONSE_BODY_CAP_BYTES`] twice over
///
/// **Every byte a row writes is charged.** `content_bytes` is the length of the committed content
/// in compact JSON, and a page serves that same value compactly re-serialized from storage. That
/// equality holds for numbers only because every crate that parses served content parses a float
/// back to the value it was written from: `jsonb` prints a number in plain notation (`1e300` as 301
/// digits), and only `serde_json`'s `float_roundtrip` feature, which this crate and temper-client
/// declare (and `jsonschema` enables in every binary that links it), guarantees that parse is
/// exact, so the float is written back in its shortest form. `serde_json`'s `arbitrary_precision`
/// must stay off for the same reason: with it, the plain expansion itself would be written back.
/// `served_floats_are_as_long_as_their_content_bytes` (temper-services' `artifact_ask_read_test`)
/// commits floats through storage and measures what is served. A row's metadata is each of its
/// text fields (the family's namespace table and name, `intent`,
/// `content_hash`, `shape_state`) at the length of the JSON string the page writes for it —
/// quotes and escapes included, so a family name of any length is paid for on every row — plus
/// [`ARTIFACT_ROW_FIXED_BYTES`] for everything else the row writes: its keys and punctuation, ids,
/// numbers, timestamp and flag. An oversized entry writes only ids and a size, under that same
/// constant.
///
/// So a page's rows are at most this many bytes of its body, and **the envelope** (shape,
/// population, order, `next`, `amended_since`, keys) is under [`ARTIFACT_PAGE_ENVELOPE_BYTES`]:
/// the API's body is at most 2,000,000 + 1,024 bytes. **The MCP relay re-encodes that body as one
/// escaped JSON string**, and the budget is sized for it on two conditions the tool must keep: the
/// body is written **compactly** — pretty-printing adds indentation per nesting level, which no
/// byte budget bounds — and it is sent once, as text. Escaping compact JSON at most doubles it (`"`
/// and `\` gain a backslash; every other byte it emits is written as it is or gains one byte).
/// Twice the body plus the relay's own envelope is 4,003,072 bytes, under the 4,500,000 cap.
pub const ARTIFACT_ASK_PAGE_BYTES: usize = 2_000_000;

/// What one batch-read row is charged beside its content and its text fields — every other byte an
/// `ArtifactView` row writes — and what an oversized entry is charged whole (see
/// [`ARTIFACT_ASK_PAGE_BYTES`]). The widest such row writes under 400 bytes of it.
pub const ARTIFACT_ROW_FIXED_BYTES: usize = 1024;

/// The most a batch-read page's envelope, or the MCP relay's around it, costs (see
/// [`ARTIFACT_ASK_PAGE_BYTES`]).
pub const ARTIFACT_PAGE_ENVELOPE_BYTES: usize = 1024;

/// The most one page of a batch read may read to find its rows, beside what it sends
/// ([`ARTIFACT_ASK_PAGE_BYTES`]). The matcher reads a candidate's whole content to decide whether
/// it matches, so a sparse ask, or a run of oversized matches, would otherwise read every remaining
/// artifact of the home to fill one page. Each candidate after the position is charged its
/// `content_bytes` and [`ARTIFACT_READ_CANDIDATE_BYTES`]; a candidate is read only while the charge
/// before it is under this budget, so the first is always read and a page always moves forward.
/// When the budget ends the read first, the page's `next` is the last candidate read, matched or
/// not, so a page can carry no artifacts and still go on. The charge is made in SQL
/// (`__temper_ungated_artifact_ask_rows`, migration `20261106000000`).
///
/// The first page's population is not under this budget: it is the ask's exact counts over the
/// whole counted set, which a read budget could only make approximate.
///
/// # 8,000,000 bytes: four page budgets
///
/// **A full page is always readable.** The rows a full page serves carry at most
/// [`ARTIFACT_ASK_PAGE_BYTES`] of content, and `MAX_ARTIFACT_PAGE_ROWS` (200) candidates at
/// [`ARTIFACT_READ_CANDIDATE_BYTES`] each add 819,200, so a page whose candidates all match is never
/// stopped by this budget before it is full: 2,819,200 is under 8,000,000. The rest is room for the
/// candidates a sparse ask reads and does not match before a page is cut short.
///
/// **The worst a page reads.** Reading stops at the first candidate whose charge before it reaches
/// the budget, so one page reads under the budget plus one candidate's content. An artifact's
/// content arrives in a commit body under the gated 25 MiB limit (`GATED_MAX_BODY_BYTES`,
/// temper-api's `routes/mod.rs`), so a page's matcher reads under 8,000,000 + 26,214,400 =
/// 34,214,400 bytes of content, over at most 8,000,000 / 4,096 + 1 = 1,954 candidates (integer
/// division). Serving re-reads the content of the rows it serves, at most
/// [`ARTIFACT_ASK_PAGE_BYTES`] more.
pub const ARTIFACT_ASK_READ_BYTES: usize = 8_000_000;

/// What every candidate a batch-read page reads is charged beside its `content_bytes` (see
/// [`ARTIFACT_ASK_READ_BYTES`]): the fixed cost of reading one — its lookup and its match — in the
/// budget's own unit, so a run of candidates with little or no content is bounded by count as well
/// as by bytes. 4,096 bytes, the order of one heap page.
pub const ARTIFACT_READ_CANDIDATE_BYTES: usize = 4_096;

/// The default request-body ceiling, in **decompressed** bytes, for a door that declares none.
///
/// **This number is ratified, not derived, and that is the point of it being here.** Before this
/// constant existed the ceiling was axum's `DefaultBodyLimit` default — the same 2 MiB, but owned
/// by a dependency, so a framework upgrade could have moved temper's request ceiling with nothing
/// in this repository failing. Setting it explicitly changes no behaviour and moves the decision
/// here, where a change to it is a change someone made on purpose.
///
/// # Read the per-door limits before reasoning about this one
///
/// This is the floor for ordinary requests, not a ceiling over the whole instance. A limit applied
/// to a narrower router runs *inside* this one and therefore wins there, and most of temper-api's
/// authenticated surface carries one:
///
/// | Door | Limit | Where |
/// |---|---|---|
/// | every `Tier::Gated` route (system-access routes, admin routes) | 25 MB | `GATED_MAX_BODY_BYTES`, `temper-api/src/routes/mod.rs` |
/// | `/api/query` | 25 MB | `QUERY_MAX_BODY_BYTES`, `temper-api/src/routes/query.rs` |
/// | blob segment uploads | 4.5 MB | `BLOB_SEGMENT_MAX_BODY_BYTES`, `temper-api/src/routes/blob_doors.rs` |
/// | blob commit | the config's single-request threshold | `blob_commit_body_limit`, `temper-api/src/routes/blob_doors.rs` |
/// | signed internal routes | 64 KiB | `temper-api/src/middleware/internal_auth.rs` |
/// | GitHub webhook intake | 25 MiB | `GITHUB_MAX_WEBHOOK_BYTES`, `temper-api/src/routes/webhook_intake.rs` |
/// | `/mcp` | 25 MB | `MCP_MAX_BODY_BYTES`, `temper-mcp-server/src/router.rs` |
///
/// So this constant governs temper-api's public, auth-only and self-gated routes that declare no
/// limit, and temper-mcp's *axum* routes (discovery, registration, health).
///
/// **`/mcp` is not merely an exception — this constant cannot reach it.** That door is mounted with
/// `nest_service` over a raw tower service, so no axum extractor runs and `DefaultBodyLimit` is
/// inapplicable rather than overridden; it uses `RequestBodyLimitLayer` instead.
///
/// # Why 2 MiB, and the measurement that bounds the claim
///
/// Ordinary request bodies here are small: the largest first-party document in this repository is
/// ~70 KB and the largest resource body observed in the vault is ~34 KB. 2 MiB is roughly thirty
/// times the former, which is ample for a door carrying one of them.
///
/// **It is emphatically not ample for every payload the contract admits, and that is the reason the
/// table above exists rather than a reason to raise this.** A composition `/api/query` calls legal
/// encodes to up to **4,358,218 bytes** `[measured — 2026-10-07]`, 2.1x this number. Had that
/// door inherited this ceiling it would have answered a legal plan with a bare 413. The doors
/// carrying large payloads by design — a composition, and the MCP tool surface with `ingest`'s
/// inline content and `data_artifacts`' JSON — each declare their own, which is the shape this
/// constant is the default half of.
///
/// So: **do not raise this to accommodate a door that needs more.** Give that door its own limit and
/// state why, as every door above does.
pub const MAX_REQUEST_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Apply the layers that sit **below** a surface's root span: the fallback handler, request
/// decompression, the request-body ceiling, and the baseline response headers.
///
/// Call this first, then add the surface's own root span and its CORS layer:
///
/// ```ignore
/// apply_base_layers(app)
///     .layer(axum::middleware::from_fn(root_span))
///     .layer(temper_services::cors::cors_layer(&config.cors_origins))
/// ```
///
/// Ordering matters and is preserved from the stack this was extracted from: decompression runs
/// innermost, so the body an extractor sees — and therefore any `DefaultBodyLimit` guarding it —
/// is measured in decompressed bytes.
pub fn apply_base_layers<S>(app: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let app = app
        .fallback(fallback_handler)
        .layer(RequestDecompressionLayer::new())
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES));

    security_header_layers(app)
}

/// Bound on the echoed request path in both the `unmatched route` event and the 404
/// body. A path is attacker-chosen input like any other; without a cap its length is
/// whatever the client sent.
const MAX_ECHOED_PATH_BYTES: usize = 512;

/// The path as it may appear in a log event or an error body: redacted through
/// [`temper_core::redact::redact_path`] — the same function the request root
/// spans run — and capped at [`MAX_ECHOED_PATH_BYTES`].
///
/// The fallback sits *outside* the root span (it is the answer when no route exists),
/// so nothing else applies redaction to what it records. An unmatched path can also
/// carry a credential-shaped segment — the invitation-token family this repo once
/// leaked exactly this way — so the event and the body must not become a side door
/// around the deny-by-default guard.
fn sanitize_echoed_path(path: &str) -> std::borrow::Cow<'_, str> {
    let redacted = temper_core::redact::redact_path(path);
    if redacted.len() <= MAX_ECHOED_PATH_BYTES {
        return redacted;
    }
    let mut cut = MAX_ECHOED_PATH_BYTES;
    while !redacted.is_char_boundary(cut) {
        cut -= 1;
    }
    std::borrow::Cow::Owned(format!("{}…", &redacted[..cut]))
}

/// Answer an unmatched route with temper's structured error body.
///
/// Axum's default fallback returns a bare 404 with an empty body. Every other error a caller can
/// receive from temper is an [`crate::error::ErrorBody`], so a client that parses errors would
/// have to special-case exactly this one.
pub async fn fallback_handler(req: axum::extract::Request) -> axum::response::Response {
    use axum::response::IntoResponse;

    let path = sanitize_echoed_path(req.uri().path());
    let method = req.method().to_string();
    tracing::warn!(path = %path, method = %method, "unmatched route");
    let body =
        crate::error::ErrorBody::new("NOT_FOUND", format!("No route matches {method} {path}"));
    (axum::http::StatusCode::NOT_FOUND, axum::Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A full page of a batch read fits the response cap after the MCP relay escapes it**, by the
    /// arithmetic on [`ARTIFACT_ASK_PAGE_BYTES`]: the budget charges every byte a row writes, so the
    /// body is the budget and the envelope. Bite: the budget raised to 2,300,000, or the doubling
    /// dropped from the sum.
    #[test]
    fn a_full_batch_page_fits_the_response_cap_after_the_relay_escapes_it() {
        let body = ARTIFACT_ASK_PAGE_BYTES + ARTIFACT_PAGE_ENVELOPE_BYTES;
        let relayed = 2 * body + ARTIFACT_PAGE_ENVELOPE_BYTES;
        assert_eq!(relayed, 4_003_072, "the figure the budget's doc states");
        assert!(relayed <= VERCEL_RESPONSE_BODY_CAP_BYTES);
    }

    /// The JSON-string length of one text field, as a page writes it and as the page's SQL charges
    /// it (`octet_length(to_json(..)::text)`).
    fn written(text: &str) -> usize {
        serde_json::to_string(text)
            .expect("a string serializes")
            .len()
    }

    /// **[`ARTIFACT_ROW_FIXED_BYTES`] covers every byte a row writes but its content and its text
    /// fields**, measured on the widest row there is: the longest `f64` and `i64` forms, a
    /// nanosecond timestamp, `is_folded` false, the `content` key present. And an oversized entry is
    /// under it whole. The text fields here carry escapes, so the subtraction is of their written
    /// length, which is what the SQL charges. Bite: the constant lowered to 256.
    #[test]
    fn the_fixed_row_charge_covers_every_byte_but_content_and_text() {
        use temper_core::types::artifact_ask::OversizedArtifact;
        use temper_core::types::data_artifact::ArtifactView;
        use temper_core::types::ids::{DataArtifactId, ResourceId};

        let id = uuid::Uuid::max();
        let content = serde_json::Value::from(0);
        let row = ArtifactView {
            artifact_id: DataArtifactId::from(id),
            resource_id: ResourceId::from(id),
            kind_owner_table: "kb_profiles".to_string(),
            kind_owner_id: id,
            artifact_kind: "a \"quoted\" \\ family\u{1}".to_string(),
            intent: "member".to_string(),
            precedence: -f64::MIN_POSITIVE,
            content_hash: "f".repeat(64),
            content_bytes: i64::MIN,
            shape_state: "declared_not_yet_checked".to_string(),
            is_folded: false,
            created: chrono::DateTime::parse_from_rfc3339("2026-10-24T09:15:30.123456789Z")
                .expect("literal parses")
                .with_timezone(&chrono::Utc),
            content: Some(content.clone()),
        };
        let text = written(&row.kind_owner_table)
            + written(&row.artifact_kind)
            + written(&row.intent)
            + written(&row.content_hash)
            + written(&row.shape_state);
        let whole = serde_json::to_string(&row).expect("a row serializes").len();
        let content_len = serde_json::to_string(&content).expect("serializes").len();
        // One more byte for the comma that separates it from the next row.
        let fixed = whole - content_len - text + 1;
        assert!(
            fixed <= ARTIFACT_ROW_FIXED_BYTES,
            "a row writes {fixed} bytes beside its content and text"
        );

        let entry = serde_json::to_string(&OversizedArtifact {
            artifact_id: DataArtifactId::from(id),
            resource_id: ResourceId::from(id),
            content_bytes: i64::MIN,
        })
        .expect("an entry serializes")
        .len();
        assert!(
            entry < ARTIFACT_ROW_FIXED_BYTES,
            "an entry writes {entry} bytes"
        );
    }

    /// The fields of a row whose JSON string has one width whatever the row: the ids and the
    /// timestamp.
    const FIXED_WIDTH_STRINGS: [&str; 4] =
        ["artifact_id", "resource_id", "kind_owner_id", "created"];

    /// The text fields the page's SQL measures on every row it serves.
    const MEASURED_STRINGS: [&str; 5] = [
        "kind_owner_table",
        "artifact_kind",
        "intent",
        "content_hash",
        "shape_state",
    ];

    /// **Every string a row writes is either of one width or measured by the page's charge**, and
    /// with every text field long the rest of the row still fits [`ARTIFACT_ROW_FIXED_BYTES`]. The
    /// row is serialized and read back as an object: every key whose value is a string must be one
    /// of [`FIXED_WIDTH_STRINGS`] or one of [`MEASURED_STRINGS`], and each measured one must appear
    /// in the page's charge (`octet_length(to_json(<alias>.<field>)::text)`) in the migration that
    /// defines `artifacts_matching_ask`. A string field added to `ArtifactView` later has to be
    /// named in the literal below to compile, and once serialized it is a string key in neither
    /// list, so this goes red until the SQL measures it or it is declared fixed width. Bites: a
    /// sixth text field on `ArtifactView`; `"intent"` dropped from [`MEASURED_STRINGS`]; or the
    /// page's `to_json(d.intent)` term removed from the migration.
    #[test]
    fn every_string_a_row_writes_is_of_one_width_or_measured_by_the_page() {
        use temper_core::types::data_artifact::ArtifactView;
        use temper_core::types::ids::{DataArtifactId, ResourceId};

        // A quote, a backslash, a C0 control and a character outside the BMP, so the written
        // length is not the byte length.
        let long = "\"\\x\u{1}\u{1F600}".repeat(512);
        let id = uuid::Uuid::max();
        let content = serde_json::Value::from(0);
        let row = ArtifactView {
            artifact_id: DataArtifactId::from(id),
            resource_id: ResourceId::from(id),
            kind_owner_table: long.clone(),
            kind_owner_id: id,
            artifact_kind: long.clone(),
            intent: long.clone(),
            precedence: -f64::MIN_POSITIVE,
            content_hash: long.clone(),
            content_bytes: i64::MIN,
            shape_state: long.clone(),
            is_folded: false,
            created: chrono::DateTime::parse_from_rfc3339("2026-10-24T09:15:30.123456789Z")
                .expect("literal parses")
                .with_timezone(&chrono::Utc),
            content: Some(content.clone()),
        };
        let page_sql = crate::MIGRATOR
            .iter()
            .find(|m| m.version == 20_261_106_000_000)
            .expect("the migration that defines the page")
            .sql
            .clone();

        let object = serde_json::to_value(&row).expect("a row serializes");
        let object = object.as_object().expect("a row is an object");
        let mut text = 0;
        for (key, value) in object {
            let Some(s) = value.as_str() else { continue };
            if FIXED_WIDTH_STRINGS.contains(&key.as_str()) {
                continue;
            }
            assert!(
                MEASURED_STRINGS.contains(&key.as_str()),
                "`{key}` is a string a row writes that the page's charge does not measure"
            );
            assert!(
                page_sql.contains(&format!(".{key})::text)")),
                "the page's charge measures `{key}`"
            );
            text += written(s);
        }
        assert_eq!(
            text,
            MEASURED_STRINGS.len() * written(&long),
            "every measured field is on the row"
        );
        let whole = serde_json::to_string(&row).expect("a row serializes").len();
        let content_len = serde_json::to_string(&content).expect("serializes").len();
        // One more byte for the comma that separates it from the next row.
        let fixed = whole - content_len - text + 1;
        assert!(
            fixed <= ARTIFACT_ROW_FIXED_BYTES,
            "a row writes {fixed} bytes beside its content and text"
        );
    }

    /// The gated commit body limit an artifact's content arrives under (`GATED_MAX_BODY_BYTES`,
    /// temper-api's `routes/mod.rs`, which this crate cannot name).
    const COMMIT_BODY_LIMIT_BYTES: usize = 25 * 1024 * 1024;

    /// **The read budget reads a full page, and bounds the worst page by the figures its doc
    /// states.** A page of `MAX_ARTIFACT_PAGE_ROWS` matching rows carrying the whole page budget of
    /// content is read within the budget; the worst read is the budget and one artifact at the
    /// commit limit; the candidates one page reads are the budget over the per-candidate charge,
    /// plus the first. Bites: the read budget lowered below 2,819,200, or the per-candidate charge
    /// changed without its doc.
    #[test]
    fn the_read_budget_reads_a_full_page_and_bounds_the_worst_one() {
        let rows = usize::try_from(temper_core::types::data_artifact::MAX_ARTIFACT_PAGE_ROWS)
            .expect("fits");
        let full_page = ARTIFACT_ASK_PAGE_BYTES + rows * ARTIFACT_READ_CANDIDATE_BYTES;
        assert_eq!(
            full_page, 2_819_200,
            "the figure the read budget's doc states"
        );
        // At least the page budget, with every row's read charge beside it.
        assert!(full_page <= ARTIFACT_ASK_READ_BYTES);
        assert_eq!(
            ARTIFACT_ASK_READ_BYTES + COMMIT_BODY_LIMIT_BYTES,
            34_214_400,
            "the worst content one page reads, as its doc states"
        );
        assert_eq!(
            ARTIFACT_ASK_READ_BYTES / ARTIFACT_READ_CANDIDATE_BYTES + 1,
            1_954,
            "the most candidates one page reads, as its doc states"
        );
    }

    /// An unmatched request is exactly how a token-in-path would travel if a route ever
    /// reintroduced one — the fallback must run the same redaction the root spans do,
    /// or it becomes the one recording site a credential can pass through unredacted.
    #[test]
    fn the_echoed_path_is_redacted_like_a_span_attribute() {
        let token_path = "/api/invitations/9f8e7d6c5b4a39281706f5e4d3c2b1a0".to_string(); // gitleaks:allow — fake token literal in a redaction regression test
        let sanitized = sanitize_echoed_path(&token_path);
        assert_eq!(sanitized, "/api/invitations/{token}");
        assert!(!sanitized.contains("9f8e7d6c"));
    }

    /// Normal paths — the overwhelming majority — pass through untouched and unallocated.
    #[test]
    fn an_ordinary_path_passes_through_untouched() {
        let sanitized = sanitize_echoed_path("/api/resources/019f97a7-ad61-7e40-b325-73028060ac06");
        assert_eq!(
            sanitized,
            "/api/resources/019f97a7-ad61-7e40-b325-73028060ac06"
        );
        assert!(matches!(sanitized, std::borrow::Cow::Borrowed(_)));
    }

    /// The cap is what makes the echo bounded: a pathological path is cut, not
    /// reflected at full length into the log stream and the response body.
    #[test]
    fn an_oversized_path_is_truncated_on_a_char_boundary() {
        let long = format!("/{}", "a".repeat(MAX_ECHOED_PATH_BYTES * 4));
        let sanitized = sanitize_echoed_path(&long);
        assert!(sanitized.len() <= MAX_ECHOED_PATH_BYTES + '…'.len_utf8());
        assert!(sanitized.ends_with('…'));
        // A multibyte path must not be cut mid-character.
        let multibyte = format!("/{}", "é".repeat(MAX_ECHOED_PATH_BYTES));
        let sanitized = sanitize_echoed_path(&multibyte);
        assert!(sanitized.is_char_boundary(sanitized.len() - '…'.len_utf8()));
    }
}
