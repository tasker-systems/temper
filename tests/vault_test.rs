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
