//! Diffs for the working copy, the index and commits.

use super::{oid, open, Result};
use git2::{Delta, Diff, DiffFindOptions, DiffOptions, Patch, Repository};
use serde::{Deserialize, Serialize};

/// Files with more lines than this are summarized instead of rendered.
const MAX_LINES: usize = 25_000;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    /// ' ' context, '+' added, '-' removed
    pub origin: char,
    pub content: String,
    /// False when the line has no trailing newline (end of file).
    pub nl: bool,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<Line>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: &'static str,
    pub binary: bool,
    pub too_large: bool,
    pub additions: usize,
    pub deletions: usize,
    pub old_mode: u32,
    pub new_mode: u32,
    pub hunks: Vec<Hunk>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiffSettings {
    pub context: Option<u32>,
    pub ignore_whitespace: Option<bool>,
}

fn status_name(d: Delta) -> &'static str {
    match d {
        Delta::Added => "added",
        Delta::Deleted => "deleted",
        Delta::Renamed => "renamed",
        Delta::Copied => "copied",
        Delta::Typechange => "typechange",
        Delta::Untracked => "untracked",
        Delta::Conflicted => "conflicted",
        _ => "modified",
    }
}

pub fn options(s: DiffSettings) -> DiffOptions {
    let mut o = DiffOptions::new();
    o.context_lines(s.context.unwrap_or(3)).indent_heuristic(true);
    if s.ignore_whitespace.unwrap_or(false) {
        o.ignore_whitespace_change(true);
    }
    o
}

fn find_renames(diff: &mut Diff) -> Result<()> {
    let mut f = DiffFindOptions::new();
    f.renames(true).copies(false);
    diff.find_similar(Some(&mut f))?;
    Ok(())
}

/// Convert delta `idx` of `diff` into a FileDiff. With `with_lines` false only
/// the summary (paths, status, counts) is filled in.
pub fn file_diff(diff: &Diff, idx: usize, with_lines: bool) -> Result<FileDiff> {
    let delta = diff.get_delta(idx).expect("delta index in range");
    let new_path = delta.new_file().path().map(|p| p.to_string_lossy().to_string());
    let old_path = delta.old_file().path().map(|p| p.to_string_lossy().to_string());
    let path = new_path.clone().or(old_path.clone()).unwrap_or_default();
    let mut fd = FileDiff {
        path: path.clone(),
        old_path: old_path.filter(|o| *o != path),
        status: status_name(delta.status()),
        binary: false,
        too_large: false,
        additions: 0,
        deletions: 0,
        old_mode: u32::from(delta.old_file().mode()),
        new_mode: u32::from(delta.new_file().mode()),
        hunks: vec![],
    };
    let Some(patch) = Patch::from_diff(diff, idx)? else {
        fd.binary = true;
        return Ok(fd);
    };
    if patch.delta().flags().is_binary() {
        fd.binary = true;
        return Ok(fd);
    }
    let (_, adds, dels) = patch.line_stats()?;
    fd.additions = adds;
    fd.deletions = dels;
    if !with_lines {
        return Ok(fd);
    }
    let mut total = 0;
    for h in 0..patch.num_hunks() {
        total += patch.num_lines_in_hunk(h)?;
    }
    if total > MAX_LINES {
        fd.too_large = true;
        return Ok(fd);
    }
    for h in 0..patch.num_hunks() {
        let (hunk, n) = patch.hunk(h)?;
        let mut lines: Vec<Line> = Vec::with_capacity(n);
        for l in 0..n {
            let line = patch.line_in_hunk(h, l)?;
            let origin = line.origin();
            match origin {
                ' ' | '+' | '-' => {
                    let raw = String::from_utf8_lossy(line.content());
                    let nl = raw.ends_with('\n');
                    let content = raw.trim_end_matches('\n').trim_end_matches('\r').to_string();
                    lines.push(Line { origin, content, nl, old_no: line.old_lineno(), new_no: line.new_lineno() });
                }
                // "\ No newline at end of file" markers: already captured by `nl`.
                _ => {}
            }
        }
        fd.hunks.push(Hunk {
            header: String::from_utf8_lossy(hunk.header()).trim_end().to_string(),
            old_start: hunk.old_start(),
            old_lines: hunk.old_lines(),
            new_start: hunk.new_start(),
            new_lines: hunk.new_lines(),
            lines,
        });
    }
    Ok(fd)
}

