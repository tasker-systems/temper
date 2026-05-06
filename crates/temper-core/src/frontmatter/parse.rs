//! Text block splitting, YAML parsing, and alias normalization at the
//! parse boundary.

use crate::error::{Result, TemperError};
use crate::frontmatter::registry::{lookup, KnownOpenField};

/// Lenient YAML frontmatter parse.
///
/// Unlike [`super::Frontmatter::try_from`], this function imposes no
/// semantic requirements on the parsed value — no `temper-type` check, no
/// doctype validation, no alias normalization. It returns the raw YAML
/// mapping as [`serde_yaml::Value`] on success or `None` for any of:
/// missing opening `---`, missing closing `---`, or YAML parse failure.
///
/// Intended for validator-style code that needs to inspect potentially
/// malformed or legacy-format files (e.g. the `temper doctor` scanner
/// that flags files with `type:` instead of `temper-type:`) and for
/// ingest discovery readers that process foreign markdown files that may
/// not yet carry any temper-* frontmatter at all.
pub fn parse_yaml_block(content: &str) -> Option<serde_yaml::Value> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }
    let rest = &content[3..];
    let end = rest.find("---")?;
    let yaml_str = &rest[..end];
    serde_yaml::from_str(yaml_str).ok()
}

/// Split a vault file into (yaml_frontmatter_text, body).
///
/// Requires the file to begin with `---` (optionally preceded by a UTF-8 BOM)
/// and contain a closing `---` on its own line. Body is returned byte-for-byte.
pub fn split_frontmatter_block(content: &str) -> Result<(String, String)> {
    let stripped = content.strip_prefix('\u{feff}').unwrap_or(content);

    let after_open = stripped
        .strip_prefix("---\n")
        .or_else(|| stripped.strip_prefix("---\r\n"))
        .ok_or_else(|| {
            TemperError::Config("missing frontmatter block: file must begin with '---'".to_string())
        })?;

    let close_idx = find_closing_fence(after_open).ok_or_else(|| {
        TemperError::Config("unterminated frontmatter block: missing closing '---'".to_string())
    })?;

    let yaml_text = after_open[..close_idx].to_string();
    let after_yaml = &after_open[close_idx..];

    let body = after_yaml
        .strip_prefix("---\n")
        .or_else(|| after_yaml.strip_prefix("---\r\n"))
        .or_else(|| after_yaml.strip_prefix("---"))
        .unwrap_or("")
        .to_string();

    Ok((yaml_text, body))
}

/// Locate the byte offset of the closing `---` fence inside `after_open`.
fn find_closing_fence(after_open: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(rel) = after_open[search_from..].find("---") {
        let abs = search_from + rel;
        let at_line_start = abs == 0 || after_open.as_bytes()[abs - 1] == b'\n';
        let after = &after_open[abs + 3..];
        let at_line_end = after.is_empty() || after.starts_with('\n') || after.starts_with("\r\n");
        if at_line_start && at_line_end {
            return Some(abs);
        }
        search_from = abs + 3;
    }
    None
}

/// Parse a YAML text block into a `serde_yaml::Value`. The root must be
/// a mapping — anything else is rejected.
pub fn parse_yaml(text: &str) -> Result<serde_yaml::Value> {
    let value: serde_yaml::Value = serde_yaml::from_str(text)
        .map_err(|e| TemperError::Config(format!("failed to parse YAML frontmatter: {e}")))?;
    if value.as_mapping().is_none() {
        return Err(TemperError::Config(
            "frontmatter is not a YAML mapping".to_string(),
        ));
    }
    Ok(value)
}

/// Rewrite known hyphen-form aliases to their canonical underscore form.
///
/// Operates in place on the top-level mapping. Non-mapping values and
/// unknown hyphen keys are left alone. If both the alias and the canonical
/// form are present (unlikely but possible after hand edits), the canonical
/// value wins and the alias is dropped.
pub fn normalize_aliases(value: &mut serde_yaml::Value) {
    let Some(mapping) = value.as_mapping_mut() else {
        return;
    };

    // Collect (alias_key, canonical_key) pairs first — we can't mutate
    // while iterating. Source: open-field hyphen-form aliases
    // (e.g. relates-to → relates_to), via the open-field registry.
    let mut rewrites: Vec<(serde_yaml::Value, serde_yaml::Value)> = Vec::new();
    for (k, _) in mapping.iter() {
        let Some(k_str) = k.as_str() else {
            continue;
        };
        if let Some(entry) = alias_target(k_str) {
            if entry.canonical != k_str {
                rewrites.push((
                    k.clone(),
                    serde_yaml::Value::String(entry.canonical.to_string()),
                ));
            }
        }
    }

    for (alias_key, canonical_key) in rewrites {
        // If canonical is already present, drop the alias (canonical wins).
        if mapping.contains_key(&canonical_key) {
            mapping.remove(&alias_key);
            continue;
        }
        // Otherwise rename: remove old, insert new with old's value.
        if let Some(val) = mapping.remove(&alias_key) {
            mapping.insert(canonical_key, val);
        }
    }
}

