//! The shipped MCP skill (`skills/temper-knowledge-base/`) against the tool layer's declarations.
//!
//! A monorepo check: the skill tree is not part of the published tool layer, so the check lives
//! in the deployed shell, which the monorepo builds with the tree beside it.

/// **Every tool the shipped MCP skill tells an agent to call must actually exist.**
///
/// This closes the half of the skill-drift gate that gate cannot reach. That gate re-emits the
/// generated files and diffs them, so it proves the tree matches its source — it says nothing
/// about whether the source is *true*, and it does not look at `knowledge-base.md` at all,
/// which is hand-written. The skill shipped for months naming a `list_events` tool this server
/// has never exposed; an agent following it burns a turn on a call that cannot succeed.
///
/// It lives in the deployed shell because it reads the monorepo's skill tree, which the published
/// tool layer does not ship. The names it checks against are the tool layer's shipped
/// declarations (`temper_mcp::declarations::TOOLS_LIST`, held equal to the router by that crate's
/// own tests), with the blob door open, so every registered tool is in the set.
///
/// **What it covers:** every name in a `Tool:` / `Tools:` position — i.e. every worked
/// invocation example. That is where a wrong name actually costs an agent a failed call, and it
/// is exactly where `list_events` was.
///
/// **Declared remainder:** a tool named only in running prose or a bullet (`- `list_resources`
/// — paginated list`) is not checked. Distinguishing those from the many backticked *field*
/// names (`context_ref`, `open_meta`, `expected_blocks`, …) would need a hand-maintained
/// denylist, and a hand-maintained list of what-not-to-check is the same rot one level down.
#[test]
fn every_tool_the_shipped_skill_names_exists_in_the_router() {
    use std::collections::BTreeSet;

    let skill_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/temper-knowledge-base");

    // Walk one level plus `references/`; the bundle is deliberately shallow.
    let mut docs: Vec<(String, String)> = Vec::new();
    let mut dirs = vec![skill_dir.clone()];
    while let Some(dir) = dirs.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "md") {
                let body = std::fs::read_to_string(&path).expect("read skill doc");
                docs.push((path.display().to_string(), body));
            }
        }
    }

    assert!(
        !docs.is_empty(),
        "no markdown found under {} — this test would pass having checked nothing",
        skill_dir.display()
    );

    let declarations: serde_json::Value =
        serde_json::from_str(temper_mcp::declarations::TOOLS_LIST).expect("the fixture parses");
    let advertised: BTreeSet<String> = declarations["tools"]
        .as_array()
        .expect("the fixture lists tools")
        .iter()
        .map(|t| {
            t["name"]
                .as_str()
                .expect("every tool names itself")
                .to_string()
        })
        .collect();

    // Names in a `Tool:` / `Tools:` position. A line may carry several
    // (`Tools: `ingest_begin` → `ingest_append``), so take every backticked or bare
    // snake_case token up to the end of the segment.
    let mut checked = 0usize;
    let mut missing: Vec<String> = Vec::new();
    for (path, body) in &docs {
        for line in body.lines() {
            let Some(idx) = line.find("Tool:").or_else(|| line.find("Tools:")) else {
                continue;
            };
            let tail = &line[idx..];
            // Cut at the first boundary that ends the tool-name region. `Input:` matters most:
            // the worked examples put the payload on the SAME line, and without this every
            // field name (`doc_type_name`, `context_ref`, …) reads as a tool. `|` ends a
            // table cell so a row's prose column is never scanned.
            let end = ["Input:", "|"]
                .iter()
                .filter_map(|b| tail.find(b))
                .min()
                .unwrap_or(tail.len());
            let segment = &tail[..end];
            let after_anchor = segment.split_once(':').map_or("", |(_, rest)| rest);

            // Backticked names first — a line may carry several
            // (``Tools: `ingest_begin` → `ingest_append` ``). Falling back to the first bare
            // token covers the unbackticked `Tool: list_resources` form. Taking every bare
            // token instead would scoop up prose.
            let ticked: Vec<&str> = segment
                .split('`')
                .skip(1)
                .step_by(2)
                .map(str::trim)
                .collect();
            let candidates: Vec<&str> = if ticked.is_empty() {
                after_anchor.split_whitespace().take(1).collect()
            } else {
                ticked
            };

            for token in candidates
                .into_iter()
                .filter(|t| t.len() > 2 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            {
                checked += 1;
                if !advertised.contains(token) {
                    missing.push(format!("{path}: `{token}`"));
                }
            }
        }
    }

    // A rewording that removes the `Tool:` anchor would silently reduce this test to nothing.
    // Refuse instead — a check that cannot fail reads as coverage.
    assert!(
        checked >= 10,
        "only {checked} tool references found across {} skill docs; the `Tool:` anchor this \
         test extracts on has probably been reworded, leaving it checking nothing",
        docs.len()
    );

    assert!(
        missing.is_empty(),
        "the shipped MCP skill names {} tool(s) this server does not advertise:\n  {}\n\
         Advertised tools: {:?}",
        missing.len(),
        missing.join("\n  "),
        advertised
    );
}
