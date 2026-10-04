#![cfg(feature = "test-db")]
//! Every carrier a resource reaches is handled by the erasure act or declared out of scope; this
//! test is the join that says so.
//!
//! Under goal *"A single resource can be erased out of a live estate"* (spec D9, the table half;
//! Witness 13). Four sources meet here, and only one of them is written for this test:
//!
//! - **What a column holds** comes from the two sibling manifests. A carrier is a column
//!   `scripts/sensitivity-scan-surface.txt` classes `scan`, or one
//!   `scripts/personal-data-surface.txt` classes `content`, `incidental` or `derived` (ruled
//!   2026-10-02). Nothing here re-classifies a column.
//! - **Which tables a resource reaches** comes from the live catalog. A foreign key, or a
//!   single-column CHECK on a `*_table` discriminator that enumerates tables, makes its table a
//!   child of the table it names; the walk takes every child of a reached table, recursively, from
//!   five roots, the ledger included (ruled 2026-10-02, amended 2026-10-03). Nothing enumerates
//!   tables by hand, so a new table is reached by construction rather than by someone remembering
//!   it.
//! - **What the act does about each carrier** is declared in `scripts/resource-erasure-surface.txt`.
//! - **Whether the act really does it** is read from the live body of
//!   `_resource_erasure_apply_redaction`, the D2 function: a `handled` line must find an UPDATE of
//!   its table assigning its column an erasure value.
//!
//! **Why the binding to the function body exists.** Without it, the manifest could say a column is
//! handled while the act had stopped erasing it, and every test here would stay green. Reverting
//! the joint-read fixes (`kb_chunks.header_path`, `kb_citation_audits.reason`) is the miss this
//! fence exists for, and `reverting_the_joint_read_fixes_fails_the_fence` proves it turns red.
//!
//! **What this does not claim.** The binding proves the statement is there and assigns an erasure
//! value. It does not prove its WHERE clause reaches every row tied to the resource; the act's own
//! witnesses own that, and the manifest's notes name the rows the act leaves. The walk is blind to
//! references spelled neither as a foreign key nor as a CHECKed `*_table` discriminator; the
//! manifest header lists them. The CHECK derivation here is a widened form of the personal-data
//! fence's `poly` derivation (`personal_data_surface_test.rs`), which names `kb_profiles` only.

use std::collections::{BTreeMap, BTreeSet};

use sqlx::PgPool;

/// The declaration half. Read from the shipped file so the manifest and this test cannot drift.
const MANIFEST: &str = include_str!("../../../scripts/resource-erasure-surface.txt");

/// Which text is prose: read for its `scan` lines. Its own test owns its format.
const SCAN_MANIFEST: &str = include_str!("../../../scripts/sensitivity-scan-surface.txt");

/// Which non-text columns hold content: read for its `content`/`incidental`/`derived` lines. Its
/// own test owns its format.
const PERSONAL_MANIFEST: &str = include_str!("../../../scripts/personal-data-surface.txt");

/// The personal-data classes that make a column a carrier. `identifier` and `reference` are about
/// a person, not about what a resource says; `discriminator` and `none` hold nothing.
const CARRIER_CLASSES: &[&str] = &["content", "incidental", "derived"];

/// The D2 function: the one home of the content shape (spec D2).
const REDACTION_FN: &str = "_resource_erasure_apply_redaction";

/// The numbered D2 steps a line may cite (spec D2, steps 1–9 and 7a).
const D2_STEPS: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "7a", "8", "9"];

/// The parent→child edges of the walk: every foreign key, and every single-column CHECK on a
/// `*_table` discriminator, read as an edge to each public table it names.
const EDGES: &str = r#"
  SELECT DISTINCT conrelid::regclass::text AS child, confrelid::regclass::text AS parent
    FROM pg_constraint
   WHERE contype = 'f' AND connamespace = 'public'::regnamespace
  UNION
  SELECT DISTINCT c.conrelid::regclass::text, m[1]
    FROM pg_constraint c
    JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = c.conkey[1]
   CROSS JOIN LATERAL regexp_matches(pg_get_constraintdef(c.oid), '''([a-z_][a-z0-9_]*)''', 'g') m
   WHERE c.contype = 'c' AND c.connamespace = 'public'::regnamespace
     AND cardinality(c.conkey) = 1 AND a.attname LIKE '%\_table'
     AND m[1] IN (SELECT relname FROM pg_class
                   WHERE relnamespace = 'public'::regnamespace AND relkind IN ('r', 'p'))
"#;

