use std::collections::HashMap;
use std::path::Path;

use crate::config::ResolvedProject;

/// Resolve a project from a working directory by matching against project paths.
/// Returns the most specific (longest path) match.
pub fn resolve_from_cwd<'a>(
    cwd: &Path,
    projects: &'a HashMap<String, ResolvedProject>,
) -> Option<&'a ResolvedProject> {
    let mut best: Option<(&ResolvedProject, usize)> = None;
    for project in projects.values() {
        if cwd.starts_with(&project.path) {
            let depth = project.path.components().count();
            if best.is_none() || depth > best.unwrap().1 {
                best = Some((project, depth));
            }
        }
    }
    best.map(|(p, _)| p)
}
