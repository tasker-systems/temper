//! `temper auditor dispatch|complete|sweep` — the citation-audit worker's doors, one server call
//! each, the answer rendered as the server gave it.

use crate::cli::AuditorCmd;
use crate::error::Result;
use crate::format::OutputFormat;

pub fn run(cmd: AuditorCmd, fmt: OutputFormat) -> Result<()> {
    match cmd {
        AuditorCmd::Dispatch {
            cap,
            correlation_id,
        } => {
            let request = temper_core::types::auditor::AuditorDispatchTickRequest { cap };
            crate::actions::runtime::render_read(fmt, move |client| {
                Box::pin(async move { client.auditor().dispatch(&request, correlation_id).await })
            })
        }
        AuditorCmd::Complete { cogmap } => {
            let cogmap = temper_workflow::operations::parse_ref(&cogmap)?.0;
            crate::actions::runtime::render_read(fmt, move |client| {
                Box::pin(async move { client.auditor().complete(cogmap).await })
            })
        }
        AuditorCmd::Sweep { cap } => {
            let query = temper_core::types::query_params::SweepQuery { cap };
            crate::actions::runtime::render_read(fmt, move |client| {
                Box::pin(async move { client.auditor().sweep(&query).await })
            })
        }
    }
}