/// Base-table `*_table` columns with no CHECK that ENUMERATES the tables they may name. A
/// discriminator the walk cannot read as an edge could point at a resource unseen.
///
/// "Enumerates" is a positive match on the two shapes Postgres prints for `col IN ('a', 'b')` and
/// `col = 'a'`, once casts, parentheses and whitespace are stripped: `CHECK col=ANY ARRAY['a','b']`
/// and `CHECK col='a'`. Anything else (a pairing CHECK, a pattern, a negation, `IS DISTINCT FROM`,
/// an OR with an open arm) does not confine the column and does not count. At least one literal
/// must name a public table.
const UNENUMERATED_DISCRIMINATORS: &str = r#"
SELECT c.table_name || '.' || c.column_name
  FROM information_schema.columns c
  JOIN information_schema.tables t
    ON t.table_name = c.table_name AND t.table_schema = 'public' AND t.table_type = 'BASE TABLE'
 WHERE c.table_schema = 'public' AND c.column_name LIKE '%\_table'
   AND NOT EXISTS (
     SELECT 1
       FROM pg_constraint k
      CROSS JOIN LATERAL regexp_replace(pg_get_constraintdef(k.oid),
                                        '::[a-z ]+(\[\])?|[()\s]', '', 'g') AS def(shape)
      CROSS JOIN LATERAL regexp_matches(def.shape, '''([a-z_][a-z0-9_]*)''', 'g') m
      WHERE k.contype = 'c' AND k.conrelid = (quote_ident(c.table_name::text))::regclass
        AND def.shape ~ ('^CHECK' || c.column_name
                         || '=(ANYARRAY\[(''[a-z_][a-z0-9_]*'',)*''[a-z_][a-z0-9_]*''\]|''[a-z_][a-z0-9_]*'')$')
        AND m[1] IN (SELECT relname FROM pg_class
                      WHERE relnamespace = 'public'::regnamespace AND relkind IN ('r', 'p')))
 ORDER BY 1
"#;

fn reachable_tables_sql() -> String {
    format!(
        "WITH RECURSIVE edge AS ({EDGES}),
         reach(tbl) AS (
           SELECT unnest(ARRAY['kb_resources','kb_content_blocks','kb_chunks',
                               'kb_block_revisions','kb_edges'])
           UNION
           SELECT edge.child FROM edge JOIN reach ON edge.parent = reach.tbl)
         SELECT tbl FROM reach ORDER BY 1"
    )
}

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

/// `table.column` → (disposition, note), from the manifest's `[tables]` section. Panics with the
/// offending line: a malformed line would otherwise drop a declaration and read as uncovered.
fn declarations() -> BTreeMap<String, (Disposition, String)> {
    let mut out = BTreeMap::new();
    let mut section: Option<&str> = None;
    for (i, raw) in MANIFEST.lines().enumerate() {
        let lineno = i + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            assert!(
                name == "tables",
                "resource-erasure-surface.txt:{lineno}: unknown section [{name}] (cut 1 knows only \
                 [tables]; the [payload] section lands with cut 2 and extends this parser)"
            );
            section = Some(name);
            continue;
        }
        assert!(
            section == Some("tables"),
            "resource-erasure-surface.txt:{lineno}: a declaration outside any section: {raw:?}"
        );
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

/// `table.column` keys of `manifest` whose second field is in `classes`.
fn keys_classed(manifest: &str, classes: &[&str]) -> BTreeSet<String> {
    manifest
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let cols: Vec<&str> = l.split('|').map(str::trim).collect();
            cols.get(1)
                .is_some_and(|c| classes.contains(c))
                .then(|| cols[0].to_string())
        })
        .collect()
}

/// Every carrier column, by the two sibling manifests.
fn carriers() -> BTreeSet<String> {
    let mut out = keys_classed(SCAN_MANIFEST, &["scan"]);
    out.extend(keys_classed(PERSONAL_MANIFEST, CARRIER_CLASSES));
    out
}

fn table_of(key: &str) -> &str {
    key.split_once('.').map_or(key, |(t, _)| t)
}

async fn reachable_tables(pool: &PgPool) -> BTreeSet<String> {
    sqlx::query_scalar::<_, String>(&reachable_tables_sql())
        .fetch_all(pool)
        .await
        .expect("derive the tables a resource reaches")
        .into_iter()
        .collect()
}

async fn redaction_body(pool: &PgPool) -> String {
    sqlx::query_scalar::<_, String>(&format!(
        "SELECT pg_get_functiondef('{REDACTION_FN}'::regproc)"
    ))
    .fetch_one(pool)
    .await
    .expect("read the D2 function's live definition")
}

