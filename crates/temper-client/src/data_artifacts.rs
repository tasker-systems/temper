//! Typed sub-client for the `/api/resources/{id}/artifacts` endpoints.

use uuid::Uuid;

use crate::error::{ClientError, Result};
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::artifact_ask::AskResponse;
use temper_core::types::data_artifact::{
    ArtifactCommitRequest, ArtifactCommitResponse, ArtifactCursor, ArtifactListParams,
    ArtifactPage, ArtifactPageRequest, ArtifactView, ARTIFACT_DRAIN_PAGE_ROWS,
};
use temper_core::types::data_artifact_shape::{ShapeDeclareRequest, ShapeView};
use temper_core::types::resource::SortOrder;

/// Sub-client for data artifact reads and writes.
pub struct DataArtifactsClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for DataArtifactsClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataArtifactsClient")
            .finish_non_exhaustive()
    }
}

/// The position a `Link` header's `rel="next"` entry continues after, or `None` when the header
/// names no next page.
///
/// Comma-separated entries, each `<target>` followed by `;`-separated parameters. An entry is
/// the next page's when its `rel` names `next`, read as RFC 8288 allows: the parameter name and
/// the relation type case-insensitively, the value quoted or bare, and the value a
/// whitespace-separated list of relation types (`rel="next last"`). Only an entry's first `rel`
/// counts. The target's query must hold both `after_created` and `after_id`. An entry that is not
/// `<...>`-shaped, or a next target that does not carry a readable position, is an error: reading
/// it as "no next page" would end a drain early without saying so.
pub fn parse_next_link(header: &str) -> Result<Option<ArtifactCursor>> {
    for entry in header.split(',') {
        let entry = entry.trim();
        let (target, params) = entry
            .strip_prefix('<')
            .and_then(|rest| rest.split_once('>'))
            .ok_or_else(|| malformed_link("an entry is not `<target>`", header))?;
        if !names_next(params) {
            continue;
        }
        let url = url::Url::parse("http://link.invalid/")
            .and_then(|base| base.join(target))
            .map_err(|_| malformed_link("the next target is not a URL", header))?;
        let mut created = None;
        let mut id = None;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "after_created" => created = Some(value.into_owned()),
                "after_id" => id = Some(value.into_owned()),
                _ => {}
            }
        }
        let created = created
            .ok_or_else(|| malformed_link("the next target has no `after_created`", header))?
            .parse::<chrono::DateTime<chrono::Utc>>()
            .map_err(|_| malformed_link("`after_created` is not an RFC 3339 timestamp", header))?;
        let artifact_id = id
            .ok_or_else(|| malformed_link("the next target has no `after_id`", header))?
            .parse::<Uuid>()
            .map_err(|_| malformed_link("`after_id` is not a UUID", header))?;
        return Ok(Some(ArtifactCursor {
            created,
            artifact_id: artifact_id.into(),
        }));
    }
    Ok(None)
}

/// Whether a link entry's parameters carry a `rel` whose relation types include `next`.
fn names_next(params: &str) -> bool {
    params
        .split(';')
        .filter_map(|param| param.split_once('='))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("rel"))
        .is_some_and(|(_, value)| {
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);
            value
                .split_whitespace()
                .any(|rel| rel.eq_ignore_ascii_case("next"))
        })
}

fn malformed_link(why: &str, header: &str) -> ClientError {
    ClientError::Other(format!(
        "malformed `Link` header on the artifact list ({why}): {header}"
    ))
}

/// Whether `next` lies strictly beyond `after` in an ascending `(created, artifact_id)` walk.
/// With no `after` (the first page) any position advances. `ArtifactCursor` has no `Ord` and
/// gains none here, so the tuple is compared explicitly.
fn advances(after: Option<ArtifactCursor>, next: ArtifactCursor) -> bool {
    after.is_none_or(|prev| {
        (next.created, next.artifact_id.uuid()) > (prev.created, prev.artifact_id.uuid())
    })
}

