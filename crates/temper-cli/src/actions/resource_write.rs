//! The CLI's resource writes — create, update, delete, annotate — dispatched through
//! `temper-client`, like every other CLI act.
//!
//! [`connect`] builds the runtime and client a write runs on, and refuses before any network
//! call when no credential resolves. Each write shapes its command into the wire payload (the
//! `shaping` submodule) and calls the one client operation for its act. The server enforces
//! everything else.
//!
//! The writes need the `embed` feature: a create or update embeds its body locally. A build
//! without it refuses all four writes with the same answer, so the four answer alike.

use std::sync::Arc;

use temper_client::TemperClient;
use temper_core::types::resource_view::ResourceView;
use temper_workflow::operations::{
    AnnotateResource, CreateResource, DeleteResource, UpdateResource,
};
use tokio::runtime::Runtime;

use crate::error::{Result, TemperError};

pub(crate) mod shaping;

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

/// Create a resource through `POST /api/ingest`.
///
/// `context_ref` is sent verbatim. A bare name (no `@`/`+` sigil, no UUID) reaches the server's
/// `parse_context_ref`, which rejects it; that server-side rejection is the refusal. A cogmap
/// home sends `home_cogmap_id` and an empty `context_ref` (see the `shaping` submodule).
pub async fn create(
    client: &TemperClient,
    cmd: &CreateResource,
    context_ref: &str,
) -> Result<ResourceView> {
    #[cfg(feature = "embed")]
    {
        let payload = shaping::cmd_to_ingest_payload(cmd, context_ref)?;
        client
            .ingest()
            .create(&payload)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)
    }
    #[cfg(not(feature = "embed"))]
    {
        let _ = (client, cmd, context_ref);
        Err(embed_required())
    }
}

/// Update a resource through `PATCH /api/resources/{id}`.
pub async fn update(client: &TemperClient, cmd: &UpdateResource) -> Result<ResourceView> {
    #[cfg(feature = "embed")]
    {
        let req = shaping::cmd_to_resource_update_request(cmd)?;
        client
            .resources()
            .update(uuid::Uuid::from(cmd.resource), &req)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)
    }
    #[cfg(not(feature = "embed"))]
    {
        let _ = (client, cmd);
        Err(embed_required())
    }
}

/// Delete a resource through `DELETE /api/resources/{id}`.
///
/// The command's act (correlation and authorship) travels as query parameters; the handler
/// reassembles it. A server `SystemAccessRequired` survives `client_err_to_temper` as
/// `TemperError::SystemAccessRequired`, so `main` can render its remedy.
pub async fn delete(client: &TemperClient, cmd: &DeleteResource) -> Result<()> {
    #[cfg(feature = "embed")]
    {
        let act: temper_core::types::ActInput = cmd.act.clone().into();
        client
            .resources()
            .delete(uuid::Uuid::from(cmd.resource), &act)
            .await
            // The CLI answers from the row it read before the delete, not from this response.
            .map(drop)
            .map_err(crate::actions::runtime::client_err_to_temper)
    }
    #[cfg(not(feature = "embed"))]
    {
        let _ = (client, cmd);
        Err(embed_required())
    }
}

/// Attach provenance sources to a resource's block without a body revise, through
/// `POST /api/resources/{id}/annotate`.
pub async fn annotate(client: &TemperClient, cmd: &AnnotateResource) -> Result<ResourceView> {
    #[cfg(feature = "embed")]
    {
        let req = shaping::cmd_to_resource_annotate_request(cmd);
        client
            .resources()
            .annotate(uuid::Uuid::from(cmd.resource), &req)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)
    }
    #[cfg(not(feature = "embed"))]
    {
        let _ = (client, cmd);
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
        let cmd = DeleteResource {
            resource: temper_core::types::ids::ResourceId::new(),
            force: false,
            act: Default::default(),
            origin: Surface::CliCloud,
        };
        let err = delete(&client, &cmd).await.unwrap_err();
        assert!(format!("{err:?}").contains("--features embed"));
    }
}
