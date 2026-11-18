//! Staging, unstaging and discarding: whole files, hunks or single lines.
//!
//! Partial operations build a unified diff containing only the selected
//! lines and apply it with libgit2 — to the index for stage/unstage, to the
//! working tree for discard.

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
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
    // LFS files go through `git add`, which stores them as LFS pointers.
    let lfs = super::lfs::lfs_paths(&repo, paths);
    let lfs_set: HashSet<&String> = lfs.iter().collect();
    let mut index = repo.index()?;
    for p in paths {
        if lfs_set.contains(p) && root.join(p).exists() {
            continue;
        }
        let full = root.join(p);
        if full.symlink_metadata().is_ok() {
            index.add_path(std::path::Path::new(p))?;
        } else {
            index.remove_path(std::path::Path::new(p))?;
        }
    }
    index.write()?;
    let lfs: Vec<String> = lfs.into_iter().filter(|p| root.join(p).exists()).collect();
    super::lfs::add(&repo, &lfs)?;
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
    if super::lfs::used(&repo) {
        // Let git (and its LFS filter) do it.
        super::lfs::add(&repo, &[".".to_string()])?;
        let mut o = git2::StatusOptions::new();
        o.include_untracked(false);
        let deleted: Vec<String> = repo.statuses(Some(&mut o))?.iter().filter(|e| e.status().is_wt_deleted()).filter_map(|e| e.path().ok().map(str::to_string)).collect();
        if !deleted.is_empty() {
            let mut index = repo.index()?;
            for d in deleted {
                index.remove_path(std::path::Path::new(&d))?;
            }
            index.write()?;
        }
    } else {
        let mut index = repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.update_all(["*"].iter(), None)?;
        index.write()?;
    }
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

/// Remove the folders a deleted file leaves empty, up to the working tree,
/// as `git clean` does. Stops at the first folder that still holds anything.
fn remove_empty_parents(root: &std::path::Path, file: &std::path::Path) {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d == root || !d.starts_with(root) || std::fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

/// Put a tracked file back as the index has it.
fn restore_from_index(repo: &Repository, root: &std::path::Path, path: &str, entry: &git2::IndexEntry) -> Result<()> {
    let full = root.join(path);
    let link = entry.mode == 0o120000;
    // libgit2 won't turn a symlink back into a file (or the reverse) in place.
    if let Ok(m) = full.symlink_metadata() {
        if m.file_type().is_symlink() != link || m.is_dir() {
            if m.is_dir() {
                std::fs::remove_dir_all(&full)?;
            } else {
                std::fs::remove_file(&full)?;
            }
        }
    }
    let mut co = CheckoutBuilder::new();
    co.force().path(path).disable_pathspec_match(true);
    repo.checkout_index(None, Some(&mut co))?;
    // libgit2's checkout skips some names (a backslash in a path, for one);
    // write those out ourselves.
    let current = repo.status_file(std::path::Path::new(path)).map(|s| !s.intersects(git2::Status::WT_MODIFIED | git2::Status::WT_DELETED | git2::Status::WT_TYPECHANGE)).unwrap_or(false);
    if !current {
        let blob = repo.find_blob(entry.id)?;
        let _ = std::fs::remove_file(&full);
        if let Some(d) = full.parent() {
            std::fs::create_dir_all(d)?;
        }
        if link {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(blob.content()), &full)?;
            #[cfg(not(unix))]
            std::fs::write(&full, blob.content())?;
        } else {
            std::fs::write(&full, blob.content())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if entry.mode == 0o100755 { 0o755 } else { 0o644 };
                std::fs::set_permissions(&full, std::fs::Permissions::from_mode(mode))?;
            }
        }
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
        if let Some(entry) = index.get_path(std::path::Path::new(p), 0) {
            restore_from_index(&repo, &root, p, &entry)?;
        } else {
            let full = root.join(p);
            if full.is_dir() {
                std::fs::remove_dir_all(&full)?;
            } else if full.symlink_metadata().is_ok() {
                std::fs::remove_file(&full)?;
            }
            remove_empty_parents(&root, &full);
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

fn push_line(out: &mut Vec<u8>, origin: char, line: &diff::Line) {
    out.push(origin as u8);
    out.extend_from_slice(&line.bytes());
    out.push(b'\n');
    if !line.nl {
        out.extend_from_slice(b"\\ No newline at end of file\n");
    }
}

/// Build the body (hunks only) of a patch for the selected lines.
fn build_hunks(hunks: &[Hunk], sel: &HashMap<usize, HashSet<usize>>, dir: Direction) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut delta: i64 = 0;
    let mut any = false;
    for (hi, h) in hunks.iter().enumerate() {
        let Some(chosen) = sel.get(&hi) else { continue };
        if !h.lines.iter().enumerate().any(|(li, l)| l.origin != ' ' && chosen.contains(&li)) {
            continue;
        }
        let mut body = Vec::new();
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
                push_line(&mut body, o, l);
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
        out.extend_from_slice(format!("@@ -{},{} +{},{} @@\n", old_hdr.max(0), pre, new_start.max(1), post).as_bytes());
        out.extend_from_slice(&body);
        delta += post - pre;
        any = true;
    }
    any.then_some(out)
}

fn apply_patch(repo: &Repository, text: &[u8], loc: ApplyLocation) -> Result<()> {
    let diff = Diff::from_buffer(text)?;
    repo.apply(&diff, loc, None).map_err(|e| {
        Error::Msg(format!(
            "Couldn't apply that selection ({}). Try selecting the whole hunk.",
            e.message()
        ))
    })
}

/// `mode`: Some for a new file (its mode), None for an existing one.
fn file_header(path: &str, new_file: Option<u32>, body: Vec<u8>) -> Vec<u8> {
    let head = match new_file {
        Some(mode) => {
            let mode = if mode == 0o100755 { "100755" } else { "100644" };
            format!("diff --git a/{path} b/{path}\nnew file mode {mode}\n--- /dev/null\n+++ b/{path}\n")
        }
        None => format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n"),
    };
    let mut out = head.into_bytes();
    out.extend(body);
    out
}

pub fn stage_lines(repo_path: &str, path: &str, sel: &[Selection], s: DiffSettings) -> Result<()> {
    let repo = open(repo_path)?;
    let d = diff::raw_unstaged(&repo, path, s)?;
    let Some(fd) = (0..d.deltas().len()).find_map(|i| diff::file_diff(&d, i, true, None).ok().filter(|f| f.path == path)) else {
        return err("That file has no unstaged changes any more.");
    };
    let map = selected_map(sel);
    if selects_everything(&fd, &map) {
        return stage_files(repo_path, &[path.to_string()]);
    }
    let Some(body) = build_hunks(&fd.hunks, &map, Direction::Forward) else { return Ok(()) };
    let untracked = fd.status == "untracked" || fd.status == "added";
    let text = file_header(path, untracked.then_some(fd.new_mode), body);
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
    let Some(fd) = (0..d.deltas().len()).find_map(|i| diff::file_diff(&d, i, true, None).ok().filter(|f| f.path == path)) else {
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
    let text = file_header(path, None, body);
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
    let Some(fd) = (0..d.deltas().len()).find_map(|i| diff::file_diff(&d, i, true, None).ok().filter(|f| f.path == path)) else {
        return err("That file has no unstaged changes any more.");
    };
    let map = selected_map(sel);
    if selects_everything(&fd, &map) {
        return discard_files(repo_path, &[path.to_string()]);
    }
    let Some(body) = build_hunks(&fd.hunks, &map, Direction::Reverse) else { return Ok(()) };
    let text = file_header(path, None, body);
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