/// Lowercased tokens of `sql`. `--` and `/* */` comments are dropped; a single-quoted literal
/// (with `''` escapes) is ONE token, kept verbatim, so text inside a RAISE message can never read
/// as a statement. `,` `=` `(` `)` `;` `::` `||` are tokens of their own, so `SET a = 1, b = 2;`
/// reads as `set a = 1 , b = 2 ;` and `'{}'::jsonb||x` as `'{}' :: jsonb || x`, spaced or not.
fn tokens(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut i = 0;
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() {
            out.push(std::mem::take(cur).to_lowercase());
        }
    };
    while i < chars.len() {
        let ch = chars[i];
        let next = chars.get(i + 1).copied();
        if ch == '-' && next == Some('-') {
            flush(&mut cur, &mut out);
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && next == Some('*') {
            flush(&mut cur, &mut out);
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else if ch == '\'' {
            flush(&mut cur, &mut out);
            let mut lit = String::from('\'');
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' && chars.get(i + 1) == Some(&'\'') {
                    lit.push_str("''");
                    i += 2;
                } else if chars[i] == '\'' {
                    break;
                } else {
                    lit.push(chars[i]);
                    i += 1;
                }
            }
            lit.push('\'');
            out.push(lit);
            i += 1;
        } else if (ch == ':' && next == Some(':')) || (ch == '|' && next == Some('|')) {
            flush(&mut cur, &mut out);
            out.push(format!("{ch}{ch}"));
            i += 2;
        } else if ch.is_whitespace() || ",=();".contains(ch) {
            flush(&mut cur, &mut out);
            if !ch.is_whitespace() {
                out.push(ch.to_string());
            }
            i += 1;
        } else {
            cur.push(ch);
            i += 1;
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// The constant erasure values: NULL, an empty string or object, and the property sentinel.
const ERASURE_CONSTANTS: &[&str] = &["null", "''", "'{}'", "'\"erased\"'"];

fn is_ident(s: &str) -> bool {
    s.chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Whether `token` names the row's own identity or the act's resource: `p_resource`, `<alias>.id`
/// or a key number `<alias>.n`. These are what a sentinel may carry.
fn is_identity(token: &str) -> bool {
    token == "p_resource"
        || token
            .split_once('.')
            .is_some_and(|(alias, col)| is_ident(alias) && matches!(col, "id" | "n"))
}

/// Whether a whole SET value expression erases. Three shapes only:
///
/// - a constant from [`ERASURE_CONSTANTS`], optionally cast (`'{}'::jsonb`);
/// - an `'erased…'` literal joined by `||` to an identity, optionally cast to text
///   (`'erased-' || r.id::text`);
/// - the zero vector, `array_fill(0, ARRAY[n])::vector`: the erasure value of a NOT NULL vector
///   column (`kb_cogmap_regions.centroid`, whose memberless convention it already is). Only a
///   literal `0` fill and a literal dimension: a fill or dimension read from a row could carry it.
///
/// Anything else is not an erasure, however it starts: `'erased-' || r.title` keeps the title,
/// `'' || content` keeps the content, and `'{}'::jsonb || payload` keeps the payload.
fn is_erasure_value(expr: &[String]) -> bool {
    let constant = |v: &String| ERASURE_CONSTANTS.contains(&v.as_str());
    let sentinel = |v: &String, op: &String, id: &String| {
        v.starts_with("'erased") && op == "||" && is_identity(id)
    };
    // A three-token value is EITHER a cast constant or an uncast sentinel. Both shapes are tried:
    // two `[a, b, c]` match arms would make the second unreachable.
    match expr {
        [v] => constant(v),
        [a, b, c] => (constant(a) && b == "::" && is_ident(c)) || sentinel(a, b, c),
        [v, op, id, cast, ty] => sentinel(v, op, id) && cast == "::" && ty == "text",
        [f, open, zero, comma, dims, close, cast, ty] => {
            f == "array_fill"
                && open == "("
                && zero == "0"
                && comma == ","
                && is_literal_dims(dims)
                && close == ")"
                && cast == "::"
                && ty == "vector"
        }
        _ => false,
    }
}

/// Whether `token` is `array[<digits>]`: a literal dimension list of one integer.
fn is_literal_dims(token: &str) -> bool {
    token
        .strip_prefix("array[")
        .and_then(|rest| rest.strip_suffix(']'))
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// The tokens of a SET value starting at `start`, up to the `,` that ends it, the `;` that ends the
/// statement, or the `where` / `from` / `returning` that ends the SET list, all at paren depth 0.
fn value_expr(t: &[String], start: usize) -> &[String] {
    let mut depth = 0usize;
    let mut end = start;
    while let Some(tok) = t.get(end) {
        match tok.as_str() {
            "(" => depth += 1,
            ")" if depth == 0 => break,
            ")" => depth -= 1,
            "," | ";" | "where" | "from" | "returning" if depth == 0 => break,
            _ => {}
        }
        end += 1;
    }
    &t[start..end]
}

/// Every value expression `body` assigns to `table.column` in an UPDATE of `table`. An assignment
/// target is a token at paren depth 0, inside the SET list (after `set`, before the `from` /
/// `where` / `returning` / `;` that ends it), right after `set` or `,` and right before `=`.
fn assignments<'a>(t: &'a [String], key: &str) -> Vec<&'a [String]> {
    let Some((table, column)) = key.split_once('.') else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in 0..t.len() {
        if t[i] != "update" || t.get(i + 1).is_none_or(|x| x != table) {
            continue;
        }
        let mut seen_set = false;
        let mut depth = 0usize;
        for j in i + 2..t.len() {
            match t[j].as_str() {
                ";" => break,
                "(" => depth += 1,
                ")" => depth = depth.saturating_sub(1),
                "set" if depth == 0 => seen_set = true,
                "from" | "where" | "returning" if depth == 0 && seen_set => break,
                tok if seen_set
                    && depth == 0
                    && tok == column
                    && matches!(t[j - 1].as_str(), "set" | ",")
                    && t.get(j + 1).is_some_and(|x| x == "=") =>
                {
                    out.push(value_expr(t, j + 2));
                }
                _ => {}
            }
        }
    }
    out
}

/// Whether `expr` reads `column`, bare or qualified: an assignment of the column's own value.
fn reads_column(expr: &[String], column: &str) -> bool {
    expr.iter()
        .any(|tok| tok == column || tok.rsplit_once('.').is_some_and(|(_, col)| col == column))
}

fn deletes_from(t: &[String], table: &str) -> bool {
    t.windows(3)
        .any(|w| w[0] == "delete" && w[1] == "from" && w[2] == table)
}

/// Whether `body` does what `disposition` claims for `key`.
///
/// A `handled` column must be assigned at least once and EVERY assignment must erase, so a later
/// UPDATE that writes the original back unbinds it. A `repointed` column needs a placing
/// assignment to its via column: one that neither reads the column (a no-op) nor is a bare
/// identity (the act's park pass, `source_id = bp.id`, which parks a row on its own id before the
/// place pass) — and the DELETE.
fn binds(body: &str, key: &str, disposition: &Disposition) -> bool {
    let t = tokens(body);
    match disposition {
        Disposition::Handled { .. } => {
            let values = assignments(&t, key);
            !values.is_empty() && values.into_iter().all(is_erasure_value)
        }
        Disposition::Repointed { via, .. } => {
            let via_column = via.split_once('.').map_or(via.as_str(), |(_, c)| c);
            assignments(&t, via).into_iter().any(|expr| {
                !expr.is_empty()
                    && !reads_column(expr, via_column)
                    && !matches!(expr, [only] if is_identity(only))
            }) && deletes_from(&t, table_of(key))
        }
        Disposition::OutOfScope => true,
    }
}

/// Carriers in reachable tables with no line in the manifest. `carriers` is a parameter, not read
/// inside, so the Witness 13 probes can stand in for the sibling-manifest PR that would class them.
fn uncovered(reach: &BTreeSet<String>, carriers: &BTreeSet<String>) -> Vec<String> {
    let declared = declarations();
    carriers
        .iter()
        .filter(|k| reach.contains(table_of(k)) && !declared.contains_key(*k))
        .cloned()
        .collect()
}

/// Lines whose disposition the D2 function does not carry out.
fn unbound(body: &str) -> Vec<String> {
    declarations()
        .into_iter()
        .filter(|(k, (d, _))| !binds(body, k, d))
        .map(|(k, _)| k)
        .collect()
}

async fn unenumerated_discriminators(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar::<_, String>(UNENUMERATED_DISCRIMINATORS)
        .fetch_all(pool)
        .await
        .expect("probe base-table *_table columns for an enumerating CHECK")
}

/// FAILS IF: a carrier sits in a table a resource reaches and the manifest says nothing about what
/// the erasure act does with it. This is the direction that matters: content that survives an
/// erasure while the act reports success.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_reachable_carrier_is_handled_or_declared(pool: PgPool) {
    let missing = uncovered(&reachable_tables(&pool).await, &carriers());
    assert!(
        missing.is_empty(),
        "these columns are carriers (`scan` in scripts/sensitivity-scan-surface.txt, or \
         content/incidental/derived in scripts/personal-data-surface.txt), sit in a table a \
         resource reaches, and have no line in scripts/resource-erasure-surface.txt.\n\
         Either make `{REDACTION_FN}` erase the column and declare it `handled:D2.<step>`, or \
         declare it `out-of-scope` with the reason the act leaves it.\n\
         Uncovered: {missing:#?}"
    );
}

/// FAILS IF: a line claims something the D2 function no longer does: a `handled` column it does not
/// assign an erasure value, or a `repointed` column whose re-point or delete is gone.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_handled_declaration_is_carried_out_by_the_redaction(pool: PgPool) {
    let broken = unbound(&redaction_body(&pool).await);
    assert!(
        broken.is_empty(),
        "scripts/resource-erasure-surface.txt declares these columns handled, but the live \
         `{REDACTION_FN}` does not do what the line says: a `handled` column needs an UPDATE of its \
         table assigning NULL, '', '{{}}', '\"erased\"' or an 'erased…' sentinel; a `repointed` \
         one needs the re-point UPDATE and the DELETE.\n\
         Restore the step, or change the line to say what the act now does.\n\
         Unbound: {broken:#?}"
    );
}

