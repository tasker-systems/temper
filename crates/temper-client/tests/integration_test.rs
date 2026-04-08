//! Integration tests for temper-client.
//!
//! These require a running API and valid auth credentials.
//! Run with: cargo test -p temper-client --features integration-tests
//!
//! The tests skip gracefully when auth credentials are absent or invalid so
//! that `cargo test --all-features` does not fail in environments without a
//! configured temper account.

#![cfg(feature = "integration-tests")]

use temper_client::config::build_client;
use temper_client::error::ClientError;
use temper_client::TemperClient;
use temper_core::types::resource::ResourceListParams;

/// Build a fully-configured client from `~/.config/temper/config.toml`.
///
/// Returns `None` when no auth credentials are present so tests can skip.
fn try_build_client() -> Option<TemperClient> {
    match temper_client::auth::current_token() {
        Ok(_) => {}
        Err(_) => {
            eprintln!(
                "skipping integration test: no valid auth credentials (run `temper auth login`)"
            );
            return None;
        }
    }
    match build_client() {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("skipping integration test: {e}");
            None
        }
    }
}

/// Returns true if the error indicates invalid/expired credentials or
/// an unreachable API — conditions where the test should skip, not fail.
fn should_skip(err: &ClientError) -> bool {
    matches!(
        err,
        ClientError::NotAuthenticated | ClientError::TokenExpired
    )
}

#[tokio::test]
async fn profile_get() {
    let Some(c) = try_build_client() else {
        return;
    };
    match c.profile().get().await {
        Ok(p) => assert!(
            !p.display_name.is_empty(),
            "display_name should not be empty"
        ),
        Err(e) if should_skip(&e) => {
            eprintln!("skipping: {e}");
        }
        Err(e) => panic!("profile get failed: {e}"),
    }
}

#[tokio::test]
async fn resource_list() {
    let Some(c) = try_build_client() else {
        return;
    };
    let params = ResourceListParams {
        kb_context_id: None,
        kb_doc_type_id: None,
        limit: Some(5),
        offset: None,
    };
    match c.resources().list(&params).await {
        Ok(r) => {
            assert!(r.len() <= 5, "should return at most 5 resources");
        }
        Err(e) if should_skip(&e) => {
            eprintln!("skipping: {e}");
        }
        Err(e) => panic!("resource list failed: {e}"),
    }
}

#[tokio::test]
async fn search_query() {
    let Some(c) = try_build_client() else {
        return;
    };
    // Use a zero vector as a smoke-test — we just verify the API call succeeds.
    // A real semantic search would require embedding text via temper-ingest first.
    let embedding = vec![0.0_f32; 768];
    match c.search().query(embedding, None, None, Some(3)).await {
        Ok(rows) => {
            assert!(rows.len() <= 3, "should return at most 3 results");
        }
        Err(e) if should_skip(&e) => {
            eprintln!("skipping: {e}");
        }
        Err(e) => panic!("search failed: {e}"),
    }
}

#[tokio::test]
async fn profile_auth_links() {
    let Some(c) = try_build_client() else {
        return;
    };
    match c.profile().auth_links().await {
        Ok(links) => {
            assert!(
                !links.is_empty(),
                "profile should have at least one auth link"
            );
        }
        Err(e) if should_skip(&e) => {
            eprintln!("skipping: {e}");
        }
        Err(e) => panic!("profile auth_links failed: {e}"),
    }
}
