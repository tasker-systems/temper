//! `temper edge` subcommand dispatch.
//!
//! Cloud-mode-only API writes — no vault-file IO. Posts to the relationship
//! endpoints via `temper-client`.

use crate::cli::{CliEdgeKind, CliPolarity, EdgeAction};
use crate::error::Result;
use crate::output;
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::relationship_requests::{
    AssertRelationshipRequest, FoldRelationshipRequest, RelationshipAck, RetypeRelationshipRequest,
    ReweightRelationshipRequest,
};

impl From<CliEdgeKind> for EdgeKind {
    fn from(k: CliEdgeKind) -> Self {
        match k {
            CliEdgeKind::Express => EdgeKind::Express,
            CliEdgeKind::Contains => EdgeKind::Contains,
            CliEdgeKind::LeadsTo => EdgeKind::LeadsTo,
            CliEdgeKind::Near => EdgeKind::Near,
        }
    }
}

impl From<CliPolarity> for Polarity {
    fn from(p: CliPolarity) -> Self {
        match p {
            CliPolarity::Forward => Polarity::Forward,
            CliPolarity::Inverse => Polarity::Inverse,
        }
    }
}

pub fn run(action: EdgeAction) -> Result<()> {
    match action {
        EdgeAction::Assert {
            source,
            target,
            kind,
            polarity,
            label,
            weight,
            act,
        } => {
            let source = temper_workflow::operations::parse_ref(&source)?;
            let target = temper_workflow::operations::parse_ref(&target)?;
            let req = AssertRelationshipRequest {
                source,
                target,
                edge_kind: kind.into(),
                polarity: polarity.into(),
                label,
                weight,
                act: act.into_act_input()?,
            };
            crate::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    let ack = client
                        .relationships()
                        .assert(&req)
                        .await
                        .map_err(crate::commands::client_err)?;
                    print_ack("asserted", &ack);
                    Ok(())
                })
            })
        }
        EdgeAction::Retype {
            edge_handle,
            kind,
            polarity,
        } => {
            let req = RetypeRelationshipRequest {
                edge_kind: kind.into(),
                polarity: polarity.into(),
            };
            crate::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    let ack = client
                        .relationships()
                        .retype(edge_handle, &req)
                        .await
                        .map_err(crate::commands::client_err)?;
                    print_ack("retyped", &ack);
                    Ok(())
                })
            })
        }
        EdgeAction::Reweight {
            edge_handle,
            weight,
        } => {
            let req = ReweightRelationshipRequest { weight };
            crate::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    let ack = client
                        .relationships()
                        .reweight(edge_handle, &req)
                        .await
                        .map_err(crate::commands::client_err)?;
                    print_ack("reweighted", &ack);
                    Ok(())
                })
            })
        }
        EdgeAction::Fold {
            edge_handle,
            reason,
            act,
        } => {
            let req = FoldRelationshipRequest {
                reason,
                act: act.into_act_input()?,
            };
            crate::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    let ack = client
                        .relationships()
                        .fold(edge_handle, &req)
                        .await
                        .map_err(crate::commands::client_err)?;
                    print_ack("folded", &ack);
                    Ok(())
                })
            })
        }
    }
}