/// FAILS IF: a line names a column that is not a carrier (the manifest's subject is the siblings'
/// carriers, nothing else), or an `out-of-scope` line names a table no resource reaches (there is
/// nothing to be out of scope OF). A `handled` or `repointed` line outside the walk is allowed:
/// the binding test checks it against the function body instead (today, `kb_remote_sources.uri`).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_declaration_is_live(pool: PgPool) {
    let carriers = carriers();
    let reach = reachable_tables(&pool).await;
    let stale: Vec<String> = declarations()
        .into_iter()
        .filter(|(k, (d, _))| {
            !carriers.contains(k) || (*d == Disposition::OutOfScope && !reach.contains(table_of(k)))
        })
        .map(|(k, _)| k)
        .collect();
    assert!(
        stale.is_empty(),
        "scripts/resource-erasure-surface.txt declares columns that are not carriers in either \
         sibling manifest, or are out of scope in a table no resource reaches. Delete the line, or \
         (if a sibling reclassified the column) check that manifest's reason first.\n\
         Stale: {stale:#?}"
    );
}

/// FAILS IF: a base-table `*_table` discriminator has no CHECK enumerating the tables it may name. The walk reads polymorphic edges from those CHECKs; a discriminator without one
/// could point at a resource and the walk would never see its table.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_polymorphic_discriminator_enumerates_its_targets(pool: PgPool) {
    let bare = unenumerated_discriminators(&pool).await;
    assert!(
        bare.is_empty(),
        "these `*_table` columns carry no CHECK enumerating their target tables \
         (`col IN ('kb_a', 'kb_b')` or `col = 'kb_a'`), so the erasure fence cannot tell what they \
         reference. A pairing CHECK, a pattern, a negation or `IS DISTINCT FROM` does not count. \
         Add the enumerating CHECK, the convention every other discriminator follows.\n\
         Unenumerated: {bare:#?}"
    );
}

