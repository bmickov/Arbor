//! Files in the repository: the tracked file list (for Quick Actions).

use super::{open, Result};

/// Every tracked file (paths from the index), sorted.
pub fn tracked_files(repo_path: &str) -> Result<Vec<String>> {
    let repo = open(repo_path)?;
    let index = repo.index()?;
    let mut out: Vec<String> = index.iter().map(|e| String::from_utf8_lossy(&e.path).to_string()).collect();
    out.dedup(); // conflicted files appear once per stage
    Ok(out)
}
