//! `temper blob` subcommand dispatch.
//!
//! Cloud-mode-only API writes/reads — no vault-file IO. `put`'s threshold decision and
//! segmentation live in [`crate::actions::blob`]; this file parses flags, resolves the
//! home, and renders.

use std::io::{IsTerminal, Write};

use crate::cli::BlobAction;
use crate::error::Result;
use crate::format::OutputFormat;
use crate::output;
use temper_core::error::TemperError;
use temper_core::types::blob::BlobRelationDirection;

pub fn run(action: BlobAction, fmt: OutputFormat) -> Result<()> {
    match action {
        BlobAction::Put {
            file,
            home,
            home_table,
            content_type,
            filename,
        } => crate::actions::runtime::with_client(|client| {
            Box::pin(async move {
                let resolved =
                    crate::actions::blob::resolve_home(client, &home, home_table).await?;
                let params = crate::actions::blob::prepare_put(
                    &file,
                    content_type.as_deref(),
                    filename.as_deref(),
                )?;
                let bytes_len = params.bytes.len() as u64;
                let threshold = crate::actions::blob::single_request_max_bytes()?;
                let segmented = bytes_len > threshold as u64;
                let resp = crate::actions::blob::put(client, resolved, &params, |k, n, staged| {
                    output::plain_err(format!("segment {k}/{n} — {staged} bytes staged"));
                })
                .await?;
                if segmented {
                    output::plain_err(format!(
                        "committed {bytes_len} bytes over {n_total} segments",
                        n_total = (bytes_len as usize).div_ceil(threshold)
                    ));
                }
                let rendered = crate::format::render(&resp, fmt)?;
                output::plain(rendered);
                Ok(())
            })
        }),
        BlobAction::Get { blob, out } => crate::actions::runtime::with_client(|client| {
            Box::pin(async move {
                let mut resp = client
                    .blobs()
                    .read_response(blob)
                    .await
                    .map_err(crate::actions::runtime::client_err_to_temper)?;
                // Byte transfer — the output-format flags do not apply; the bytes ARE the
                // answer. A TTY stdout without --out would smear binary into the terminal,
                // which is never what was asked for: refuse with the remedy, don't dump.
                match out {
                    Some(path) => {
                        let mut file = std::fs::File::create(&path).map_err(|e| {
                            TemperError::Config(format!("create {}: {e}", path.display()))
                        })?;
                        stream_to(&mut resp, &mut file).await?;
                        output::plain_err(format!("wrote {}", path.display()));
                    }
                    None if std::io::stdout().is_terminal() => {
                        return Err(TemperError::Config(
                            "refusing to write binary to the terminal — pass -o <path> or pipe \
                             stdout"
                                .to_string(),
                        ));
                    }
                    None => {
                        let mut stdout = std::io::stdout().lock();
                        stream_to(&mut resp, &mut stdout).await?;
                    }
                }
                Ok(())
            })
        }),
        BlobAction::List { home, home_table } => crate::actions::runtime::with_client(|client| {
            Box::pin(async move {
                let home = match home.as_deref() {
                    None => None,
                    Some(h) => {
                        let resolved =
                            crate::actions::blob::resolve_home(client, h, home_table).await?;
                        Some((resolved.table, resolved.id))
                    }
                };
                let rows = client
                    .blobs()
                    .list(home)
                    .await
                    .map_err(crate::actions::runtime::client_err_to_temper)?;
                let rendered = crate::format::render(&rows, fmt)?;
                output::plain(rendered);
                Ok(())
            })
        }),
        BlobAction::Relate {
            blob,
            to,
            peer_table,
            direction,
            kind,
            polarity,
            label,
            weight,
            act,
        } => {
            let (peer_table, peer_id) = crate::actions::blob::resolve_peer(&to, peer_table)?;
            let req = temper_core::types::blob::BlobRelationAssertRequest {
                direction: BlobRelationDirection::from(direction),
                peer_table: peer_table.to_string(),
                peer_id,
                edge_kind: kind.into(),
                polarity: polarity.into(),
                label,
                weight,
                act: act.into_act_input()?,
            };
            crate::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    let ack = client
                        .blobs()
                        .relate(blob, &req)
                        .await
                        .map_err(crate::actions::runtime::client_err_to_temper)?;
                    let rendered = crate::format::render(&ack, fmt)?;
                    output::plain(rendered);
                    Ok(())
                })
            })
        }
    }
}

/// Pump the response body in chunks — a blob read-through must not require the whole
/// blob in memory (that is the point of streaming, D6). The writer stays sync; the
/// chunks arrive off the async body.
async fn stream_to(resp: &mut reqwest::Response, writer: &mut impl Write) -> Result<u64> {
    let mut total = 0u64;
    loop {
        let chunk = resp
            .chunk()
            .await
            .map_err(|e| TemperError::Network(format!("blob read transfer failed: {e}")))?;
        let Some(chunk) = chunk else { return Ok(total) };
        writer
            .write_all(&chunk)
            .map_err(|e| TemperError::Network(format!("blob write failed: {e}")))?;
        total += chunk.len() as u64;
    }
}
