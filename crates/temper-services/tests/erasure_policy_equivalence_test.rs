//! The TOML erasure-policy manifest says exactly what the three `.txt` manifests say. Temporary:
//! it exists to prove the conversion, and goes with the `.txt` files.
//!
//! Each module below holds a copy of one old test's line parser, verbatim, reading the old `.txt`
//! file, and compares what it parses, field by field and notes included, with the matching part of
//! [`temper_services::erasure_policy::in_force`]. Both sides are projected to strings, so a
//! difference reads as text, and every difference is listed at once rather than the first.
//!
//! The parsers are copied, not shared, because they are the old meaning: the test is a proof only
//! if the old side is read exactly as the old tests read it. Two changes to the copies, each marked
//! where it is made: the personal-data parser keeps the note it used to drop (`cols[4]`), and the
//! sensitivity header count is read by the old test's expression, lifted into a function.

use std::collections::BTreeMap;
use std::fmt::Debug;

use temper_core::erasure_policy::{
    Achieved, Ceiling, Claim, Class, Disposition, PayloadDisposition, Qualifier, ScanDisposition,
};
use temper_services::erasure_policy::in_force;

/// Assert two maps are equal, listing every key on one side only and every value that differs.
fn assert_same<K: Ord + Debug, V: PartialEq + Debug>(
    what: &str,
    old: &BTreeMap<K, V>,
    new: &BTreeMap<K, V>,
) {
    let mut problems = Vec::new();
    for (key, old_value) in old {
        match new.get(key) {
            None => problems.push(format!("only in the .txt: {key:?} = {old_value:?}")),
            Some(new_value) if new_value != old_value => problems.push(format!(
                "differs at {key:?}:\n    .txt:  {old_value:?}\n    .toml: {new_value:?}"
            )),
            Some(_) => {}
        }
    }
    for (key, new_value) in new {
        if !old.contains_key(key) {
            problems.push(format!("only in the .toml: {key:?} = {new_value:?}"));
        }
    }
    assert!(
        problems.is_empty(),
        "{what}: the .toml manifest and the .txt manifest disagree ({} of {} .txt entries, {} \
         .toml entries):\n{}",
        problems.len(),
        old.len(),
        new.len(),
        problems.join("\n")
    );
}

fn claim_text(claim: &Claim) -> String {
    match claim {
        Claim::Erased { arm } => format!("erased:{arm}"),
        Claim::Deleted { arm } => format!("deleted:{arm}"),
        Claim::Cascaded { arm, parent } => format!("cascaded:{arm}:{parent}"),
        Claim::ResourceErasure { step } => format!("resource-erasure:D2.{step}"),
    }
}

fn achieved_text(achieved: &Achieved) -> String {
    match achieved {
        Achieved::Unstated => "-".to_owned(),
        Achieved::Claims(claims) => claims
            .iter()
            .map(claim_text)
            .collect::<Vec<_>>()
            .join(" + "),
        Achieved::Remainder(reason) => format!("remainder: {reason}"),
    }
}

fn class_text(class: Class) -> &'static str {
    match class {
        Class::Identifier => "identifier",
        Class::Reference => "reference",
        Class::Content => "content",
        Class::Derived => "derived",
        Class::Incidental => "incidental",
        Class::Discriminator => "discriminator",
        Class::None => "none",
    }
}

fn ceiling_text(ceiling: Ceiling) -> &'static str {
    match ceiling {
        Ceiling::Full => "full",
        Ceiling::Recompute => "recompute",
        Ceiling::Pseudonym => "pseudonym",
        Ceiling::None => "none",
        Ceiling::NotApplicable => "n-a",
    }
}

fn scan_disposition_text(disposition: ScanDisposition) -> &'static str {
    match disposition {
        ScanDisposition::Scan => "scan",
        ScanDisposition::Structural => "structural",
        ScanDisposition::Credential => "credential",
        ScanDisposition::OutOfScope => "out-of-scope",
    }
}

fn table_disposition_text(disposition: &Disposition) -> String {
    match disposition {
        Disposition::Handled { step } => format!("handled:D2.{step}"),
        Disposition::Repointed { step, via } => format!("repointed:D2.{step}:{via}"),
        Disposition::OutOfScope => "out-of-scope".to_owned(),
    }
}

