use tempfile::TempDir;

#[test]
fn test_count_md_files_recursive() {
    let dir = TempDir::new().unwrap();

    let project_a = dir.path().join("project_a");
    let project_b = dir.path().join("project_b");
    std::fs::create_dir_all(&project_a).unwrap();
    std::fs::create_dir_all(&project_b).unwrap();

    std::fs::write(project_a.join("file1.md"), "# File 1").unwrap();
    std::fs::write(project_a.join("file2.md"), "# File 2").unwrap();
    std::fs::write(project_b.join("file3.md"), "# File 3").unwrap();
    std::fs::write(project_b.join("not-md.txt"), "skip me").unwrap();

    let count = temper_cli::commands::status::count_md_files(dir.path());
    assert_eq!(count, 3, "should count all .md files recursively");
}

#[test]
fn test_count_md_files_empty_dir() {
    let dir = TempDir::new().unwrap();
    let count = temper_cli::commands::status::count_md_files(dir.path());
    assert_eq!(count, 0);
}

#[test]
fn test_count_md_files_nonexistent_dir() {
    let count = temper_cli::commands::status::count_md_files(std::path::Path::new("/nonexistent"));
    assert_eq!(count, 0);
}
