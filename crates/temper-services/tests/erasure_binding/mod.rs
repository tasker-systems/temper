#![allow(dead_code)]
//! The statement reader both erasure fences bind with: `resource_erasure_surface_test.rs` against
//! the resource act's body, `personal_data_surface_test.rs` against the person act's. One copy, so
//! the two fences cannot disagree about what an erasing statement is.
//!
//! It reads statements, not control flow and not rows: an assignment under a branch never taken,
//! after a RETURN, or with a WHERE that matches nothing still counts. Each manifest says so where its
//! reader will see it.

/// Lowercased tokens of `sql`. `--` and `/* */` comments are dropped; a single-quoted literal
/// (with `''` escapes) is ONE token, kept verbatim, so text inside a RAISE message can never read
/// as a statement. `,` `=` `(` `)` `;` `::` `||` are tokens of their own, so `SET a = 1, b = 2;`
/// reads as `set a = 1 , b = 2 ;` and `'{}'::jsonb||x` as `'{}' :: jsonb || x`, spaced or not.
pub fn tokens(sql: &str) -> Vec<String> {
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
pub const ERASURE_CONSTANTS: &[&str] = &["null", "''", "'{}'", "'\"erased\"'"];

pub fn is_ident(s: &str) -> bool {
    s.chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Whether `token` names the row's own identity or the act's resource: `p_resource`, `<alias>.id`,
/// a key number `<alias>.n`, or the event that asserted the row, `<alias>.asserted_by_event_id`
/// (the per-event sentinel, D4: an artifact family's `erased:<event_id>`). These are what a
/// sentinel may carry.
pub fn is_identity(token: &str) -> bool {
    token == "p_resource"
        || token.split_once('.').is_some_and(|(alias, col)| {
            is_ident(alias) && matches!(col, "id" | "n" | "asserted_by_event_id")
        })
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
pub fn is_erasure_value(expr: &[String]) -> bool {
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
pub fn is_literal_dims(token: &str) -> bool {
    token
        .strip_prefix("array[")
        .and_then(|rest| rest.strip_suffix(']'))
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// The tokens of a SET value starting at `start`, up to the `,` that ends it, the `;` that ends the
/// statement, or the `where` / `from` / `returning` that ends the SET list, all at paren depth 0.
pub fn value_expr(t: &[String], start: usize) -> &[String] {
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
pub fn assignments<'a>(t: &'a [String], key: &str) -> Vec<&'a [String]> {
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
pub fn reads_column(expr: &[String], column: &str) -> bool {
    expr.iter()
        .any(|tok| tok == column || tok.rsplit_once('.').is_some_and(|(_, col)| col == column))
}

pub fn deletes_from(t: &[String], table: &str) -> bool {
    t.windows(3)
        .any(|w| w[0] == "delete" && w[1] == "from" && w[2] == table)
}
