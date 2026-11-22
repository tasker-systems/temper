//! D1.1's checksum, re-implemented from the spec's text, for the erasure tests' fixtures.

use sha2::Digest;
use temper_substrate::payloads::{ErasureMapEntry, ErasureMemberTable};
use uuid::Uuid;

fn table_name(table: ErasureMemberTable) -> String {
    serde_json::to_value(table)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

/// D1.1, re-implemented from the spec's text and independently of `erasure_map_checksum`, so
/// a test can compare the two. The header names the erasure and its manifest digest; each line is
/// one declared member, tab-separated, an absent parent or prior erasure written as the empty
/// string; lines sort by member table as bytes (`COLLATE "C"`), then by member id (a uuid compares
/// as its bytes in both Postgres and `uuid::Uuid`). Every separator is a single tab or newline
/// byte.
pub fn d1_1_checksum(erasure: Uuid, digest: &str, map: &[ErasureMapEntry]) -> String {
    let mut lines: Vec<(String, Uuid, String)> = map
        .iter()
        .map(|e| {
            let table = table_name(e.table);
            let (parent_table, parent_id) = match e.parent {
                Some(p) => (table_name(p.table), p.id.to_string()),
                None => (String::new(), String::new()),
            };
            let prior = e.prior_erasure.map(|p| p.to_string()).unwrap_or_default();
            let line = format!("{table}\t{}\t{parent_table}\t{parent_id}\t{prior}\n", e.id);
            (table, e.id, line)
        })
        .collect();
    lines.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()).then(a.1.cmp(&b.1)));
    let mut text = format!("temper-erasure-map/v1\t{erasure}\t{digest}\n");
    for (_, _, line) in lines {
        text.push_str(&line);
    }
    hex::encode(sha2::Sha256::digest(text.as_bytes()))
}
