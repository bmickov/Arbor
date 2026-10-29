//! Staging, unstaging and discarding: whole files, hunks or single lines.
//!
//! Partial operations build a unified diff containing only the selected
//! lines and apply it with libgit2 — to the index for stage/unstage, to the
//! working tree for discard.

use super::diff::{self, DiffSettings, FileDiff, Hunk};
use super::undo::{self, Action};
use super::{err, open, Error, Result};
use git2::{build::CheckoutBuilder, ApplyLocation, Diff, Repository};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Deserialize, Debug, Clone)]
pub struct Selection {
    /// Index of the hunk in the diff the UI showed.
    pub hunk: usize,
    /// Indexes into that hunk's `lines` (only '+' and '-' lines matter).
    pub lines: Vec<usize>,
}

fn label_for(paths: &[String], verb: &str) -> String {
    match paths.len() {
        1 => format!("{verb} {}", paths[0]),
        n => format!("{verb} {n} files"),
    }
}

// ---------------------------------------------------------------- whole files

pub fn stage_files(repo_path: &str, paths: &[String]) -> Result<()> {
    let repo = open(repo_path)?;
    let before = undo::index_tree(&repo);
    let root = repo.workdir().unwrap().to_path_buf();
    let mut index = repo.index()?;
    for p in paths {
        let full = root.join(p);
        if full.symlink_metadata().is_ok() {
            index.add_path(std::path::Path::new(p))?;
        } else {
            index.remove_path(std::path::Path::new(p))?;
        }
    }
    index.write()?;
    if let Some(t) = before {
        undo::record(&repo, label_for(paths, "Stage"), Action::RestoreIndex { tree: t });
    }
    Ok(())
}

pub fn unstage_files(repo_path: &str, paths: &[String]) -> Result<()> {
    let repo = open(repo_path)?;
    let before = undo::index_tree(&repo);
    match repo.head().ok().and_then(|h| h.peel_to_commit().ok()) {
        Some(c) => repo.reset_default(Some(c.as_object()), paths.iter())?,
        None => {
            let mut index = repo.index()?;
            for p in paths {
                let _ = index.remove_path(std::path::Path::new(p));
            }
            index.write()?;
        }
    }
    if let Some(t) = before {
        undo::record(&repo, label_for(paths, "Unstage"), Action::RestoreIndex { tree: t });
    }
    Ok(())
}

pub fn stage_all(repo_path: &str) -> Result<()> {
    let repo = open(repo_path)?;
    let before = undo::index_tree(&repo);
    let mut index = repo.index()?;
    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
    index.update_all(["*"].iter(), None)?;
    index.write()?;
    if let Some(t) = before {
        undo::record(&repo, "Stage all changes", Action::RestoreIndex { tree: t });
    }
    Ok(())
}

pub fn unstage_all(repo_path: &str) -> Result<()> {
    let repo = open(repo_path)?;
    let before = undo::index_tree(&repo);
    match repo.head().ok().and_then(|h| h.peel_to_commit().ok()) {
        Some(c) => repo.reset(c.as_object(), git2::ResetType::Mixed, None)?,
        None => {
            let mut index = repo.index()?;
            index.clear()?;
            index.write()?;
        }
    }
    if let Some(t) = before {
        undo::record(&repo, "Unstage all changes", Action::RestoreIndex { tree: t });
    }
    Ok(())
}

/// Throw away unstaged changes to `paths`. Untracked files are deleted.
/// The old contents are kept as blobs so the discard can be undone.
pub fn discard_files(repo_path: &str, paths: &[String]) -> Result<()> {
    let repo = open(repo_path)?;
    let saved = undo::save_files(&repo, paths)?;
    let root = repo.workdir().unwrap().to_path_buf();
    let index = repo.index()?;
    for p in paths {
        let tracked = index.get_path(std::path::Path::new(p), 0).is_some();
        if tracked {
            let mut co = CheckoutBuilder::new();
            co.force().path(p).disable_pathspec_match(true);
            repo.checkout_index(None, Some(&mut co))?;
        } else {
            let full = root.join(p);
            if full.is_dir() {
                std::fs::remove_dir_all(&full)?;
            } else if full.symlink_metadata().is_ok() {
                std::fs::remove_file(&full)?;
            }
        }
    }
    undo::record(&repo, label_for(paths, "Discard changes to"), Action::RestoreFiles { files: saved });
    Ok(())
}