fn payload_disposition_text(disposition: &PayloadDisposition) -> String {
    match disposition {
        PayloadDisposition::Redact(class) => format!("redact:{class}"),
        PayloadDisposition::Structural => "structural".to_owned(),
        PayloadDisposition::OutOfScope => "out-of-scope".to_owned(),
        PayloadDisposition::Permissive => "permissive".to_owned(),
    }
}

/// A refinement's `where`, in the old `[key=…,owner=…]` spelling.
fn qualifier_text(qualifier: &Qualifier) -> String {
    let mut parts = Vec::new();
    if let Some(key) = &qualifier.key {
        parts.push(format!("key={key}"));
    }
    if let Some(owner) = &qualifier.owner {
        parts.push(format!("owner={owner}"));
    }
    parts.join(",")
}

/// `personal_data_surface_test.rs`'s parser, verbatim but for the kept note.
mod personal {
    use std::collections::BTreeMap;

    use super::{achieved_text, assert_same, ceiling_text, class_text, in_force};

    const MANIFEST: &str = include_str!("../../../scripts/personal-data-surface.txt");

    const CLASSES: &[&str] = &[
        "identifier",
        "reference",
        "content",
        "derived",
        "incidental",
        "discriminator",
        "none",
    ];
    const CEILINGS: &[&str] = &["full", "recompute", "pseudonym", "none", "n-a"];

    /// The person act's arms, as its body's comments number them (20261023100000). A label for the
    /// reader: comments are dropped before binding, so the test checks only that a line names one.
    const ARMS: &[&str] = &[
        "1", "2", "3", "4", "4a", "5", "6", "6a", "7", "7a", "8", "9", "9a", "10", "11", "12",
    ];

