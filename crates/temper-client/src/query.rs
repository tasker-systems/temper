//! Typed sub-client for the `/api/query` endpoint.
//!
//! **The method is `run`, not `query`.** `SearchClient` already has a `query` (`search.rs`), which
//! takes a pre-computed embedding and answers the wide arm — a different thing entirely. Two
//! sibling sub-clients each exposing `query`, meaning different things, is a collision a reader
//! meets at the call site rather than at the definition. `run` also says what this does: a
//! composition is a plan, and a plan is run.

use reqwest::Method;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::query::{Composition, QueryResponse};

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
    pub async fn run(&self, composition: &Composition) -> Result<QueryResponse> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/query").json(composition);
        let resp = self
            .http
            .send(&Method::POST, "/api/query", req, Some(&token))
            .await?;
        let bytes = resp.bytes().await?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}
