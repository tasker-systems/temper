//! Hardcoded display column registry for CLI table output.
//!
//! Columns are curated per doc type — which to display, in what order, with
//! what width and alignment. The set of fields that *exist* comes from the
//! schema; the selection/ordering is code-owned for layout stability.

use serde_json::Value;

use super::table::{Alignment, Column};

/// Ordered list of columns to render for `doc_type` in table formats.
pub fn display_columns(doc_type: &str) -> Vec<Column> {
    let mut cols = vec![
        Column::new("Context", 16, Alignment::Left),
        Column::new("Type", 10, Alignment::Left),
        Column::new("Slug", 40, Alignment::Left),
        Column::new("Updated", 12, Alignment::Left),
    ];
    match doc_type {
        "task" => {
            cols.push(Column::new("Stage", 12, Alignment::Left));
            cols.push(Column::new("Mode", 6, Alignment::Left));
            cols.push(Column::new("Effort", 7, Alignment::Left));
            cols.push(Column::new("Goal", 16, Alignment::Left));
        }
        "goal" => {
            cols.push(Column::new("Status", 10, Alignment::Left));
            cols.push(Column::new("Seq", 4, Alignment::Right));
        }
        "session" | "research" | "concept" | "decision" => {}
        _ => return Vec::new(),
    }
    cols
}

/// Map a column header back to the frontmatter key it reads from.
fn field_key_for(header: &str) -> &'static str {
    match header {
        "Context" => "temper-context",
        "Type" => "temper-type",
        "Slug" => "temper-slug",
        "Updated" => "temper-updated",
        "Stage" => "temper-stage",
        "Mode" => "temper-mode",
        "Effort" => "temper-effort",
        "Goal" => "temper-goal",
        "Status" => "temper-status",
        "Seq" => "temper-seq",
        _ => "",
    }
}

/// Extract stringified cells from frontmatter for the given columns.
///
/// Missing fields render as empty strings. `temper-updated` is formatted as
/// YYYY-MM-DD (date-only) from an RFC3339 timestamp.
pub fn extract_row(frontmatter: &Value, columns: &[Column]) -> Vec<String> {
    columns
        .iter()
        .map(|col| {
            let key = field_key_for(&col.header);
            let raw = frontmatter.get(key);
            let text = match raw {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::Bool(b)) => b.to_string(),
                Some(Value::Null) | None => String::new(),
                Some(other) => other.to_string(),
            };
            if col.header == "Updated" {
                date_only(&text)
            } else {
                text
            }
        })
        .collect()
}

/// Truncate an RFC3339 timestamp to YYYY-MM-DD. If the string doesn't look
/// like a date, return it unchanged.
fn date_only(s: &str) -> String {
    if s.len() >= 10
        && s.as_bytes()[4] == b'-'
        && s.as_bytes()[7] == b'-'
        && s[..4].chars().all(|c| c.is_ascii_digit())
        && s[5..7].chars().all(|c| c.is_ascii_digit())
        && s[8..10].chars().all(|c| c.is_ascii_digit())
    {
        return s[..10].to_string();
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn task_columns_include_universal_and_extras() {
        let cols = display_columns("task");
        let headers: Vec<&str> = cols.iter().map(|c| c.header.as_str()).collect();
        assert_eq!(
            headers,
            vec!["Context", "Type", "Slug", "Updated", "Stage", "Mode", "Effort", "Goal"]
        );
    }

    #[test]
    fn goal_columns_have_status_and_seq() {
        let cols = display_columns("goal");
        let headers: Vec<&str> = cols.iter().map(|c| c.header.as_str()).collect();
        assert_eq!(
            headers,
            vec!["Context", "Type", "Slug", "Updated", "Status", "Seq"]
        );
    }

    #[test]
    fn session_columns_universal_only() {
        let cols = display_columns("session");
        assert_eq!(cols.len(), 4);
    }

    #[test]
    fn unknown_type_returns_empty() {
        let cols = display_columns("widget");
        assert!(cols.is_empty());
    }

    #[test]
    fn extract_row_populates_known_fields() {
        let cols = display_columns("task");
        let fm = json!({
            "temper-context": "temper",
            "temper-type": "task",
            "temper-slug": "2026-04-07-thing",
            "temper-updated": "2026-04-07T12:34:56Z",
            "temper-stage": "in-progress",
            "temper-mode": "build",
            "temper-effort": "medium",
            "temper-goal": "core",
        });
        let row = extract_row(&fm, &cols);
        assert_eq!(row[0], "temper");
        assert_eq!(row[1], "task");
        assert_eq!(row[2], "2026-04-07-thing");
        assert_eq!(
            row[3], "2026-04-07",
            "RFC3339 should be truncated to YYYY-MM-DD"
        );
        assert_eq!(row[4], "in-progress");
    }

    #[test]
    fn extract_row_missing_fields_render_empty() {
        let cols = display_columns("task");
        let fm = json!({ "temper-context": "temper", "temper-slug": "x" });
        let row = extract_row(&fm, &cols);
        assert_eq!(row[0], "temper");
        assert_eq!(row[1], "");
        assert_eq!(row[2], "x");
        assert_eq!(row[3], "");
    }

    #[test]
    fn extract_row_non_rfc3339_updated_left_as_is() {
        let cols = display_columns("task");
        let fm = json!({ "temper-updated": "unknown" });
        let row = extract_row(&fm, &cols);
        assert_eq!(row[3], "unknown");
    }

    #[test]
    fn seq_numeric_column_is_right_aligned() {
        let cols = display_columns("goal");
        let seq_col = cols.iter().find(|c| c.header == "Seq").unwrap();
        assert_eq!(seq_col.alignment, Alignment::Right);
    }
}