    /// The D2 steps a `resource-erasure:` claim may cite: `resource-erasure-surface.txt`'s numbering.
    const D2_STEPS: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "7a", "8", "9", "9f"];

    /// One way the person act reaches a column. Each is bound to a statement in the live act.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Claim {
        /// `_erasure_apply_redaction` assigns the column an erasure value.
        Erased { arm: String },
        /// `_erasure_apply_redaction` deletes from the column's table.
        Deleted { arm: String },
        /// `_erasure_apply_redaction` deletes from `parent`, which cascades to the column's table.
        Cascaded { arm: String, parent: String },
        /// The resource erasures the act runs over the estate, whose manifest handles the column.
        ResourceErasure { step: String },
    }

    /// What a line says the person act achieves.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Achieved {
        /// `-`: the line's ceiling is not `full`, so there is nothing to achieve.
        Unstated,
        Claims(Vec<Claim>),
        /// `remainder: <reason>`: the act does not reach it.
        Remainder(String),
    }

    #[derive(Debug, Clone)]
    struct Declaration {
        class: String,
        ceiling: String,
        achieved: Achieved,
        /// CHANGED from the copied parser: it dropped the note; the proof needs it.
        note: String,
    }

    fn parse_arm(arm: &str, lineno: usize) -> String {
        assert!(
            ARMS.contains(&arm),
            "personal-data-surface.txt:{lineno}: no arm {arm:?} in the person act (known: {ARMS:?})"
        );
        arm.to_string()
    }

    fn parse_claim(raw: &str, lineno: usize) -> Claim {
        if let Some(arm) = raw.strip_prefix("erased:") {
            return Claim::Erased {
                arm: parse_arm(arm, lineno),
            };
        }
        if let Some(arm) = raw.strip_prefix("deleted:") {
            return Claim::Deleted {
                arm: parse_arm(arm, lineno),
            };
        }
        if let Some(rest) = raw.strip_prefix("cascaded:") {
            let (arm, parent) = rest.split_once(':').unwrap_or_else(|| {
                panic!(
                    "personal-data-surface.txt:{lineno}: expected `cascaded:<arm>:<parent table>`"
                )
            });
            return Claim::Cascaded {
                arm: parse_arm(arm, lineno),
                parent: parent.to_string(),
            };
        }
        let step = raw.strip_prefix("resource-erasure:D2.").unwrap_or_else(|| {
            panic!(
                "personal-data-surface.txt:{lineno}: unknown achieved claim {raw:?} (known: \
                 `erased:<arm>`, `deleted:<arm>`, `cascaded:<arm>:<table>`, \
                 `resource-erasure:D2.<step>`, `remainder: <reason>`, `-`)"
            )
        });
        assert!(
            D2_STEPS.contains(&step),
            "personal-data-surface.txt:{lineno}: no D2 step {step:?} (known: {D2_STEPS:?})"
        );
        Claim::ResourceErasure {
            step: step.to_string(),
        }
    }

    fn parse_achieved(raw: &str, lineno: usize) -> Achieved {
        if raw == "-" {
            return Achieved::Unstated;
        }
        if let Some(reason) = raw.strip_prefix("remainder:") {
            return Achieved::Remainder(reason.trim().to_string());
        }
        Achieved::Claims(
            raw.split(" + ")
                .map(|c| parse_claim(c.trim(), lineno))
                .collect(),
        )
    }

    /// `table.column` → its declaration. Panics with the offending line, since a malformed manifest
    /// line would otherwise silently drop a declaration and read as an undeclared candidate.
    fn declarations() -> BTreeMap<String, Declaration> {
        let mut out = BTreeMap::new();
        for (lineno, raw) in MANIFEST.lines().enumerate() {
            let lineno = lineno + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Four splits: a note may itself hold `||`.
            let cols: Vec<&str> = line.splitn(5, '|').map(str::trim).collect();
            assert!(
                cols.len() >= 4,
                "personal-data-surface.txt:{lineno}: expected \
                 `table.column | class | ceiling | achieved | note`, got {raw:?}"
            );
            let (key, class, ceiling) = (cols[0].to_string(), cols[1], cols[2]);
            assert!(
                CLASSES.contains(&class),
                "personal-data-surface.txt:{lineno}: unknown class {class:?} (known: {CLASSES:?})"
            );
            assert!(
                CEILINGS.contains(&ceiling),
                "personal-data-surface.txt:{lineno}: unknown ceiling {ceiling:?} (known: {CEILINGS:?})"
            );
            let declaration = Declaration {
                class: class.to_string(),
                ceiling: ceiling.to_string(),
                achieved: parse_achieved(cols[3], lineno),
                // CHANGED from the copied parser: keep the note column.
                note: cols.get(4).copied().unwrap_or_default().to_string(),
            };
            assert!(
                out.insert(key.clone(), declaration).is_none(),
                "personal-data-surface.txt:{lineno}: {key} declared twice"
            );
        }
        out
    }

    fn old_claim_text(claim: &Claim) -> String {
        match claim {
            Claim::Erased { arm } => format!("erased:{arm}"),
            Claim::Deleted { arm } => format!("deleted:{arm}"),
            Claim::Cascaded { arm, parent } => format!("cascaded:{arm}:{parent}"),
            Claim::ResourceErasure { step } => format!("resource-erasure:D2.{step}"),
        }
    }

    fn old_achieved_text(achieved: &Achieved) -> String {
        match achieved {
            Achieved::Unstated => "-".to_owned(),
            Achieved::Claims(claims) => claims
                .iter()
                .map(old_claim_text)
                .collect::<Vec<_>>()
                .join(" + "),
            Achieved::Remainder(reason) => format!("remainder: {reason}"),
        }
    }

    /// (class, ceiling, achieved, note).
    type Row = (String, String, String, String);

    #[test]
    fn personal_data_toml_equals_the_txt() {
        let old: BTreeMap<String, Row> = declarations()
            .into_iter()
            .map(|(key, d)| {
                let achieved = old_achieved_text(&d.achieved);
                (key, (d.class, d.ceiling, achieved, d.note))
            })
            .collect();
        let new: BTreeMap<String, Row> = in_force()
            .personal_data
            .iter()
            .map(|(key, c)| {
                (
                    key.as_str().to_owned(),
                    (
                        class_text(c.class).to_owned(),
                        ceiling_text(c.ceiling).to_owned(),
                        achieved_text(&c.achieved),
                        c.note.clone(),
                    ),
                )
            })
            .collect();
        assert_same("personal-data", &old, &new);
    }
}

/// `sensitivity_scan_surface_test.rs`'s parser, verbatim.
mod scan {
    use std::collections::BTreeMap;

    use super::{assert_same, in_force, scan_disposition_text};

    const SCAN_MANIFEST: &str = include_str!("../../../scripts/sensitivity-scan-surface.txt");

    const DISPOSITIONS: &[&str] = &["scan", "structural", "credential", "out-of-scope"];

