//! No server command learns what its own write did by reading back the events its transaction
//! appended (goal clause `a-write-learns-its-outcome-from-the-write`). The write returns what its
//! caller needs: a move's departed home, for instance, is `writes::UpdateOutcome::left_home`.
//!
//! The tell of such a read-back is a `kb_events` query keyed on the current transaction id, so this
//! scans every source file under `src/` for that function. A source scan, not a DB test: it needs
//! no database and runs in the unit job. It does not catch a read-back keyed some other way (by
//! event id, by correlation id); that half of the clause is review's.

use std::path::{Path, PathBuf};

/// Built from parts so this file does not match itself if the scan ever widens to `tests/`.
fn own_transaction_id() -> String {
    ["pg_current", "_xact_id"].concat()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_service_source_reads_events_by_its_own_transaction_id() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(!files.is_empty(), "scanned nothing under {}", src.display());
    let needle = own_transaction_id();
    let offenders: Vec<String> = files
        .iter()
        .filter(|f| {
            std::fs::read_to_string(f)
                .expect("read source")
                .contains(&needle)
        })
        .map(|f| f.strip_prefix(&src).unwrap_or(f).display().to_string())
        .collect();
    assert!(
        offenders.is_empty(),
        "these files query by the current transaction id, the shape of a write reading back its \
         own events; return what the caller needs from the write instead: {offenders:?}"
    );
}
