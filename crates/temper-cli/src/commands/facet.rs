//! `temper resource facet` subcommand dispatch.
//!
//! Cloud-mode-only API write — no vault-file IO. Posts to the facet
//! endpoint via `temper-client`.

use crate::cli::ActArgs;
use crate::error::Result;
use crate::format::OutputFormat;
use crate::output;
use temper_core::types::facet_requests::{FacetAck, FacetSetRequest};

/// Run `temper resource facet <ref> --values <json> [--weight <f64>]`.
pub fn run(
    r#ref: String,
    values: String,
    weight: Option<f64>,
    act: ActArgs,
    fmt: OutputFormat,
) -> Result<()> {
    let resource = temper_workflow::operations::parse_ref(&r#ref)?;
    let values: serde_json::Value = serde_json::from_str(&values)
        .map_err(|e| crate::error::TemperError::Project(format!("invalid --values JSON: {e}")))?;
    let req = FacetSetRequest {
        resource: resource.0,
        values,
        weight: weight.unwrap_or(1.0),
        act: act.into_act_input()?,
    };
    crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            let ack = client
                .facets()
                .set(&req)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            print_ack(&ack, fmt)?;
            Ok(())
        })
    })
}

fn print_ack(ack: &FacetAck, fmt: OutputFormat) -> Result<()> {
    match fmt {
        OutputFormat::Json => {
            let rendered = crate::format::render(ack, fmt)?;
            output::plain(rendered);
        }
        OutputFormat::Toon => {
            output::success("Facet set.".to_string());
            output::dim(format!("  property_id: {}", ack.property_id));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::cli::{Cli, Commands, ResourceAction};

    #[test]
    fn resource_facet_parses() {
        let cli = Cli::try_parse_from([
            "temper",
            "resource",
            "facet",
            "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "--values={\"summary\":\"example\"}",
        ])
        .expect("parse should succeed");

        match cli.command {
            Commands::Resource {
                action:
                    ResourceAction::Facet {
                        r#ref,
                        values,
                        weight,
                        ..
                    },
            } => {
                assert_eq!(r#ref, "019e84ab-26ba-7560-9d34-c60d74a9fbe2");
                assert_eq!(values, "{\"summary\":\"example\"}");
                assert_eq!(weight, None);
            }
            _ => panic!("expected Commands::Resource / ResourceAction::Facet"),
        }
    }

    #[test]
    fn resource_facet_parses_with_explicit_weight() {
        let cli = Cli::try_parse_from([
            "temper",
            "resource",
            "facet",
            "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "--values={\"summary\":\"example\"}",
            "--weight=0.5",
        ])
        .expect("parse should succeed");

        match cli.command {
            Commands::Resource {
                action: ResourceAction::Facet { weight, .. },
            } => {
                assert_eq!(weight, Some(0.5));
            }
            _ => panic!("expected Commands::Resource / ResourceAction::Facet"),
        }
    }
}