    /// `table.column` → (disposition, note) from the scan manifest. Panics with the offending line:
    /// a malformed line would otherwise drop a declaration and read as an undeclared column.
    fn scan_declarations() -> BTreeMap<String, (String, String)> {
        let mut out = BTreeMap::new();
        for (lineno, raw) in SCAN_MANIFEST.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = line.split('|').map(str::trim).collect();
            assert!(
                cols.len() == 3,
                "sensitivity-scan-surface.txt:{}: expected `table.column | disposition | note`, got {raw:?}",
                lineno + 1
            );
            let (key, disposition, note) = (cols[0].to_string(), cols[1], cols[2]);
            assert!(
                DISPOSITIONS.contains(&disposition),
                "sensitivity-scan-surface.txt:{}: unknown disposition {disposition:?} (known: {DISPOSITIONS:?}). \
                 There is deliberately no `unadjudicated` class.",
                lineno + 1
            );
            assert!(
                out.insert(key.clone(), (disposition.to_string(), note.to_string()))
                    .is_none(),
                "sensitivity-scan-surface.txt:{}: {key} declared twice",
                lineno + 1
            );
        }
        out
    }

    /// CHANGED from the copied test: `the_header_count_matches_the_declarations`'s expression for
    /// the stated count, lifted into a function.
    fn stated_count() -> usize {
        SCAN_MANIFEST
            .lines()
            .find_map(|l| {
                let rest = l.strip_prefix("# COUNT at ")?;
                let tail = rest.split("declared here").next()?;
                tail.rsplit(';').next()?.trim().parse().ok()
            })
            .expect(
                "sensitivity-scan-surface.txt: a `# COUNT at <sha>: ...; <n> declared here.` line",
            )
    }

    /// The migration the header's COUNT line names (`# COUNT at migration <m>: …`). New: the old
    /// test never read it, but the TOML carries it as `count_at`.
    fn stated_count_at() -> String {
        SCAN_MANIFEST
            .lines()
            .find_map(|l| {
                let rest = l.strip_prefix("# COUNT at migration ")?;
                Some(rest.split(':').next()?.trim().to_owned())
            })
            .expect("sensitivity-scan-surface.txt: a `# COUNT at migration <m>: …` line")
    }

    #[test]
    fn sensitivity_scan_toml_equals_the_txt() {
        let old = scan_declarations();
        let scan = &in_force().sensitivity;
        let new: BTreeMap<String, (String, String)> = scan
            .columns
            .iter()
            .map(|(key, c)| {
                (
                    key.as_str().to_owned(),
                    (
                        scan_disposition_text(c.disposition).to_owned(),
                        c.note.clone(),
                    ),
                )
            })
            .collect();
        assert_same("sensitivity-scan", &old, &new);
        assert_eq!(
            scan.declared_count,
            stated_count(),
            "declared_count differs from the .txt header's COUNT"
        );
        assert_eq!(
            scan.count_at,
            stated_count_at(),
            "count_at differs from the migration the .txt header's COUNT names"
        );
    }
}

/// `resource_erasure_surface_test.rs`'s parsers, verbatim.
mod resource {
    use std::collections::{BTreeMap, BTreeSet};

    use super::{
        assert_same, in_force, payload_disposition_text, qualifier_text, table_disposition_text,
    };

    const MANIFEST: &str = include_str!("../../../scripts/resource-erasure-surface.txt");

