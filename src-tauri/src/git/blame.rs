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
    // `git blame` is several times faster than libgit2's on long histories
    // (and honours blame.ignoreRevsFile); libgit2 is the fallback.
    let hunks = match blame_with_git(&repo, head.id(), path) {
        Some(h) => h,
        None => blame_hunks_libgit2(&repo, head.id(), path)?,
    };
    Ok(Blame { lines, hunks, binary: false })
}

/// Blame through the `git` program: None if it isn't there or fails.
fn blame_with_git(repo: &git2::Repository, head: git2::Oid, path: &str) -> Option<Vec<BlameHunk>> {
    let dir = repo.workdir().unwrap_or(repo.path());
    let out = crate::platform::quiet_command("git")
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        // The plain file, as libgit2 would blame it: no textconv programs, and
        // nothing the repository's config would start (core.fsmonitor).
        .args(["-c", "core.fsmonitor=false", "blame", "--porcelain", "--no-textconv", "-M", &head.to_string(), "--", path])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_porcelain(&String::from_utf8_lossy(&out.stdout))
}

/// `git blame --porcelain` → hunks, adjacent lines from one commit merged.
pub fn parse_porcelain(text: &str) -> Option<Vec<BlameHunk>> {
    struct Meta {
        author: String,
        time: i64,
        summary: String,
    }
    let mut meta: std::collections::HashMap<String, Meta> = Default::default();
    // (commit, final line) for every line, in order.
    let mut lines: Vec<(String, usize)> = vec![];
    let mut current: Option<(String, usize)> = None;
    for l in text.lines() {
        if let Some(_content) = l.strip_prefix('\t') {
            lines.push(current.take()?);
            continue;
        }
        let mut f = l.split(' ');
        let first = f.next().unwrap_or("");
        if first.len() == 40 && first.bytes().all(|b| b.is_ascii_hexdigit()) {
            let _orig = f.next()?;
            let fin: usize = f.next()?.parse().ok()?;
            meta.entry(first.to_string()).or_insert(Meta { author: String::new(), time: 0, summary: String::new() });
            current = Some((first.to_string(), fin));
        } else if let Some((sha, _)) = &current {
            let m = meta.get_mut(sha)?;
            let rest = l.split_once(' ').map(|(_, v)| v).unwrap_or("");
            match first {
                "author" => m.author = rest.to_string(),
                "author-time" => m.time = rest.parse().unwrap_or(0),
                "summary" => m.summary = rest.to_string(),
                _ => {}
            }
        }
    }
    lines.sort_by_key(|(_, n)| *n);
    let mut hunks: Vec<BlameHunk> = vec![];
    for (sha, n) in lines {
        if let Some(h) = hunks.last_mut() {
            if h.oid == sha && h.start + h.count == n {
                h.count += 1;
                continue;
            }
        }
        let m = meta.get(&sha)?;
        let id = git2::Oid::from_str(&sha).ok()?;
        hunks.push(BlameHunk { start: n, count: 1, oid: sha.clone(), short: short(id), author: m.author.clone(), time: m.time, summary: m.summary.clone() });
    }
    Some(hunks)
}

/// Blame with libgit2.
pub fn blame_hunks_libgit2(repo: &git2::Repository, head: git2::Oid, path: &str) -> Result<Vec<BlameHunk>> {
    let mut opts = BlameOptions::new();
    opts.newest_commit(head).track_copies_same_file(true);
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
    Ok(hunks)
}