/// FAILS IF: an `out-of-scope` line gives no reason. The class means "the act leaves this", and a
/// reader needs to know why to tell a judgement from an omission.
#[test]
fn every_out_of_scope_line_states_its_reason() {
    let silent: Vec<String> = declarations()
        .into_iter()
        .filter(|(_, (d, note))| *d == Disposition::OutOfScope && note.is_empty())
        .map(|(k, _)| k)
        .collect();
    assert!(
        silent.is_empty(),
        "`out-of-scope` requires the reason in the note column.\nSilent: {silent:#?}"
    );
}

/// Witness 13. FAILS IF: a carrier added to a resource-reachable table, in any of the ways a table
/// becomes reachable, is not reported; or one added to an unreached table is.
///
/// The reachable probes are: a text column on an existing table, a jsonb column on one (the
/// non-text carriers, ruled 2026-10-02), a new child of a root, a new grandchild (the walk is
/// transitive), a new polymorphic owner of `kb_resources`, a new polymorphic owner of a root other
/// than `kb_resources`, a new polymorphic owner of a reached table that is not a root (edges come
/// from a CHECK naming ANY table), and a new table holding only a foreign key into `kb_events` (the
/// walk descends from the ledger, ruled 2026-10-03). The new tables are the case a hand-maintained
/// table list would miss. The last probe references only `kb_teams`, which no resource reaches, and
/// must NOT be reported: reach is not everything.
///
/// The probe columns join the carrier set here, standing in for the sibling-manifest PR that would
/// class them; the "until declared" half is `every_reachable_carrier_is_handled_or_declared`, which
/// reads the shipped manifest. Each `sqlx::test` runs in its own database, so nothing escapes.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_added_carrier_in_reach_is_reported(pool: PgPool) {
    assert!(
        uncovered(&reachable_tables(&pool).await, &carriers()).is_empty(),
        "precondition: the fence holds before the probes are added"
    );

    for ddl in [
        "ALTER TABLE kb_content_blocks ADD COLUMN erasure_fence_probe text",
        "ALTER TABLE kb_content_blocks ADD COLUMN erasure_fence_probe_doc jsonb",
        "CREATE TABLE erasure_fence_probe_child (
             id uuid PRIMARY KEY,
             block_id uuid NOT NULL REFERENCES kb_content_blocks(id),
             note text)",
        "CREATE TABLE erasure_fence_probe_grandchild (
             child_id uuid NOT NULL REFERENCES erasure_fence_probe_child(id),
             note text)",
        "CREATE TABLE erasure_fence_probe_owner (
             owner_table text NOT NULL CHECK (owner_table IN ('kb_resources', 'kb_cogmaps')),
             owner_id uuid NOT NULL,
             note text)",
        "CREATE TABLE erasure_fence_probe_block_owner (
             target_table text NOT NULL CHECK (target_table IN ('kb_content_blocks')),
             target_id uuid NOT NULL,
             note text)",
        "CREATE TABLE erasure_fence_probe_artifact_owner (
             target_table text NOT NULL CHECK (target_table IN ('kb_data_artifacts')),
             target_id uuid NOT NULL,
             note text)",
        "CREATE TABLE erasure_fence_probe_caused (
             caused_by_event_id uuid NOT NULL REFERENCES kb_events(id),
             note text)",
        "CREATE TABLE erasure_fence_probe_unrelated (
             team_id uuid NOT NULL REFERENCES kb_teams(id),
             note text)",
    ] {
        sqlx::query(ddl)
            .execute(&pool)
            .await
            .expect("apply probe DDL");
    }

    let reported = [
        "erasure_fence_probe_artifact_owner.note",
        "erasure_fence_probe_block_owner.note",
        "erasure_fence_probe_caused.note",
        "erasure_fence_probe_child.note",
        "erasure_fence_probe_grandchild.note",
        "erasure_fence_probe_owner.note",
        "kb_content_blocks.erasure_fence_probe",
        "kb_content_blocks.erasure_fence_probe_doc",
    ];
    let mut carriers = carriers();
    carriers.extend(reported.iter().map(|p| p.to_string()));
    carriers.insert("erasure_fence_probe_unrelated.note".to_string());

    assert_eq!(
        uncovered(&reachable_tables(&pool).await, &carriers),
        reported.map(String::from).to_vec(),
        "every reachable probe must be reported, and the unrelated one must not"
    );
}

