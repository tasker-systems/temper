#[test]
fn test_parse_frontmatter() {
    let content = "---\ntype: session\ntitle: Test\n---\n\n# Body\n";
    let fm = temper_cli::vault::parse_frontmatter(content);
    assert!(fm.is_some());
    let fm = fm.unwrap();
    assert_eq!(fm.get("type").unwrap().as_str().unwrap(), "session");
}

#[test]
fn test_parse_frontmatter_none() {
    let content = "# Just a header\n";
    assert!(temper_cli::vault::parse_frontmatter(content).is_none());
}

#[test]
fn test_slugify() {
    assert_eq!(temper_cli::vault::slugify("Hello World!"), "hello-world");
    assert_eq!(temper_cli::vault::slugify("Fix: the bug"), "fix-the-bug");
}

#[test]
fn test_set_frontmatter_field() {
    let content = "---\nstage: backlog\ntitle: test\n---\n\n# Body\n";
    let updated = temper_cli::vault::set_frontmatter_field(content, "stage", "done");
    assert!(updated.contains("stage: done"));
    assert!(!updated.contains("stage: backlog"));
}

#[test]
fn test_extract_wikilinks() {
    let content = "See [[Concept A]] and [[Other|Display]] for details.";
    let links = temper_cli::vault::extract_wikilinks(content);
    assert_eq!(links, vec!["Concept A", "Other"]);
}

#[test]
fn test_rename_frontmatter_field() {
    let content = "---\nid: \"abc-123\"\ntype: task\ntitle: \"Hello\"\n---\n\n# Hello\n";
    let result = temper_cli::vault::rename_frontmatter_field(content, "id", "temper-id");
    assert!(result.contains("temper-id: \"abc-123\""), "got:\n{result}");
    assert!(!result.contains("\nid:"), "old key should be gone");
    assert!(result.contains("# Hello"), "body preserved");
}

#[test]
fn test_rename_frontmatter_field_preserves_body() {
    let content = "---\nstage: backlog\n---\n\nSome body with stage: info here.\n";
    let result = temper_cli::vault::rename_frontmatter_field(content, "stage", "temper-stage");
    assert!(result.contains("temper-stage: backlog"));
    assert!(
        result.contains("stage: info here"),
        "body line with 'stage:' should not be renamed"
    );
}

#[test]
fn test_remove_frontmatter_field() {
    let content = "---\nid: \"abc\"\ntype: task\ntitle: \"Hello\"\n---\n\n# Hello\n";
    let result = temper_cli::vault::remove_frontmatter_field(content, "type");
    assert!(!result.contains("\ntype:"), "field should be removed");
    assert!(result.contains("id: \"abc\""), "other fields preserved");
    assert!(result.contains("# Hello"), "body preserved");
}

#[test]
fn test_insert_frontmatter_field() {
    let content = "---\ntitle: \"Hello\"\n---\n\n# Hello\n";
    let result = temper_cli::vault::insert_frontmatter_field(content, "temper-id", "\"new-uuid\"");
    assert!(result.contains("temper-id: \"new-uuid\""));
    let id_pos = result.find("temper-id").unwrap();
    let title_pos = result.find("title:").unwrap();
    assert!(
        id_pos < title_pos,
        "inserted field should be at top of frontmatter"
    );
}