impl<'a> DataArtifactsClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// List artifacts for a resource, in whatever shape the params ask for.
    ///
    /// Returns `serde_json::Value` because the endpoint answers one of two shapes, decided
    /// server-side from the params: the bare array of artifacts, or `Vec<ArtifactCountRow>`
    /// (`counts=true`). It does not read the `Link` continuation header.
    ///
    /// This is the raw door, and its no-`limit` form is the unbounded one a later release
    /// retires. A caller that wants artifacts uses [`Self::list_page`] or [`Self::list_all`];
    /// this one remains for `counts=true`, which is not paged.
    pub async fn list(
        &self,
        resource_id: Uuid,
        params: &ArtifactListParams,
    ) -> Result<serde_json::Value> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_ARTIFACTS;
        let path = op.path(&[&resource_id])?;
        let req = self.http.request(op, &path).query(params);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// One bounded page of a resource's artifacts.
    ///
    /// `params` carries the filters (`kind`, `intent`, `include_folded`); the bound, the
    /// continuation position and the direction come from `request`, which replaces any `limit`,
    /// `after_*` or `order` on `params`. Always asks for artifacts, never counts. The server
    /// refuses a `limit` of 0 or above `MAX_ARTIFACT_PAGE_ROWS`.
    ///
    /// The continuation is the response's `Link: <...>; rel="next"` header; with no such header
    /// the page is the last. An older server that does not know `limit` answers every artifact
    /// with no `Link`, which is read here as the final page. A `Link` that is present but cannot
    /// be read is an error, never a silent last page.
    pub async fn list_page(
        &self,
        resource_id: Uuid,
        params: &ArtifactListParams,
        request: ArtifactPageRequest,
    ) -> Result<ArtifactPage> {
        let paged = ArtifactListParams {
            counts: None,
            limit: Some(request.limit),
            after_created: request.after.map(|c| c.created),
            after_id: request.after.map(|c| c.artifact_id.uuid()),
            order: Some(request.order),
            ..params.clone()
        };
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_ARTIFACTS;
        let path = op.path(&[&resource_id])?;
        let req = self.http.request(op, &path).query(&paged);
        let resp = self
            .http
            .send(&op.method(), &path, req, token.as_deref())
            .await?;
        let mut next = None;
        for value in resp.headers().get_all(reqwest::header::LINK) {
            let value = value
                .to_str()
                .map_err(|_| malformed_link("not visible ASCII", "<non-ASCII value>"))?;
            if let Some(cursor) = parse_next_link(value)? {
                next = Some(cursor);
                break;
            }
        }
        let bytes = resp.bytes().await?;
        let artifacts: Vec<ArtifactView> = serde_json::from_slice(&bytes)?;
        Ok(ArtifactPage { artifacts, next })
    }

    /// Every artifact of a resource, fetched a bounded page at a time.
    ///
    /// For a caller whose contract is "everything": it follows `next` until the last page
    /// (`next: null`), so no single response is unbounded and none is silently truncated. The
    /// page size is [`ARTIFACT_DRAIN_PAGE_ROWS`], oldest first. `params.limit`, `after_*` and
    /// `order` are replaced by the drain's own. A `next` that repeats the position it was asked
    /// to continue after, or that lies behind it in `(created, artifact_id)` order, is an error
    /// rather than another lap: the walk must strictly advance, so a cycling or backward-moving
    /// server cannot grow the result without bound.
    pub async fn list_all(
        &self,
        resource_id: Uuid,
        params: &ArtifactListParams,
    ) -> Result<Vec<ArtifactView>> {
        let mut artifacts = Vec::new();
        let mut after = None;
        loop {
            let page = self
                .list_page(
                    resource_id,
                    params,
                    ArtifactPageRequest {
                        limit: ARTIFACT_DRAIN_PAGE_ROWS,
                        after,
                        order: SortOrder::Asc,
                    },
                )
                .await?;
            artifacts.extend(page.artifacts);
            match page.next {
                // The drain walks ascending, so each position must lie strictly beyond the one
                // just asked for. Equal repeats the page; behind it laps (A, B, A) or runs
                // backward. An honest server's keyset is strict, but a server that is not honest
                // must not turn the drain into unbounded memory growth.
                Some(cursor) if !advances(after, cursor) => {
                    return Err(ClientError::Other(format!(
                        "the artifact list did not advance: its `Link` named a position at or \
                         behind the one it was asked to continue after ({} / {})",
                        cursor.created.to_rfc3339(),
                        cursor.artifact_id.uuid()
                    )));
                }
                Some(cursor) => after = Some(cursor),
                None => return Ok(artifacts),
            }
        }
    }

    /// Get a single artifact by ID under its owning resource.
    pub async fn get(&self, resource_id: Uuid, artifact_id: Uuid) -> Result<ArtifactView> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_ARTIFACT;
        let path = op.path(&[&resource_id, &artifact_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// Get a single artifact by ID alone — the flat read (`GET /api/data-artifacts/{id}`).
    ///
    /// Visibility-gated on the artifact's actual owning resource; answers folded
    /// artifacts; 404 when the artifact does not exist or is not visible. The MCP
    /// `get_data_artifact` tool's door — the tool carries no `resource_id` field.
    pub async fn get_by_id(&self, artifact_id: Uuid) -> Result<ArtifactView> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_ARTIFACT_BY_ID;
        let path = op.path(&[&artifact_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// One page of the artifacts one ask matches (`POST /api/data-artifacts/ask`).
    ///
    /// The request is usually an [`AskRequest`](temper_core::types::artifact_ask::AskRequest), and
    /// may be any value that serializes to one's JSON: the MCP edge relays the asks it is handed
    /// without reading them, so the server alone decides whether a body is readable, and one it
    /// cannot read comes back as [`ClientError::UnreadablePlan`]. Transport only: nothing here
    /// validates the ask or follows `next`. An ask the server will not answer comes back as
    /// [`ClientError::PlanRefused`], carrying every refusal — the ask's and the request's — at once.
    pub async fn ask<P: serde::Serialize + ?Sized>(&self, request: &P) -> Result<AskResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::ASK_DATA_ARTIFACTS;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// Commit one data artifact to a resource.
    pub async fn commit(
        &self,
        resource_id: Uuid,
        request: &ArtifactCommitRequest,
    ) -> Result<ArtifactCommitResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::COMMIT_ARTIFACT;
        let path = op.path(&[&resource_id])?;
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// List live shapes declared for a context home.
    pub async fn list_shapes(&self, context_id: Uuid) -> Result<Vec<ShapeView>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_SHAPES;
        let path = op.path(&[&context_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// Get a single shape by ID.
    pub async fn get_shape(&self, shape_id: Uuid) -> Result<ShapeView> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_SHAPE;
        let path = op.path(&[&shape_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// Declare a shape for a data-artifact family within a context home.
    pub async fn declare_shape(
        &self,
        context_id: Uuid,
        request: &ShapeDeclareRequest,
    ) -> Result<ShapeView> {
        let token = self.http.resolve_token()?;
        let op = &ops::DECLARE_SHAPE;
        let path = op.path(&[&context_id])?;
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// List live shapes declared for a cognitive-map home — the cogmap-home arm of
    /// [`Self::list_shapes`] (`GET /api/cognitive-maps/{id}/shapes`).
    pub async fn list_cogmap_shapes(&self, cogmap_id: Uuid) -> Result<Vec<ShapeView>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_COGMAP_SHAPES;
        let path = op.path(&[&cogmap_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// Declare a shape for a data-artifact family within a cognitive-map home — the
    /// cogmap-home arm of [`Self::declare_shape`] (`POST /api/cognitive-maps/{id}/shapes`).
    pub async fn declare_cogmap_shape(
        &self,
        cogmap_id: Uuid,
        request: &ShapeDeclareRequest,
    ) -> Result<ShapeView> {
        let token = self.http.resolve_token()?;
        let op = &ops::DECLARE_COGMAP_SHAPE;
        let path = op.path(&[&cogmap_id])?;
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// The home-anchored shapes read, dispatched by the anchor's kind — the
    /// parity door for MCP's home-type-keyed tool (its `home_type` is a
    /// vocabulary, not a route).
    pub async fn list_for(
        &self,
        home: temper_core::types::home::HomeAnchor,
    ) -> Result<Vec<ShapeView>> {
        match home {
            temper_core::types::home::HomeAnchor::Context(id) => self.list_shapes(id.uuid()).await,
            temper_core::types::home::HomeAnchor::Cogmap(id) => {
                self.list_cogmap_shapes(id.uuid()).await
            }
        }
    }

    /// The home-anchored shape declare, dispatched by the anchor's kind — same
    /// door as [`Self::list_for`].
    pub async fn declare_for(
        &self,
        home: temper_core::types::home::HomeAnchor,
        request: &ShapeDeclareRequest,
    ) -> Result<ShapeView> {
        match home {
            temper_core::types::home::HomeAnchor::Context(id) => {
                self.declare_shape(id.uuid(), request).await
            }
            temper_core::types::home::HomeAnchor::Cogmap(id) => {
                self.declare_cogmap_shape(id.uuid(), request).await
            }
        }
    }
}