/// The done-when sanity check. FAILS IF: reverting the joint-read fixes leaves the fence green.
///
/// The first draft of D2 missed `kb_chunks.header_path` and `kb_citation_audits.reason`. This
/// rewrites the live function so those two UPDATEs keep the column's own value, which still
/// mentions and assigns the column but erases nothing, and asserts the binding reports exactly those
/// two. Each rewrite must match exactly once, so the probe cannot silently test nothing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reverting_the_joint_read_fixes_fails_the_fence(pool: PgPool) {
    let body = redaction_body(&pool).await;
    assert!(
        unbound(&body).is_empty(),
        "precondition: every line is bound"
    );

    let mut reverted = body.clone();
    for (fix, revert) in [
        ("SET header_path = NULL", "SET header_path = header_path"),
        ("SET reason = NULL", "SET reason = reason"),
    ] {
        assert_eq!(
            reverted.matches(fix).count(),
            1,
            "the probe expects `{fix}` exactly once in {REDACTION_FN}; the function changed shape"
        );
        reverted = reverted.replace(fix, revert);
    }
    sqlx::query(&reverted)
        .execute(&pool)
        .await
        .expect("install the reverted function");

    assert_eq!(
        unbound(&redaction_body(&pool).await),
        vec![
            "kb_chunks.header_path".to_string(),
            "kb_citation_audits.reason".to_string()
        ],
        "reverting the joint-read fixes must unbind exactly those two columns"
    );
}

/// FAILS IF: reverting the derived-vector statements (7b, 7c) leaves the fence green.
///
/// Ruled 2026-10-04: the act nulls the home context's telos snapshot and zeroes a folded region's
/// centroid. This rewrites those two UPDATEs to keep the column's own value and asserts the binding
/// reports exactly those two columns, so the zero-vector shape the fence learned for them binds
/// only the erasing statement. Each rewrite must match exactly once.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reverting_the_derived_vector_statements_fails_the_fence(pool: PgPool) {
    let body = redaction_body(&pool).await;
    assert!(
        unbound(&body).is_empty(),
        "precondition: every line is bound"
    );

    let mut reverted = body.clone();
    for (fix, revert) in [
        (
            "SET telos_centroid = NULL",
            "SET telos_centroid = telos_centroid",
        ),
        (
            "SET centroid = array_fill(0, ARRAY[768])::vector",
            "SET centroid = centroid",
        ),
    ] {
        assert_eq!(
            reverted.matches(fix).count(),
            1,
            "the probe expects `{fix}` exactly once in {REDACTION_FN}; the function changed shape"
        );
        reverted = reverted.replace(fix, revert);
    }
    sqlx::query(&reverted)
        .execute(&pool)
        .await
        .expect("install the reverted function");

    assert_eq!(
        unbound(&redaction_body(&pool).await),
        vec![
            "kb_cogmap_regions.centroid".to_string(),
            "kb_contexts.telos_centroid".to_string()
        ],
        "reverting the derived-vector statements must unbind exactly those two columns"
    );
}