    /// The numbered D2 steps a line may cite (spec D2, steps 1–9, 7a and 9f).
    const D2_STEPS: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "7a", "8", "9", "9f"];

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Disposition {
        /// Emptied or sentineled by this D2 step.
        Handled { step: String },
        /// Its citers are re-pointed through `via` (`table.column`) and the orphaned original deleted.
        Repointed { step: String, via: String },
        /// Deliberately left; the note carries the reason.
        OutOfScope,
    }

    fn parse_step(step: &str, lineno: usize) -> String {
        assert!(
            D2_STEPS.contains(&step),
            "resource-erasure-surface.txt:{lineno}: no D2 step {step:?} (known: {D2_STEPS:?})"
        );
        step.to_string()
    }

    fn parse_disposition(raw: &str, lineno: usize) -> Disposition {
        if let Some(step) = raw.strip_prefix("handled:D2.") {
            return Disposition::Handled {
                step: parse_step(step, lineno),
            };
        }
        if let Some(rest) = raw.strip_prefix("repointed:D2.") {
            let (step, via) = rest.split_once(':').unwrap_or_else(|| {
                panic!(
                    "resource-erasure-surface.txt:{lineno}: expected `repointed:D2.<step>:<table>.<column>`"
                )
            });
            assert!(
                via.split_once('.').is_some(),
                "resource-erasure-surface.txt:{lineno}: {via:?} is not `table.column`"
            );
            return Disposition::Repointed {
                step: parse_step(step, lineno),
                via: via.to_string(),
            };
        }
        assert!(
            raw == "out-of-scope",
            "resource-erasure-surface.txt:{lineno}: unknown disposition {raw:?} (known: \
             `handled:D2.<step>`, `repointed:D2.<step>:<table>.<column>`, `out-of-scope`)"
        );
        Disposition::OutOfScope
    }

    /// The manifest's declaration lines, each with its section and 1-based line number. Panics on an
    /// unknown section or a line outside one: a malformed line would otherwise drop a declaration and
    /// read as uncovered.
    fn section_lines() -> Vec<(&'static str, usize, &'static str)> {
        let mut out = Vec::new();
        let mut section: Option<&'static str> = None;
        for (i, raw) in MANIFEST.lines().enumerate() {
            let lineno = i + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                assert!(
                    matches!(name, "tables" | "payload"),
                    "resource-erasure-surface.txt:{lineno}: unknown section [{name}] (known: [tables], \
                     [payload])"
                );
                section = Some(if name == "tables" {
                    "tables"
                } else {
                    "payload"
                });
                continue;
            }
            let section = section.unwrap_or_else(|| {
                panic!(
                    "resource-erasure-surface.txt:{lineno}: a declaration outside any section: {raw:?}"
                )
            });
            out.push((section, lineno, line));
        }
        out
    }

    /// `table.column` → (disposition, note), from the manifest's `[tables]` section.
    fn declarations() -> BTreeMap<String, (Disposition, String)> {
        let mut out = BTreeMap::new();
        for (section, lineno, line) in section_lines() {
            if section != "tables" {
                continue;
            }
            let raw = line;
            let cols: Vec<&str> = line.split('|').map(str::trim).collect();
            assert!(
                cols.len() == 3,
                "resource-erasure-surface.txt:{lineno}: expected `table.column | disposition | note`, got {raw:?}"
            );
            let key = cols[0].to_string();
            assert!(
                key.split_once('.')
                    .is_some_and(|(t, c)| !t.is_empty() && !c.is_empty()),
                "resource-erasure-surface.txt:{lineno}: {key:?} is not `table.column`"
            );
            let disposition = parse_disposition(cols[1], lineno);
            assert!(
                out.insert(key.clone(), (disposition, cols[2].to_string()))
                    .is_none(),
                "resource-erasure-surface.txt:{lineno}: {key} declared twice"
            );
        }
        out
    }

    /// The redaction classes of D4, one per sentinel shape.
    const REDACT_CLASSES: &[&str] = &[
        "title",
        "origin-uri",
        "remote-source-url",
        "property-key",
        "property-value",
        "facet-value",
        "doc-type",
        "artifact-family",
        "edge-label",
        "reason",
        "scar",
        "authorship",
    ];

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum PayloadDisposition {
        Redact(String),
        Structural,
        OutOfScope,
        Permissive,
    }

    /// One `[payload]` line: `<event_type>:<path>[<qualifier>]`, its disposition and its note.
    /// `event_type` is `metadata` for an authorship key.
    #[derive(Debug, Clone)]
    struct PayloadLine {
        lineno: usize,
        event_type: String,
        path: String,
        qualifier: Option<String>,
        disposition: PayloadDisposition,
        note: String,
    }

    fn payload_lines() -> Vec<PayloadLine> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        for (section, lineno, line) in section_lines() {
            if section != "payload" {
                continue;
            }
            let cols: Vec<&str> = line.split('|').map(str::trim).collect();
            assert!(
                cols.len() == 3,
                "resource-erasure-surface.txt:{lineno}: expected `<type>:<path> | disposition | note`, got {line:?}"
            );
            let (event_type, rest) = cols[0].split_once(':').unwrap_or_else(|| {
                panic!(
                    "resource-erasure-surface.txt:{lineno}: {:?} is not `<type>:<path>`",
                    cols[0]
                )
            });
            let (path, qualifier) = match rest.split_once('[') {
                Some((p, q)) => (
                    p,
                    Some(
                        q.strip_suffix(']')
                            .unwrap_or_else(|| {
                                panic!("resource-erasure-surface.txt:{lineno}: unclosed qualifier")
                            })
                            .to_string(),
                    ),
                ),
                None => (rest, None),
            };
            assert!(
                path == "*" || path.starts_with('/'),
                "resource-erasure-surface.txt:{lineno}: a path is a JSON pointer or `*`, got {path:?}"
            );
            let disposition = match cols[1] {
                "structural" => PayloadDisposition::Structural,
                "out-of-scope" => PayloadDisposition::OutOfScope,
                "permissive" => PayloadDisposition::Permissive,
                d => {
                    let class = d.strip_prefix("redact:").unwrap_or_else(|| {
                        panic!(
                            "resource-erasure-surface.txt:{lineno}: unknown disposition {d:?} (known: \
                             `redact:<class>`, `structural`, `out-of-scope`, `permissive`)"
                        )
                    });
                    assert!(
                        REDACT_CLASSES.contains(&class),
                        "resource-erasure-surface.txt:{lineno}: no D4 class {class:?} (known: {REDACT_CLASSES:?})"
                    );
                    PayloadDisposition::Redact(class.to_string())
                }
            };
            assert!(
                (path == "*") == (disposition == PayloadDisposition::Permissive),
                "resource-erasure-surface.txt:{lineno}: `<type>:*` and `permissive` go together"
            );
            assert!(
                seen.insert(cols[0].to_string()),
                "resource-erasure-surface.txt:{lineno}: {} declared twice",
                cols[0]
            );
            out.push(PayloadLine {
                lineno,
                event_type: event_type.to_string(),
                path: path.to_string(),
                qualifier,
                disposition,
                note: cols[2].to_string(),
            });
        }
        assert!(
            !out.is_empty(),
            "the manifest parse found no [payload] lines"
        );
        out
    }

    fn old_table_disposition_text(disposition: &Disposition) -> String {
        match disposition {
            Disposition::Handled { step } => format!("handled:D2.{step}"),
            Disposition::Repointed { step, via } => format!("repointed:D2.{step}:{via}"),
            Disposition::OutOfScope => "out-of-scope".to_owned(),
        }
    }

    fn old_payload_disposition_text(disposition: &PayloadDisposition) -> String {
        match disposition {
            PayloadDisposition::Redact(class) => format!("redact:{class}"),
            PayloadDisposition::Structural => "structural".to_owned(),
            PayloadDisposition::OutOfScope => "out-of-scope".to_owned(),
            PayloadDisposition::Permissive => "permissive".to_owned(),
        }
    }

    #[test]
    fn resource_erasure_tables_toml_equals_the_txt() {
        let old: BTreeMap<String, (String, String)> = declarations()
            .into_iter()
            .map(|(key, (d, note))| (key, (old_table_disposition_text(&d), note)))
            .collect();
        let new: BTreeMap<String, (String, String)> = in_force()
            .resource_erasure
            .tables
            .iter()
            .map(|(key, line)| {
                (
                    key.as_str().to_owned(),
                    (table_disposition_text(&line.disposition), line.note.clone()),
                )
            })
            .collect();
        assert_same("resource-erasure [tables]", &old, &new);
    }

    /// (event type, path, qualifier) → (disposition, note).
    type PayloadMap = BTreeMap<(String, String, Option<String>), (String, String)>;

    #[test]
    fn resource_erasure_payload_toml_equals_the_txt() {
        let mut old = PayloadMap::new();
        for line in payload_lines() {
            let key = (line.event_type, line.path, line.qualifier);
            let value = (old_payload_disposition_text(&line.disposition), line.note);
            assert!(
                old.insert(key, value).is_none(),
                "resource-erasure-surface.txt:{}: a duplicate the old parser let through",
                line.lineno
            );
        }
        let mut new = PayloadMap::new();
        for (event_type, paths) in &in_force().resource_erasure.payload {
            for (path, line) in paths {
                new.insert(
                    (event_type.clone(), path.clone(), None),
                    (
                        payload_disposition_text(&line.disposition),
                        line.note.clone(),
                    ),
                );
                for refinement in &line.refine {
                    let key = (
                        event_type.clone(),
                        path.clone(),
                        Some(qualifier_text(&refinement.r#where)),
                    );
                    let value = (
                        payload_disposition_text(&refinement.disposition),
                        refinement.note.clone(),
                    );
                    assert!(
                        new.insert(key.clone(), value).is_none(),
                        "{key:?} appears twice in resource-erasure.toml"
                    );
                }
            }
        }
        assert_same("resource-erasure [payload]", &old, &new);
    }
}