fn pick(diff: &Diff, path: &str) -> Result<Option<FileDiff>> {
    for i in 0..diff.deltas().len() {
        let d = diff.get_delta(i).unwrap();
        let np = d.new_file().path().map(|p| p.to_string_lossy().to_string());
        let op = d.old_file().path().map(|p| p.to_string_lossy().to_string());
        if np.as_deref() == Some(path) || (np.is_none() && op.as_deref() == Some(path)) {
            return Ok(Some(file_diff(diff, i, true)?));
        }
    }
    Ok(None)
}

/// Raw index→workdir diff for one path (untracked files included).
pub fn raw_unstaged<'a>(repo: &'a Repository, path: &str, s: DiffSettings) -> Result<Diff<'a>> {
    let mut o = options(s);
    o.include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true)
        .disable_pathspec_match(true)
        .pathspec(path);
    Ok(repo.diff_index_to_workdir(None, Some(&mut o))?)
}

/// Raw HEAD→index diff for one path (and its rename source, when given).
pub fn raw_staged<'a>(repo: &'a Repository, path: &str, old_path: Option<&str>, s: DiffSettings) -> Result<Diff<'a>> {
    let mut o = options(s);
    o.disable_pathspec_match(true).pathspec(path);
    if let Some(op) = old_path {
        o.pathspec(op);
    }
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let mut d = repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut o))?;
    if old_path.is_some() {
        find_renames(&mut d)?;
    }
    Ok(d)
}

pub fn unstaged_file(repo_path: &str, path: &str, s: DiffSettings) -> Result<Option<FileDiff>> {
    let repo = open(repo_path)?;
    let d = raw_unstaged(&repo, path, s)?;
    pick(&d, path)
}

pub fn staged_file(repo_path: &str, path: &str, old_path: Option<&str>, s: DiffSettings) -> Result<Option<FileDiff>> {
    let repo = open(repo_path)?;
    let d = raw_staged(&repo, path, old_path, s)?;
    pick(&d, path)
}

/// Diffs that make up a commit: against its first parent (or nothing for a
/// root commit). For stashes, untracked files from the third parent are added.
fn commit_diffs<'a>(repo: &'a Repository, id: &str, s: DiffSettings) -> Result<Vec<Diff<'a>>> {
    let c = repo.find_commit(oid(id)?)?;
    let tree = c.tree()?;
    let parent_tree = if c.parent_count() > 0 { Some(c.parent(0)?.tree()?) } else { None };
    let mut o = options(s);
    let mut d = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut o))?;
    find_renames(&mut d)?;
    let mut out = vec![d];
    let is_stash = c.parent_count() == 3
        && c.message().map(|m| m.starts_with("WIP on") || m.starts_with("On ")).unwrap_or(false);
    if is_stash {
        let untracked = c.parent(2)?.tree()?;
        let mut o = options(s);
        out.push(repo.diff_tree_to_tree(None, Some(&untracked), Some(&mut o))?);
    }
    Ok(out)
}

/// Summaries (no lines) of every file changed by a commit.
pub fn commit_files(repo_path: &str, id: &str) -> Result<Vec<FileDiff>> {
    let repo = open(repo_path)?;
    let mut out = vec![];
    for d in commit_diffs(&repo, id, DiffSettings::default())? {
        for i in 0..d.deltas().len() {
            let f = file_diff(&d, i, false)?;
            if out.iter().any(|x: &FileDiff| x.path == f.path) {
                continue;
            }
            out.push(f);
        }
    }
    Ok(out)
}

pub fn commit_file(repo_path: &str, id: &str, path: &str, s: DiffSettings) -> Result<Option<FileDiff>> {
    let repo = open(repo_path)?;
    for d in commit_diffs(&repo, id, s)? {
        if let Some(f) = pick(&d, path)? {
            return Ok(Some(f));
        }
    }
    Ok(None)
}

/// Content of a file at a revision (or the working tree when `rev` is None),
/// used for image previews and the full-file view.
pub fn file_at(repo_path: &str, rev: Option<&str>, path: &str) -> Result<Option<Vec<u8>>> {
    let repo = open(repo_path)?;
    match rev {
        None => {
            let full = repo.workdir().unwrap().join(path);
            Ok(std::fs::read(full).ok())
        }
        Some(r) => {
            let obj = repo.revparse_single(r)?;
            let tree = obj.peel_to_tree()?;
            match tree.get_path(std::path::Path::new(path)) {
                Ok(e) => Ok(Some(repo.find_blob(e.id())?.content().to_vec())),
                Err(_) => Ok(None),
            }
        }
    }
}
