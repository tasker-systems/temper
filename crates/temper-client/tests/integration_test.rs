//! Integration tests for temper-client.
//!
//! These require a running API and valid auth credentials.
//! Run with: cargo test -p temper-client --features integration-tests
//!
//! The tests skip gracefully when no auth credentials are present so that
//! `cargo test --all-features` does not fail in environments without a
//! configured temper account.

#![cfg(feature = "integration-tests")]

use temper_client::config::build_client;
use temper_client::TemperClient;
use temper_core::types::api::SearchParams;
use temper_core::types::resource::ResourceListParams;

/// Build a fully-configured client from `~/.config/temper/config.toml`.
///
/// Returns `None` when no auth credentials are present so tests can skip.
fn try_build_client() -> Option<TemperClient> {
    // Check whether stored auth exists before attempting to make calls.
    match temper_client::auth::current_token() {
        Ok(_) => {}
        Err(_) => {
            eprintln!("skipping integration test: no auth credentials (run `temper auth login`)");
            return None;
        }
    }
    Some(build_client().expect("failed to build client from config"))
}

#[tokio::test]
async fn profile_get() {
    let Some(c) = try_build_client() else {
        return;
    };
    let profile = c.profile().get().await;
    match profile {
        Ok(p) => assert!(
            !p.display_name.is_empty(),
            "display_name should not be empty"
        ),
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
        limit: Some(5),
        offset: None,
    };
    let resources = c.resources().list(&params).await;
    match resources {
        Ok(r) => {
            // Verify the call succeeded — may return an empty list.
            assert!(r.len() <= 5, "should return at most 5 resources");
        }
        Err(e) => panic!("resource list failed: {e}"),
    }
}

#[tokio::test]
async fn search_query() {
    let Some(c) = try_build_client() else {
        return;
    };
    let params = SearchParams {
        q: "test".into(),
        kb_context_id: None,
        limit: Some(3),
    };
    let results = c.search().query(&params).await;
    assert!(results.is_ok(), "search failed: {:?}", results.err());
    let rows = results.unwrap();
    assert!(rows.len() <= 3, "should return at most 3 results");
}

#[tokio::test]
async fn profile_auth_links() {
    let Some(c) = try_build_client() else {
        return;
    };
    let links = c.profile().auth_links().await;
    match links {
        Ok(links) => {
            // A freshly created profile always has at least one auth link.
            assert!(
                !links.is_empty(),
                "profile should have at least one auth link"
            );
        }
        Err(e) => panic!("profile auth_links failed: {e}"),
    }
}
