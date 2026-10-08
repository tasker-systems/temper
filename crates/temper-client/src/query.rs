//! Typed sub-client for the `/api/query` endpoint.
//!
//! **The method is `run`, not `query`.** `SearchClient` already has a `query` (`search.rs`), which
//! takes a pre-computed embedding and answers the wide arm — a different thing entirely. Two
//! sibling sub-clients each exposing `query`, meaning different things, is a collision a reader
//! meets at the call site rather than at the definition. `run` also says what this does: a
//! composition is a plan, and a plan is run.

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::query::QueryResponse;

/// Sub-client for composed queries.
pub struct QueryClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for QueryClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryClient").finish_non_exhaustive()
    }
}

impl<'a> QueryClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Run a composition and return its answer.
    ///
    /// A plan the server will not run comes back as [`crate::error::ClientError::PlanRefused`],
    /// carrying **every** static refusal rather than the first — transport only, no judgment: this
    /// method neither validates the plan before sending nor ranks anything in the response.
    ///
    /// The plan is usually a [`Composition`](temper_core::types::query::Composition), and may be
    /// any value that serializes to one's JSON: the MCP edge relays plans it never parsed, so the
    /// server alone decides whether a plan is readable, and one it cannot read comes back as
    /// [`crate::error::ClientError::UnreadablePlan`].
    pub async fn run<P: serde::Serialize + ?Sized>(&self, plan: &P) -> Result<QueryResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::QUERY;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(plan);
        let resp = self
            .http
            .send(&op.method(), &path, req, Some(&token))
            .await?;
        let bytes = resp.bytes().await?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}