/// Look up whether `key` matches any known open field (canonical or alias).
fn alias_target(key: &str) -> Option<&'static KnownOpenField> {
    lookup(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_block_strips_opening_and_closing_fences() {
        let input = "---\na: 1\n---\nbody\n";
        let (yaml, body) = split_frontmatter_block(input).expect("split ok");
        assert_eq!(yaml, "a: 1\n");
        assert_eq!(body, "body\n");
    }

    #[test]
    fn split_block_handles_bom() {
        let input = "\u{feff}---\na: 1\n---\n";
        let (yaml, body) = split_frontmatter_block(input).expect("split ok");
        assert_eq!(yaml, "a: 1\n");
        assert_eq!(body, "");
    }

    #[test]
    fn split_block_rejects_missing_opening_fence() {
        let input = "no frontmatter here\n";
        assert!(split_frontmatter_block(input).is_err());
    }

    #[test]
    fn split_block_rejects_unterminated_block() {
        let input = "---\na: 1\n";
        assert!(split_frontmatter_block(input).is_err());
    }

    #[test]
    fn split_block_preserves_body_byte_for_byte() {
        let input = "---\nk: v\n---\nline1\nline2\n\nline4\n";
        let (_, body) = split_frontmatter_block(input).expect("ok");
        assert_eq!(body, "line1\nline2\n\nline4\n");
    }

    #[test]
    fn parse_yaml_succeeds_for_mapping() {
        let value = parse_yaml("a: 1\nb: [x, y]\n").expect("parse ok");
        assert!(value.as_mapping().is_some());
    }

    #[test]
    fn parse_yaml_errors_on_non_mapping_root() {
        assert!(parse_yaml("- just\n- a\n- list\n").is_err());
    }

    #[test]
    fn normalize_aliases_rewrites_hyphen_form_keys() {
        let mut v: serde_yaml::Value =
            serde_yaml::from_str("relates-to: [a]\ndepends-on: [b]\nparent: c\n").unwrap();
        normalize_aliases(&mut v);
        let m = v.as_mapping().unwrap();
        assert!(m.contains_key(serde_yaml::Value::String("relates_to".into())));
        assert!(m.contains_key(serde_yaml::Value::String("depends_on".into())));
        assert!(!m.contains_key(serde_yaml::Value::String("relates-to".into())));
        assert!(!m.contains_key(serde_yaml::Value::String("depends-on".into())));
    }

    #[test]
    fn normalize_aliases_preserves_values() {
        let mut v: serde_yaml::Value = serde_yaml::from_str("relates-to: [a, b, c]\n").unwrap();
        normalize_aliases(&mut v);
        let list = v
            .as_mapping()
            .unwrap()
            .get(serde_yaml::Value::String("relates_to".into()))
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn normalize_aliases_is_idempotent() {
        let mut v: serde_yaml::Value = serde_yaml::from_str("relates-to: [a]\n").unwrap();
        normalize_aliases(&mut v);
        let before = v.clone();
        normalize_aliases(&mut v);
        assert_eq!(before, v);
    }

    #[test]
    fn normalize_aliases_ignores_unknown_hyphen_keys() {
        let mut v: serde_yaml::Value = serde_yaml::from_str("my-custom-field: value\n").unwrap();
        let before = v.clone();
        normalize_aliases(&mut v);
        assert_eq!(before, v);
    }

    #[test]
    fn normalize_aliases_collision_prefers_canonical_form() {
        // If both alias and canonical are present (unlikely but possible
        // after hand edits), keep the canonical value and drop the alias.
        let mut v: serde_yaml::Value =
            serde_yaml::from_str("relates_to: [canonical]\nrelates-to: [alias]\n").unwrap();
        normalize_aliases(&mut v);
        let m = v.as_mapping().unwrap();
        assert!(!m.contains_key(serde_yaml::Value::String("relates-to".into())));
        let list = m
            .get(serde_yaml::Value::String("relates_to".into()))
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(list[0].as_str().unwrap(), "canonical");
    }
}
