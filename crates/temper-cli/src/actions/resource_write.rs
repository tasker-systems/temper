//! The CLI's resource writes — create, update, delete, annotate — dispatched through
//! `temper-client`, like every other CLI act.
//!
//! [`connect`] builds the runtime and client a write runs on, and refuses before any network
//! call when no credential resolves. Each write shapes its request with `temper_workflow::shaping`,
//! the one function per act MCP shapes through too, so the same inputs send the same request from
//! either door. The server enforces everything else.
//!
//! The one step the CLI adds is embedding: a create or update with a body sends the body's chunks
//! and vectors, computed locally. So the writes need the `embed` feature; a build without it
//! refuses all four with the same answer.

use std::sync::Arc;

use temper_client::TemperClient;
use temper_core::types::resource_view::ResourceView;
use temper_core::types::ActInput;
use temper_workflow::shaping::{AnnotateInput, CreateInput, UpdateInput};
use tokio::runtime::Runtime;

use crate::error::{Result, TemperError};

/// Build the tokio runtime and the authenticated client a CLI write dispatches through.
///
/// There is no offline path. With no `TEMPER_TOKEN`, no token cached on disk, and no ambient
/// credential (`TEMPER_AUTH=ambient`, attached in transit), this refuses with a pointer to
/// `temper auth login` before any request is sent.
pub fn connect() -> Result<(Runtime, Arc<TemperClient>)> {
    let runtime = Runtime::new().map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;

    let (_cfg, store, client) = crate::actions::runtime::build_config_store_and_client()?;

    // An ambient client has no token by design; the server's answer is its check.
    if !client.is_ambient() && store.load().ok().flatten().is_none() {
        return Err(TemperError::Config(
            "not authenticated — run `temper auth login` first".into(),
        ));
    }

    Ok((runtime, Arc::new(client)))
}

/// A shaping failure, in the CLI's error vocabulary.
#[cfg(feature = "embed")]
pub(crate) fn shape_err(e: temper_workflow::shaping::ShapeError) -> TemperError {
    TemperError::Project(e.to_string())
}

/// Create a resource through `POST /api/ingest`.
pub async fn create(client: &TemperClient, input: CreateInput) -> Result<ResourceView> {
    #[cfg(feature = "embed")]
    {
        let mut payload = temper_workflow::shaping::shape_create(input).map_err(shape_err)?;
        if !payload.content.is_empty() {
            let chunks = crate::actions::ingest::compute_body_chunks(&payload.content)?;
            payload.content_hash = Some(chunks.content_hash);
            payload.chunks_packed = Some(chunks.chunks_packed);
        }
        client
            .ingest()
            .create(&payload)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)
    }
    #[cfg(not(feature = "embed"))]
    {
        let _ = (client, input);
        Err(embed_required())
    }
}

/// Update a resource through `PATCH /api/resources/{id}`.
pub async fn update(
    client: &TemperClient,
    id: uuid::Uuid,
    input: UpdateInput,
) -> Result<ResourceView> {
    #[cfg(feature = "embed")]
    {
        let mut req = temper_workflow::shaping::shape_update(input).map_err(shape_err)?;
        if let Some(content) = req.content.as_deref() {
            let chunks = crate::actions::ingest::compute_body_chunks(content)?;
            req.content_hash = Some(chunks.content_hash);
            req.chunks_packed = Some(chunks.chunks_packed);
        }
        client
            .resources()
            .update(id, &req)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)
    }
    #[cfg(not(feature = "embed"))]
    {
        let _ = (client, id, input);
        Err(embed_required())
    }
}

/// Delete a resource through `DELETE /api/resources/{id}`.
///
/// Delete has no shaping: its act (correlation and authorship) travels as query parameters,
/// exactly as given. A server `SystemAccessRequired` survives `client_err_to_temper` as
/// `TemperError::SystemAccessRequired`, so `main` can render its remedy.
pub async fn delete(client: &TemperClient, id: uuid::Uuid, act: &ActInput) -> Result<()> {
    #[cfg(feature = "embed")]
    {
        client
            .resources()
            .delete(id, act)
            .await
            // The CLI answers from the row it read before the delete, not from this response.
            .map(drop)
            .map_err(crate::actions::runtime::client_err_to_temper)
    }
    #[cfg(not(feature = "embed"))]
    {
        let _ = (client, id, act);
        Err(embed_required())
    }
}

/// Attach provenance sources to a resource's block without a body revise, through
/// `POST /api/resources/{id}/annotate`.
pub async fn annotate(
    client: &TemperClient,
    id: uuid::Uuid,
    input: AnnotateInput,
) -> Result<ResourceView> {
    #[cfg(feature = "embed")]
    {
        let req = temper_workflow::shaping::shape_annotate(input);
        client
            .resources()
            .annotate(id, &req)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)
    }
    #[cfg(not(feature = "embed"))]
    {
        let _ = (client, id, input);
        Err(embed_required())
    }
}

/// The refusal every resource write gives in a build without the `embed` feature.
#[cfg(not(feature = "embed"))]
fn embed_required() -> TemperError {
    TemperError::BadRequest("cloud mode requires --features embed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_refuses_without_a_token() {
        let temp = tempfile::tempdir().unwrap();
        let auth_path = temp.path().join("auth.json");
        let nonexistent_config = temp.path().join("no-such-config.toml");

        temp_env::with_vars(
            [
                ("TEMPER_TOKEN", None::<&str>),
                ("TEMPER_AUTH", None),
                ("TEMPER_AUTH_PATH", Some(auth_path.to_str().unwrap())),
                (
                    "TEMPER_GLOBAL_CONFIG",
                    Some(nonexistent_config.to_str().unwrap()),
                ),
                // A valid URL, so the unconfigured-cloud refusal does not shadow the token
                // check — both fire at client construction, URL first.
                ("TEMPER_API_URL", Some("https://temper.test")),
            ],
            || match connect() {
                Ok(_) => panic!("expected Err when no token, got Ok"),
                Err(err) => assert!(
                    format!("{err:?}").contains("temper auth login"),
                    "expected auth-login error, got: {err:?}"
                ),
            },
        );
    }

    #[cfg(not(feature = "embed"))]
    #[tokio::test]
    async fn writes_refuse_without_embed() {
        use temper_client::auth::MemoryTokenStore;
        use temper_workflow::operations::Surface;

        let store = Arc::new(MemoryTokenStore::empty());
        let client = TemperClient::new("http://localhost:0", None, Surface::CliCloud, store)
            .expect("loopback test URL validates");
        let err = delete(&client, uuid::Uuid::now_v7(), &ActInput::default())
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("--features embed"));
    }
}