// --------------------------------------------------------------- partial ops

#[derive(Clone, Copy, PartialEq)]
enum Direction {
    /// Apply the selected part of an old→new diff to the old side.
    Forward,
    /// Revert the selected part of an old→new diff on the new side.
    Reverse,
}

fn selected_map(sel: &[Selection]) -> HashMap<usize, HashSet<usize>> {
    let mut m: HashMap<usize, HashSet<usize>> = HashMap::new();
    for s in sel {
        m.entry(s.hunk).or_default().extend(s.lines.iter().copied());
    }
    m
}

/// True when every added/removed line of the file is selected.
fn selects_everything(fd: &FileDiff, sel: &HashMap<usize, HashSet<usize>>) -> bool {
    fd.hunks.iter().enumerate().all(|(hi, h)| {
        h.lines.iter().enumerate().all(|(li, l)| l.origin == ' ' || sel.get(&hi).is_some_and(|s| s.contains(&li)))
    })
}

fn push_line(out: &mut String, origin: char, content: &str, nl: bool) {
    out.push(origin);
    out.push_str(content);
    out.push('\n');
    if !nl {
        out.push_str("\\ No newline at end of file\n");
    }
}

/// Build the body (hunks only) of a patch for the selected lines.
fn build_hunks(hunks: &[Hunk], sel: &HashMap<usize, HashSet<usize>>, dir: Direction) -> Option<String> {
    let mut out = String::new();
    let mut delta: i64 = 0;
    let mut any = false;
    for (hi, h) in hunks.iter().enumerate() {
        let Some(chosen) = sel.get(&hi) else { continue };
        if !h.lines.iter().enumerate().any(|(li, l)| l.origin != ' ' && chosen.contains(&li)) {
            continue;
        }
        let mut body = String::new();
        let (mut pre, mut post) = (0i64, 0i64);
        for (li, l) in h.lines.iter().enumerate() {
            let picked = chosen.contains(&li);
            let mapped = match (dir, l.origin, picked) {
                (_, ' ', _) => Some(' '),
                (Direction::Forward, '-', true) => Some('-'),
                (Direction::Forward, '-', false) => Some(' '),
                (Direction::Forward, '+', true) => Some('+'),
                (Direction::Forward, '+', false) => None,
                (Direction::Reverse, '+', true) => Some('-'),
                (Direction::Reverse, '+', false) => Some(' '),
                (Direction::Reverse, '-', true) => Some('+'),
                (Direction::Reverse, '-', false) => None,
                _ => None,
            };
            if let Some(o) = mapped {
                match o {
                    ' ' => {
                        pre += 1;
                        post += 1
                    }
                    '-' => pre += 1,
                    _ => post += 1,
                }
                push_line(&mut body, o, &l.content, l.nl);
            }
        }
        // Where the preimage starts (1-based) in the file the patch applies to.
        let (start, count) = match dir {
            Direction::Forward => (h.old_start as i64, h.old_lines as i64),
            Direction::Reverse => (h.new_start as i64, h.new_lines as i64),
        };
        let first = if count == 0 { start + 1 } else { start };
        // libgit2 finds hunks by their position in the partly patched result,
        // so new_start must be exact.
        let new_start = first + delta;
        let old_hdr = if pre == 0 { first - 1 } else { first };
        out.push_str(&format!("@@ -{},{} +{},{} @@\n", old_hdr.max(0), pre, new_start.max(1), post));
        out.push_str(&body);
        delta += post - pre;
        any = true;
    }
    any.then_some(out)
}

fn apply_patch(repo: &Repository, text: &str, loc: ApplyLocation) -> Result<()> {
    let diff = Diff::from_buffer(text.as_bytes())?;
    repo.apply(&diff, loc, None).map_err(|e| {
        Error::Msg(format!(
            "Couldn't apply that selection ({}). Try selecting the whole hunk.",
            e.message()
        ))
    })
}

fn file_header(path: &str, new_file: bool) -> String {
    if new_file {
        format!("diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n")
    } else {
        format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n")
    }
}

