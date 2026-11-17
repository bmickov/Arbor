//! Blame: which commit last changed each line of a file.

use super::{open, short, Result};
use git2::BlameOptions;
use serde::Serialize;
use std::path::Path;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BlameHunk {
    /// 1-based first line.
    pub start: usize,
    pub count: usize,
    pub oid: String,
    pub short: String,
    pub author: String,
    pub time: i64,
    pub summary: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Blame {
    pub lines: Vec<String>,
    pub hunks: Vec<BlameHunk>,
    pub binary: bool,
}

/// Blame `path` as committed at HEAD.
pub fn blame(repo_path: &str, path: &str) -> Result<Blame> {
    let repo = open(repo_path)?;
    let head = repo.head()?.peel_to_commit()?;
    let entry = head
        .tree()?
        .get_path(Path::new(path))
        .map_err(|_| super::Error::Msg(format!("{path} isn't in a commit yet, so there's no history to show for its lines.")))?;
    let blob = repo.find_blob(entry.id())?;
    if blob.is_binary() {
        return Ok(Blame { lines: vec![], hunks: vec![], binary: true });
    }
    let text = String::from_utf8_lossy(blob.content()).to_string();
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let mut opts = BlameOptions::new();
    opts.newest_commit(head.id()).track_copies_same_file(true);
    let b = repo.blame_file(Path::new(path), Some(&mut opts))?;
    let mut hunks = vec![];
    for h in b.iter() {
        let id = h.final_commit_id();
        let (author, time, summary) = match repo.find_commit(id) {
            Ok(c) => (
                c.author().name().unwrap_or("").to_string(),
                c.author().when().seconds(),
                super::summary(&c),
            ),
            Err(_) => (String::new(), 0, String::new()),
        };
        hunks.push(BlameHunk {
            start: h.final_start_line(),
            count: h.lines_in_hunk(),
            oid: id.to_string(),
            short: short(id),
            author,
            time,
            summary,
        });
    }
    Ok(Blame { lines, hunks, binary: false })
}