/// FAILS IF: a `*_table` column whose CHECKs do not enumerate public tables is not reported by the
/// guard.
///
/// One probe per way a CHECK can fail. The guard has two conjuncts, the enumeration shape and "at
/// least one literal names a public table", and each probe is excluded by exactly one: no CHECK at
/// all; a CHECK with a second column in it; a pattern; a negation; `IS DISTINCT FROM` (each of
/// these names a real table, so only the shape excludes it); a list whose first value has an
/// embedded quote, `'kb_resources''x'`, which a looser list pattern reads as two items, one of them
/// a table; and a well-shaped
/// enumeration of a name that is no table (only the public-table conjunct excludes it).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_unenumerated_discriminator_fails_the_guard(pool: PgPool) {
    assert!(
        unenumerated_discriminators(&pool).await.is_empty(),
        "precondition"
    );
    for ddl in [
        "CREATE TABLE erasure_fence_probe_bare (owner_table text, owner_id uuid)",
        "CREATE TABLE erasure_fence_probe_paired (
             owner_table text, owner_id uuid,
             CHECK (owner_table = 'kb_resources' OR owner_id IS NULL))",
        "CREATE TABLE erasure_fence_probe_pattern (
             owner_table text CHECK (owner_table ~ 'kb_resources'), owner_id uuid)",
        "CREATE TABLE erasure_fence_probe_negated (
             owner_table text CHECK (owner_table <> 'kb_resources'), owner_id uuid)",
        "CREATE TABLE erasure_fence_probe_distinct (
             owner_table text CHECK (owner_table IS DISTINCT FROM 'kb_resources'), owner_id uuid)",
        "CREATE TABLE erasure_fence_probe_quoted (
             owner_table text CHECK (owner_table IN ('kb_resources''x', 'kb_no_such_table')), owner_id uuid)",
        "CREATE TABLE erasure_fence_probe_nontable (
             owner_table text CHECK (owner_table IN ('kb_no_such_table')), owner_id uuid)",
    ] {
        sqlx::query(ddl)
            .execute(&pool)
            .await
            .expect("create probe table");
    }
    assert_eq!(
        unenumerated_discriminators(&pool).await,
        vec![
            "erasure_fence_probe_bare.owner_table".to_string(),
            "erasure_fence_probe_distinct.owner_table".to_string(),
            "erasure_fence_probe_negated.owner_table".to_string(),
            "erasure_fence_probe_nontable.owner_table".to_string(),
            "erasure_fence_probe_paired.owner_table".to_string(),
            "erasure_fence_probe_pattern.owner_table".to_string(),
            "erasure_fence_probe_quoted.owner_table".to_string(),
        ]
    );
}

/// The binding itself, on the shapes the D2 function uses and the shapes it must not be fooled by.
/// Each negative case is excluded by exactly one rule, named beside it.
#[test]
fn the_binding_reads_erasing_assignments_not_mentions() {
    let sql = "
        UPDATE kb_chunks c
           SET embedding = NULL, embedded_with = NULL
         WHERE header_path = NULL;
        -- UPDATE kb_chunk_content SET content = NULL;
        /* UPDATE kb_block_content SET content = ''; */
        RAISE EXCEPTION 'UPDATE kb_edges SET label = NULL;';
        UPDATE kb_resources r
           SET title = 'erased-' || r.id::text, origin_uri = 'erased:' || r.origin_uri;
        UPDATE kb_ingestion_records SET source_uri = '' || source_uri;
        UPDATE kb_workflow_jobs j SET payload = '{}'::jsonb || j.payload, last_error = 'erased';
        WITH ranked AS (SELECT 1)
        UPDATE kb_properties p
           SET property_key   = 'erased-key-' || ranked.n::text,
               property_value = '\"erased\"'::jsonb
          FROM ranked;
        UPDATE kb_block_provenance bp SET source_id = (v ->> bp.id::text)::uuid;
        DELETE FROM kb_remote_sources r WHERE r.id = v;
    ";
    let handled = Disposition::Handled { step: "9".into() };
    for key in [
        "kb_chunks.embedding",
        "kb_chunks.embedded_with",
        "kb_resources.title",
        "kb_properties.property_key",
        "kb_properties.property_value",
    ] {
        assert!(binds(sql, key, &handled), "{key} is erased here");
    }
    for (key, why) in [
        (
            "kb_chunks.header_path",
            "a WHERE comparison (the set/, rule)",
        ),
        ("kb_chunk_content.content", "a line comment"),
        ("kb_block_content.content", "a block comment"),
        ("kb_edges.label", "a string literal"),
        (
            "kb_resources.origin_uri",
            "a sentinel joined to the original value",
        ),
        (
            "kb_ingestion_records.source_uri",
            "an empty string joined to the original",
        ),
        (
            "kb_workflow_jobs.payload",
            "an empty object merged with the original",
        ),
        (
            "kb_workflow_jobs.last_error",
            "a sentinel literal that is not a constant",
        ),
        (
            "kb_remote_sources.uri",
            "a DELETE is not a handled assignment",
        ),
    ] {
        assert!(!binds(sql, key, &handled), "{key}: {why} is not an erasure");
    }

    let repointed = Disposition::Repointed {
        step: "9".into(),
        via: "kb_block_provenance.source_id".into(),
    };
    assert!(binds(sql, "kb_remote_sources.uri", &repointed));
    for (body, why) in [
        (
            "DELETE FROM kb_remote_sources r WHERE r.id = v;",
            "a delete without the re-point leaves the citers on the original URL",
        ),
        (
            "UPDATE kb_block_provenance bp SET source_id = v;",
            "a re-point without the delete leaves the original row",
        ),
        (
            "UPDATE kb_block_provenance bp SET source_id = bp.source_id;
             DELETE FROM kb_remote_sources r WHERE r.id = v;",
            "a re-point to its own value leaves every citer in place, so the delete never fires",
        ),
    ] {
        assert!(!binds(body, "kb_remote_sources.uri", &repointed), "{why}");
    }
}