pub fn stage_lines(repo_path: &str, path: &str, sel: &[Selection], s: DiffSettings) -> Result<()> {
    let repo = open(repo_path)?;
    let d = diff::raw_unstaged(&repo, path, s)?;
    let Some(fd) = (0..d.deltas().len()).find_map(|i| diff::file_diff(&d, i, true).ok().filter(|f| f.path == path)) else {
        return err("That file has no unstaged changes any more.");
    };
    let map = selected_map(sel);
    if selects_everything(&fd, &map) {
        return stage_files(repo_path, &[path.to_string()]);
    }
    let Some(body) = build_hunks(&fd.hunks, &map, Direction::Forward) else { return Ok(()) };
    let untracked = fd.status == "untracked" || fd.status == "added";
    let text = file_header(path, untracked) + &body;
    let before = undo::index_tree(&repo);
    apply_patch(&repo, &text, ApplyLocation::Index)?;
    if let Some(t) = before {
        undo::record(&repo, format!("Stage lines in {path}"), Action::RestoreIndex { tree: t });
    }
    Ok(())
}

pub fn unstage_lines(repo_path: &str, path: &str, old_path: Option<&str>, sel: &[Selection], s: DiffSettings) -> Result<()> {
    let repo = open(repo_path)?;
    let d = diff::raw_staged(&repo, path, old_path, s)?;
    let Some(fd) = (0..d.deltas().len()).find_map(|i| diff::file_diff(&d, i, true).ok().filter(|f| f.path == path)) else {
        return err("That file has no staged changes any more.");
    };
    let map = selected_map(sel);
    if selects_everything(&fd, &map) {
        let mut paths = vec![path.to_string()];
        if let Some(o) = &fd.old_path {
            paths.push(o.clone());
        }
        return unstage_files(repo_path, &paths);
    }
    let Some(body) = build_hunks(&fd.hunks, &map, Direction::Reverse) else { return Ok(()) };
    let text = file_header(path, false) + &body;
    let before = undo::index_tree(&repo);
    apply_patch(&repo, &text, ApplyLocation::Index)?;
    if let Some(t) = before {
        undo::record(&repo, format!("Unstage lines in {path}"), Action::RestoreIndex { tree: t });
    }
    Ok(())
}

pub fn discard_lines(repo_path: &str, path: &str, sel: &[Selection], s: DiffSettings) -> Result<()> {
    let repo = open(repo_path)?;
    let d = diff::raw_unstaged(&repo, path, s)?;
    let Some(fd) = (0..d.deltas().len()).find_map(|i| diff::file_diff(&d, i, true).ok().filter(|f| f.path == path)) else {
        return err("That file has no unstaged changes any more.");
    };
    let map = selected_map(sel);
    if selects_everything(&fd, &map) {
        return discard_files(repo_path, &[path.to_string()]);
    }
    let Some(body) = build_hunks(&fd.hunks, &map, Direction::Reverse) else { return Ok(()) };
    let text = file_header(path, false) + &body;
    let saved = undo::save_files(&repo, &[path.to_string()])?;
    apply_patch(&repo, &text, ApplyLocation::WorkDir)?;
    undo::record(&repo, format!("Discard lines in {path}"), Action::RestoreFiles { files: saved });
    Ok(())
}

/// Resolve a conflicted file by taking one side ("ours" or "theirs") and staging it.
pub fn resolve_conflict(repo_path: &str, path: &str, side: &str) -> Result<()> {
    let repo = open(repo_path)?;
    let index = repo.index()?;
    let conflict = index
        .conflicts()?
        .filter_map(|c| c.ok())
        .find(|c| {
            let p = c.our.as_ref().or(c.their.as_ref()).or(c.ancestor.as_ref());
            p.map(|e| String::from_utf8_lossy(&e.path) == path).unwrap_or(false)
        })
        .ok_or_else(|| Error::Msg(format!("{path} isn't conflicted.")))?;
    let entry = if side == "ours" { conflict.our } else { conflict.their };
    let full = repo.workdir().unwrap().join(path);
    match entry {
        Some(e) => {
            let blob = repo.find_blob(e.id)?;
            std::fs::write(&full, blob.content())?;
        }
        None => {
            // That side deleted the file.
            if full.exists() {
                std::fs::remove_file(&full)?;
            }
        }
    }
    let mut index = repo.index()?;
    if full.exists() {
        index.add_path(std::path::Path::new(path))?;
    } else {
        index.remove_path(std::path::Path::new(path))?;
    }
    index.write()?;
    Ok(())
}
