//! Typed sub-client for the `/api/graph/*` read surface.
//!
//! Covers the two reads with a live caller — the entry orientation slice and the
//! traversal walk. The rest of the family (the retired Atlas six) is deliberately
//! absent; see the door register's *The graph family is a second live departure*.
//!
//! **Seeds and anchors go over the wire comma-separated in ONE param, not as repeated
//! ones.** `handlers/graph.rs` spells them `q.from.split(',')` and `q.places.split(',')`,
//! so a repeated-param encoding hands the service a single unparseable uuid and 400s.
//! The path builders below are pure and unit-tested for exactly that reason.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::graph_atlas::{AtlasEntry, AtlasSubgraph};

/// Join ids the way both graph query params expect: comma-separated, one param.
fn join_ids(ids: &[Uuid]) -> String {
    ids.iter()
        .map(Uuid::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// `GET /api/graph/entry` — both params optional. Empty anchors means the whole
/// visible corpus, which is the read's headline case (a reader who asked nothing).
pub(crate) fn entry_path(anchors: &[Uuid], k: Option<i32>) -> String {
    let mut params: Vec<String> = Vec::new();
    if !anchors.is_empty() {
        params.push(format!("in={}", join_ids(anchors)));
    }
    if let Some(k) = k {
        params.push(format!("k={k}"));
    }
    if params.is_empty() {
        "/api/graph/entry".to_string()
    } else {
        format!("/api/graph/entry?{}", params.join("&"))
    }
}

/// `GET /api/graph/traverse` — `from` is required, `depth` is omitted when the caller
/// names none so the default stays in one place (the handler's `unwrap_or(1)`).
pub(crate) fn traverse_path(seeds: &[Uuid], depth: Option<i32>) -> String {
    let mut path = format!("/api/graph/traverse?from={}", join_ids(seeds));
    if let Some(depth) = depth {
        path.push_str(&format!("&depth={depth}"));
    }
    path
}

/// Sub-client for the graph read surface.
pub struct GraphClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for GraphClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphClient").finish_non_exhaustive()
    }
}

impl<'a> GraphClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// GET /api/graph/entry — the most-connected resources this principal can see,
    /// plus every edge among them, plus the read's own bound declaration.
    ///
    /// `anchors` confines the ranking to resources homed in the named places; empty
    /// ranks across the whole visible corpus.
    pub async fn entry(&self, anchors: &[Uuid], k: Option<i32>) -> Result<AtlasEntry> {
        let token = self.http.resolve_token()?;
        let path = entry_path(anchors, k);
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// GET /api/graph/traverse — the subgraph reached from the given nodes.
    ///
    /// The walk is **not** confined to any prior result set: the service walks the
    /// reader's whole visible corpus from these seeds.
    pub async fn traverse(&self, seeds: &[Uuid], depth: Option<i32>) -> Result<AtlasSubgraph> {
        let token = self.http.resolve_token()?;
        let path = traverse_path(seeds, depth);
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_path_is_bare_when_nothing_is_named() {
        assert_eq!(entry_path(&[], None), "/api/graph/entry");
    }

    #[test]
    fn entry_path_joins_anchors_with_commas_into_one_param() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        assert_eq!(
            entry_path(&[a, b], None),
            format!("/api/graph/entry?in={a},{b}")
        );
    }

    #[test]
    fn entry_path_carries_k_alone() {
        assert_eq!(entry_path(&[], Some(40)), "/api/graph/entry?k=40");
    }

    #[test]
    fn entry_path_carries_both_params() {
        let a = Uuid::from_u128(3);
        assert_eq!(
            entry_path(&[a], Some(7)),
            format!("/api/graph/entry?in={a}&k=7")
        );
    }

    /// The trap this module's rustdoc names: the page grammar spells seeds as repeated
    /// `from` params and the endpoint spells them comma-joined. Passing the repeated
    /// form through hands the service one unparseable uuid.
    #[test]
    fn traverse_path_joins_seeds_with_commas_not_repeated_params() {
        let a = Uuid::from_u128(4);
        let b = Uuid::from_u128(5);
        let path = traverse_path(&[a, b], None);
        assert_eq!(path, format!("/api/graph/traverse?from={a},{b}"));
        assert!(
            !path.contains(&format!("from={a}&from={b}")),
            "repeated-param form would 400 at the service"
        );
    }

    #[test]
    fn traverse_path_omits_depth_when_unnamed_so_the_default_lives_in_one_place() {
        let a = Uuid::from_u128(6);
        assert_eq!(
            traverse_path(&[a], None),
            format!("/api/graph/traverse?from={a}")
        );
    }

    #[test]
    fn traverse_path_carries_depth_when_named() {
        let a = Uuid::from_u128(8);
        assert_eq!(
            traverse_path(&[a], Some(3)),
            format!("/api/graph/traverse?from={a}&depth=3")
        );
    }
}