/// The binding's remaining rules, each case excluded by exactly the one named beside it.
#[test]
fn the_binding_is_not_fooled_by_spacing_restoring_or_parking() {
    let handled = Disposition::Handled { step: "9".into() };
    let bound = |sql: &str, key: &str| binds(sql, key, &handled);

    assert!(
        bound(
            "UPDATE kb_resources r SET title = 'erased-'||r.id::text;",
            "kb_resources.title"
        ),
        "an erasing sentinel binds however it is spaced"
    );
    assert!(
        bound(
            "UPDATE kb_ingestion_records SET source_uri = 'erased:' || p_resource;",
            "kb_ingestion_records.source_uri"
        ),
        "an uncast sentinel binds: a three-token value is tried as both a cast and a sentinel"
    );
    for (sql, key, why) in [
        (
            "UPDATE kb_data_artifact_content SET content = '{}'::jsonb||content;",
            "kb_data_artifact_content.content",
            "a merge glued to its cast (operator tokens)",
        ),
        (
            "UPDATE kb_resources r SET title = 'erased-' || title||r.id;",
            "kb_resources.title",
            "the original glued in front of an id (operator tokens)",
        ),
        (
            "UPDATE kb_resources r SET title = 'erased-' || title->>k.id;",
            "kb_resources.title",
            "an id read out of the original value (the alias must be an identifier)",
        ),
        (
            "UPDATE kb_resources r SET title = 'x-' || r.id;",
            "kb_resources.title",
            "a literal that is not an 'erased' sentinel",
        ),
        (
            "UPDATE kb_resources r SET title = 'erased-' + r.id;",
            "kb_resources.title",
            "a join that is not ||",
        ),
        (
            "UPDATE kb_resources r SET title = 'erased-' || r.id::varchar;",
            "kb_resources.title",
            "a sentinel cast to something other than text",
        ),
        (
            "UPDATE kb_resources r SET title = 'erased-' || r.id::text;
             UPDATE kb_resources r SET title = h.title FROM h;",
            "kb_resources.title",
            "a later UPDATE writing the original back (every assignment must erase)",
        ),
        (
            "UPDATE kb_chunk_content SET content_hash = content_hash RETURNING id, content = '';",
            "kb_chunk_content.content",
            "a comparison after RETURNING (targets end with the SET list)",
        ),
        (
            "UPDATE kb_chunk_content SET content_hash = coalesce(x, content = '');",
            "kb_chunk_content.content",
            "a comparison inside parentheses (targets are at depth 0)",
        ),
    ] {
        assert!(!bound(sql, key), "{key}: {why} is not an erasure");
    }

    let repointed = Disposition::Repointed {
        step: "9".into(),
        via: "kb_block_provenance.source_id".into(),
    };
    assert!(
        !binds(
            "UPDATE kb_block_provenance bp SET source_id = bp.id;
             DELETE FROM kb_remote_sources r WHERE r.id = v;",
            "kb_remote_sources.uri",
            &repointed
        ),
        "the park pass alone (a bare identity) leaves provenance on its own ids: no place pass"
    );
}
