//! `temper edge` subcommand dispatch.
//!
//! Cloud-mode-only API writes — no vault-file IO. Posts to the relationship
//! endpoints via `temper-client`.

use crate::cli::{CliEdgeKind, CliPolarity, EdgeAction};
use crate::error::Result;
use crate::output;
use temper_core::operations::ResourceRef;
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
        } => {
            let source = ResourceRef::Uuid {
                id: temper_core::operations::parse_ref(&source)?,
            };
            let target = temper_core::operations::parse_ref(&target)?;
            let req = AssertRelationshipRequest {
                source,
                target,
                edge_kind: kind.into(),
                polarity: polarity.into(),
                label,
                weight,
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
            correlation_id,
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
                        .retype(correlation_id, &req)
                        .await
                        .map_err(crate::commands::client_err)?;
                    print_ack("retyped", &ack);
                    Ok(())
                })
            })
        }
        EdgeAction::Reweight {
            correlation_id,
            weight,
        } => {
            let req = ReweightRelationshipRequest { weight };
            crate::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    let ack = client
                        .relationships()
                        .reweight(correlation_id, &req)
                        .await
                        .map_err(crate::commands::client_err)?;
                    print_ack("reweighted", &ack);
                    Ok(())
                })
            })
        }
        EdgeAction::Fold {
            correlation_id,
            reason,
        } => {
            let req = FoldRelationshipRequest { reason };
            crate::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    let ack = client
                        .relationships()
                        .fold(correlation_id, &req)
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
    output::dim(format!("  correlation_id: {}", ack.correlation_id));
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
        let correlation_id = uuid::Uuid::nil();
        let cli = Cli::try_parse_from([
            "temper",
            "edge",
            "retype",
            &correlation_id.to_string(),
            "--kind=contains",
            "--polarity=forward",
        ])
        .expect("parse should succeed");

        match cli.command {
            Commands::Edge {
                action:
                    EdgeAction::Retype {
                        correlation_id: cid,
                        kind,
                        polarity,
                    },
            } => {
                assert_eq!(cid, correlation_id);
                assert_eq!(kind, CliEdgeKind::Contains);
                assert_eq!(polarity, CliPolarity::Forward);
            }
            _ => panic!("expected Commands::Edge / EdgeAction::Retype"),
        }
    }

    #[test]
    fn edge_reweight_parses() {
        let correlation_id = uuid::Uuid::nil();
        let cli = Cli::try_parse_from([
            "temper",
            "edge",
            "reweight",
            &correlation_id.to_string(),
            "--weight=2.5",
        ])
        .expect("parse should succeed");

        match cli.command {
            Commands::Edge {
                action:
                    EdgeAction::Reweight {
                        correlation_id: cid,
                        weight,
                    },
            } => {
                assert_eq!(cid, correlation_id);
                assert!((weight - 2.5).abs() < f64::EPSILON);
            }
            _ => panic!("expected Commands::Edge / EdgeAction::Reweight"),
        }
    }

    #[test]
    fn edge_fold_parses() {
        let correlation_id = uuid::Uuid::nil();
        let cli = Cli::try_parse_from([
            "temper",
            "edge",
            "fold",
            &correlation_id.to_string(),
            "--reason=outdated",
        ])
        .expect("parse should succeed");

        match cli.command {
            Commands::Edge {
                action:
                    EdgeAction::Fold {
                        correlation_id: cid,
                        reason,
                    },
            } => {
                assert_eq!(cid, correlation_id);
                assert_eq!(reason, Some("outdated".to_string()));
            }
            _ => panic!("expected Commands::Edge / EdgeAction::Fold"),
        }
    }

    #[test]
    fn edge_fold_parses_without_reason() {
        let correlation_id = uuid::Uuid::nil();
        let cli = Cli::try_parse_from(["temper", "edge", "fold", &correlation_id.to_string()])
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
