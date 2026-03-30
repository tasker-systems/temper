pub fn run(
    path: &str,
    _dir: bool,
    _context: &str,
    _doc_type: &str,
    _format: &str,
    _force: bool,
) -> crate::error::Result<()> {
    if path.starts_with("http://") || path.starts_with("https://") {
        return Err(crate::error::TemperError::Config(
            "URL support not yet implemented. Please provide a file path.".to_string(),
        ));
    }
    eprintln!("temper add: not yet implemented");
    Ok(())
}
