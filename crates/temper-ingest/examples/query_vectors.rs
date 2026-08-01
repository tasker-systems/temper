//! Emit the query embedding the CLI would compute, as a pgvector literal.
//!
//! **Why this exists.** Wayfind's Stage-1 tuning constants (κ, α/β, the anchor prior) are
//! SQL-resident and server-side, so the only way to run a *counterfactual* over them is to
//! recompute `wayfind_region_scores`' CTEs in SQL with the constant varied. That needs a query
//! vector — and until now the only vectors available to such a probe were `kb_chunks.embedding`
//! rows (document vectors), because nothing exposes the vector the client computes for a real
//! natural-language query. Document vectors and query vectors do not sit the same way against
//! region centroids, so a counterfactual computed on the former cannot speak for what a caller
//! gets. This closes that gap: embed the query text with the shipped model, print the vector,
//! feed it to the same differential SQL.
//!
//! It is the *same* code path the CLI uses. `temper_cli::actions::search::embed_query` is a
//! one-line delegate to `temper_ingest::embed::embed_text` (`crates/temper-cli/src/actions/
//! search.rs:12`), and the e2e test `server_query_embedding_matches_cli_embed_query`
//! (`tests/e2e/tests/server_query_embed_test.rs:148`) pins server and CLI to that one function.
//! No BGE "represent this sentence…" query prefix is applied, matching how the corpus was
//! ingested.
//!
//! Reads one query per line on stdin; writes `query<TAB>[v1,v2,…]` per line on stdout, so the
//! output pipes straight into `psql -v`. Blank lines and `#` comments are skipped.
//!
//! ```bash
//! TEMPER_ONNX_MODEL_PATH=crates/temper-ingest/models/bge-base-en-v1.5/model_quantized.onnx \
//!   cargo run --release -p temper-ingest --no-default-features --features embed-download \
//!   --example query_vectors < queries.txt > vectors.tsv
//! ```

use std::io::{BufRead, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `--batch` embeds every input in ONE `embed_texts` call — the path the INGEST pipeline uses
    // (`prepare_block` batches a block's chunks). Default is one `embed_text` per line, the path
    // the QUERY takes. The two are worth being able to compare: batching pads sequences to the
    // batch maximum, and under a quantized model that is not guaranteed to be numerically
    // identical to embedding the same string alone.
    let batch = std::env::args().any(|a| a == "--batch");

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let queries: Vec<String> = stdin
        .lock()
        .lines()
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    let batched: Option<Vec<Vec<f32>>> = if batch {
        let refs: Vec<&str> = queries.iter().map(String::as_str).collect();
        Some(temper_ingest::embed::embed_texts(&refs)?)
    } else {
        None
    };

    for (i, query) in queries.iter().enumerate() {
        let query = query.as_str();
        let embedding = match &batched {
            Some(all) => all[i].clone(),
            None => temper_ingest::embed::embed_text(query)?,
        };

        // pgvector literal: `[0.1,0.2,…]`. Full f32 precision — a truncated vector is a
        // different vector, and this feeds a margin comparison measured in thousandths.
        let mut literal = String::with_capacity(embedding.len() * 12);
        literal.push('[');
        for (i, v) in embedding.iter().enumerate() {
            if i > 0 {
                literal.push(',');
            }
            literal.push_str(&v.to_string());
        }
        literal.push(']');

        writeln!(out, "{query}\t{literal}")?;
        out.flush()?;
    }

    Ok(())
}
