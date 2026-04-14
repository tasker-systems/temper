#[test]
fn test_slugify() {
    assert_eq!(temper_cli::vault::slugify("Hello World!"), "hello-world");
    assert_eq!(temper_cli::vault::slugify("Fix: the bug"), "fix-the-bug");
}

#[test]
fn test_extract_wikilinks() {
    let content = "See [[Concept A]] and [[Other|Display]] for details.";
    let links = temper_cli::vault::extract_wikilinks(content);
    assert_eq!(links, vec!["Concept A", "Other"]);
}