fn print_ack(verb: &str, ack: &RelationshipAck) {
    output::success(format!("Relationship {}.", verb));
    output::dim(format!("  edge_handle: {}", ack.edge_handle));
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::cli::{Cli, CliEdgeKind, CliPolarity, Commands, EdgeAction};

    #[test]
    fn edge_assert_parses() {
        let cli = Cli::try_parse_from([
            "temper",
            "edge",
            "assert",
            "source-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "target-019e84ab-26ba-7560-9d34-c60d74a9fbe3",
            "--kind=leads-to",
            "--polarity=inverse",
            "--label=depends_on",
        ])
        .expect("parse should succeed");

        match cli.command {
            Commands::Edge {
                action:
                    EdgeAction::Assert {
                        source,
                        target,
                        kind,
                        polarity,
                        label,
                        weight,
                        ..
                    },
            } => {
                assert_eq!(source, "source-019e84ab-26ba-7560-9d34-c60d74a9fbe2");
                assert_eq!(target, "target-019e84ab-26ba-7560-9d34-c60d74a9fbe3");
                assert_eq!(kind, CliEdgeKind::LeadsTo);
                assert_eq!(polarity, CliPolarity::Inverse);
                assert_eq!(label, "depends_on");
                assert!((weight - 1.0).abs() < f64::EPSILON);
            }
            _ => panic!("expected Commands::Edge / EdgeAction::Assert"),
        }
    }

    #[test]
    fn edge_assert_parses_with_explicit_weight() {
        let cli = Cli::try_parse_from([
            "temper",
            "edge",
            "assert",
            "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "019e84ab-26ba-7560-9d34-c60d74a9fbe3",
            "--kind=near",
            "--polarity=forward",
            "--label=references",
            "--weight=0.5",
        ])
        .expect("parse should succeed");

        match cli.command {
            Commands::Edge {
                action: EdgeAction::Assert { weight, .. },
            } => {
                assert!((weight - 0.5).abs() < f64::EPSILON);
            }
            _ => panic!("expected Commands::Edge / EdgeAction::Assert"),
        }
    }

    #[test]
    fn edge_retype_parses() {
        let edge_handle = uuid::Uuid::nil();
        let cli = Cli::try_parse_from([
            "temper",
            "edge",
            "retype",
            &edge_handle.to_string(),
            "--kind=contains",
            "--polarity=forward",
        ])
        .expect("parse should succeed");

        match cli.command {
            Commands::Edge {
                action:
                    EdgeAction::Retype {
                        edge_handle: cid,
                        kind,
                        polarity,
                    },
            } => {
                assert_eq!(cid, edge_handle);
                assert_eq!(kind, CliEdgeKind::Contains);
                assert_eq!(polarity, CliPolarity::Forward);
            }
            _ => panic!("expected Commands::Edge / EdgeAction::Retype"),
        }
    }

    #[test]
    fn edge_reweight_parses() {
        let edge_handle = uuid::Uuid::nil();
        let cli = Cli::try_parse_from([
            "temper",
            "edge",
            "reweight",
            &edge_handle.to_string(),
            "--weight=2.5",
        ])
        .expect("parse should succeed");

        match cli.command {
            Commands::Edge {
                action:
                    EdgeAction::Reweight {
                        edge_handle: cid,
                        weight,
                    },
            } => {
                assert_eq!(cid, edge_handle);
                assert!((weight - 2.5).abs() < f64::EPSILON);
            }
            _ => panic!("expected Commands::Edge / EdgeAction::Reweight"),
        }
    }

    #[test]
    fn edge_fold_parses() {
        let edge_handle = uuid::Uuid::nil();
        let cli = Cli::try_parse_from([
            "temper",
            "edge",
            "fold",
            &edge_handle.to_string(),
            "--reason=outdated",
        ])
        .expect("parse should succeed");

        match cli.command {
            Commands::Edge {
                action:
                    EdgeAction::Fold {
                        edge_handle: cid,
                        reason,
                        ..
                    },
            } => {
                assert_eq!(cid, edge_handle);
                assert_eq!(reason, Some("outdated".to_string()));
            }
            _ => panic!("expected Commands::Edge / EdgeAction::Fold"),
        }
    }

    #[test]
    fn edge_fold_parses_without_reason() {
        let edge_handle = uuid::Uuid::nil();
        let cli = Cli::try_parse_from(["temper", "edge", "fold", &edge_handle.to_string()])
            .expect("parse should succeed");

        match cli.command {
            Commands::Edge {
                action: EdgeAction::Fold { reason, .. },
            } => {
                assert_eq!(reason, None);
            }
            _ => panic!("expected Commands::Edge / EdgeAction::Fold"),
        }
    }
}
