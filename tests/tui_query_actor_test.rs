use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use temper_cli::tui::query_actor::{spawn_query_actor, QueryRequest, QueryResult};

#[tokio::test]
async fn test_query_actor_handles_search_request() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let (req_tx, req_rx) = mpsc::channel::<QueryRequest>(8);
    let (res_tx, mut res_rx) = mpsc::channel::<QueryResult>(8);

    let _handle = spawn_query_actor(config, req_rx, res_tx);

    req_tx
        .send(QueryRequest::Search {
            query: "test query".to_string(),
        })
        .await
        .unwrap();

    let result = timeout(Duration::from_secs(30), res_rx.recv())
        .await
        .expect("timed out waiting for query result")
        .expect("channel closed before receiving result");

    assert!(
        matches!(result, QueryResult::SearchResults(_)),
        "expected SearchResults variant, got: {:?}",
        result
    );
}
