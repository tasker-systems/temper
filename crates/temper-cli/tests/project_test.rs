use std::collections::HashMap;
use std::path::PathBuf;
use temper_cli::config::ResolvedProject;
use temper_cli::project;

#[test]
fn test_resolve_from_cwd_exact_match() {
    let mut projects = HashMap::new();
    projects.insert(
        "myapp".to_string(),
        ResolvedProject {
            name: "myapp".to_string(),
            repo: "org/myapp".to_string(),
            path: PathBuf::from("/tmp/projects/myapp"),
        },
    );
    let result = project::resolve_from_cwd(&PathBuf::from("/tmp/projects/myapp/src"), &projects);
    assert_eq!(result.unwrap().name, "myapp");
}

#[test]
fn test_resolve_from_cwd_no_match() {
    let projects = HashMap::new();
    let result = project::resolve_from_cwd(&PathBuf::from("/tmp/unrelated"), &projects);
    assert!(result.is_none());
}

#[test]
fn test_resolve_most_specific_match() {
    let mut projects = HashMap::new();
    projects.insert(
        "parent".to_string(),
        ResolvedProject {
            name: "parent".to_string(),
            repo: String::new(),
            path: PathBuf::from("/tmp/projects"),
        },
    );
    projects.insert(
        "child".to_string(),
        ResolvedProject {
            name: "child".to_string(),
            repo: String::new(),
            path: PathBuf::from("/tmp/projects/child"),
        },
    );
    let result = project::resolve_from_cwd(&PathBuf::from("/tmp/projects/child/src"), &projects);
    assert_eq!(result.unwrap().name, "child");
}
