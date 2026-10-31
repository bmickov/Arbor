//! Stashes: save, apply, pop, drop — all undoable.

use super::undo::{self, Action};
use super::{err, is_clean, open, signature, Error, Result};
use git2::{Oid, Repository, StashApplyOptions, StashFlags};
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StashEntry {
    pub index: usize,
    pub oid: String,
    pub message: String,
    pub branch: Option<String>,
    pub time: i64,
    pub has_untracked: bool,
}

/// "On main: message" → "message", as shown in the stash list.
fn pretty(message: &str) -> String {
    match message.split_once(": ") {
        Some((_, rest)) => rest.to_string(),
        None => message.to_string(),
    }
}

pub fn list(repo_path: &str) -> Result<Vec<StashEntry>> {
    let mut repo = open(repo_path)?;
    let mut raw: Vec<(usize, String, Oid)> = vec![];
    repo.stash_foreach(|i, msg, id| {
        raw.push((i, msg.to_string(), *id));
        true
    })?;
    let mut out = vec![];
    for (index, message, id) in raw {
        let c = repo.find_commit(id)?;
        // "On main: message" / "WIP on main: abc123 summary"
        let branch = message
            .strip_prefix("WIP on ")
            .or_else(|| message.strip_prefix("On "))
            .and_then(|r| r.split_once(':'))
            .map(|(b, _)| b.to_string());
        out.push(StashEntry {
            index,
            oid: id.to_string(),
            message: pretty(&message),
            branch,
            time: c.time().seconds(),
            has_untracked: c.parent_count() > 2,
        });
    }
    Ok(out)
}

pub fn top_oid(repo: &Repository) -> Result<Option<Oid>> {
    Ok(repo.find_reference("refs/stash").ok().and_then(|r| r.target()))
}

fn entry_at(repo: &mut Repository, index: usize) -> Result<(Oid, String)> {
    let mut found = None;
    repo.stash_foreach(|i, msg, id| {
        if i == index {
            found = Some((*id, msg.to_string()));
            false
        } else {
            true
        }
    })?;
    found.ok_or_else(|| Error::Msg("That stash no longer exists.".into()))
}

/// Put a stash commit back on top of the stash list.
pub fn restore_entry(repo: &Repository, id: Oid, message: &str) -> Result<()> {
    let sig = signature(repo)?;
    let mut reflog = repo.reflog("refs/stash")?;
    reflog.append(id, &sig, Some(message))?;
    reflog.write()?;
    repo.reference("refs/stash", id, true, message)?;
    // Creating the ref may log a second entry; keep exactly one.
    let mut reflog = repo.reflog("refs/stash")?;
    if reflog.len() >= 2 {
        let a = reflog.get(0).map(|e| e.id_new());
        let b = reflog.get(1).map(|e| e.id_new());
        if a == Some(id) && b == Some(id) {
            reflog.remove(0, false)?;
            reflog.write()?;
        }
    }
    Ok(())
}

pub fn save(repo_path: &str, message: Option<&str>, include_untracked: bool, keep_index: bool, quiet: bool) -> Result<()> {
    save_paths(repo_path, message, include_untracked, keep_index, quiet, &[])
}

/// Stash changes, only in `paths` when given (like `git stash push -- paths`).
pub fn save_paths(repo_path: &str, message: Option<&str>, include_untracked: bool, keep_index: bool, quiet: bool, paths: &[String]) -> Result<()> {
    let mut repo = open(repo_path)?;
    let sig = signature(&repo)?;
    let mut flags = StashFlags::DEFAULT;
    if include_untracked {
        flags |= StashFlags::INCLUDE_UNTRACKED;
    }
    if keep_index {
        flags |= StashFlags::KEEP_INDEX;
    }
    let msg = message.filter(|m| !m.trim().is_empty());
    // libgit2 stashes only the given paths but resets every file in the
    // working tree, so keep the other changes aside and put them back.
    let keep = if paths.is_empty() { vec![] } else { snapshot_others(&repo, paths)? };
    let saved = if paths.is_empty() {
        repo.stash_save2(&sig, msg, Some(flags))
    } else {
        let mut opts = git2::StashSaveOptions::new(sig.clone());
        opts.flags(Some(flags));
        for p in paths {
            opts.pathspec(p.as_str());
        }
        repo.stash_save_ext(Some(&mut opts))
    };
    let saved = match (saved, msg, paths.is_empty()) {
        // git2 can't pass a message along with paths, so give the stash
        // commit and its list entry the message afterwards, as git would.
        (Ok(id), Some(m), false) => {
            let branch = super::head_branch(&repo).unwrap_or_else(|| "(no branch)".into());
            let text = format!("On {branch}: {m}");
            let c = repo.find_commit(id)?;
            let parents: Vec<git2::Commit> = c.parents().collect();
            let prefs: Vec<&git2::Commit> = parents.iter().collect();
            let named = repo.commit(None, &c.author(), &c.committer(), &text, &c.tree()?, &prefs)?;
            let mut reflog = repo.reflog("refs/stash")?;
            reflog.remove(0, false)?;
            reflog.append(named, &sig, Some(&text))?;
            reflog.write()?;
            repo.reference("refs/stash", named, true, &text)?;
            // Setting the ref may log it again; keep one entry.
            let mut reflog = repo.reflog("refs/stash")?;
            if reflog.len() >= 2 && reflog.get(0).map(|e| e.id_new()) == Some(named) && reflog.get(1).map(|e| e.id_new()) == Some(named) {
                reflog.remove(0, false)?;
                reflog.write()?;
            }
            Ok(named)
        }
        (other, _, _) => other,
    };
    if !keep.is_empty() {
        restore_others(&repo, keep)?;
    }
    let id = saved.map_err(|e| {
        if e.code() == git2::ErrorCode::NotFound {
            Error::Msg("There are no local changes to stash.".into())
        } else {
            e.into()
        }
    })?;
    let label = match (msg, paths.len()) {
        (Some(m), _) => format!("Stash \"{m}\""),
        (None, 0) => "Stash changes".to_string(),
        (None, 1) => format!("Stash {}", paths[0]),
        (None, n) => format!("Stash {n} files"),
    };
    if !quiet {
        undo::record(&repo, label, Action::PopStash { expect: id.to_string() });
    }
    Ok(())
}

fn apply_opts() -> StashApplyOptions<'static> {
    let mut o = StashApplyOptions::new();
    o.reinstantiate_index();
    o
}

/// Apply a stash, also when other changes are staged (libgit2 refuses
/// then, git doesn't): those are unstaged for the moment and staged again
/// afterwards. Their files in the working tree are never touched.
fn apply_stash(repo: &mut Repository, index: usize) -> std::result::Result<(), git2::Error> {
    let staged = staged_entries(repo)?;
    if staged.is_empty() {
        return repo.stash_apply(index, Some(&mut apply_opts()));
    }
    let head_id = repo.head()?.peel_to_commit()?.id();
    {
        let head = repo.find_object(head_id, None)?;
        let paths: Vec<&str> = staged.iter().map(|(p, _)| p.as_str()).collect();
        repo.reset_default(Some(&head), paths.iter())?;
    }
    let result = repo.stash_apply(index, Some(&mut apply_opts()));
    let head_tree = repo.find_commit(head_id)?.tree()?;
    let mut idx = repo.index()?;
    for (path, entry) in staged {
        // Where the stash staged the same file, its version wins.
        let mine = idx.get_path(std::path::Path::new(&path), 0);
        let at_head = head_tree.get_path(std::path::Path::new(&path)).ok().map(|e| e.id());
        if result.is_ok() && mine.as_ref().map(|e| Some(e.id) != at_head).unwrap_or(false) {
            continue;
        }
        match entry {
            Some(e) => idx.add(&e)?,
            None => {
                let _ = idx.remove_path(std::path::Path::new(&path));
            }
        }
    }
    idx.write()?;
    result
}

/// Paths whose index entry differs from HEAD, with that entry (None: deleted in the index).
fn staged_entries(repo: &Repository) -> std::result::Result<Vec<(String, Option<git2::IndexEntry>)>, git2::Error> {
    let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let index = repo.index()?;
    let diff = repo.diff_tree_to_index(head.as_ref(), Some(&index), None)?;
    Ok(diff
        .deltas()
        .filter_map(|d| d.new_file().path().or(d.old_file().path()).map(|p| p.to_string_lossy().replace('\\', "/")))
        .map(|p| {
            let e = index.get_path(std::path::Path::new(&p), 0);
            (p, e)
        })
        .collect())
}

fn map_apply(e: git2::Error) -> Error {
    if e.code() == git2::ErrorCode::Conflict {
        Error::Msg("Applying this stash would overwrite your local changes. Commit or stash them first.".into())
    } else if e.code() == git2::ErrorCode::MergeConflict {
        Error::Msg("This stash conflicts with your current files. Conflicting files were left marked.".into())
    } else {
        e.into()
    }
}

pub fn apply(repo_path: &str, index: usize) -> Result<()> {
    let mut repo = open(repo_path)?;
    let clean = is_clean(&repo)?;
    let (_, msg) = entry_at(&mut repo, index)?;
    apply_stash(&mut repo, index).map_err(map_apply)?;
    if clean {
        undo::record(&repo, format!("Apply stash \"{msg}\""), Action::ResetWorktree);
    }
    Ok(())
}

pub fn pop(repo_path: &str, index: usize, quiet: bool) -> Result<()> {
    let mut repo = open(repo_path)?;
    let clean = is_clean(&repo)?;
    let (id, msg) = entry_at(&mut repo, index)?;
    // Apply, then drop only if nothing conflicted, like `git stash pop`:
    // a conflicted pop keeps the stash so the changes can't be lost.
    let applied = apply_stash(&mut repo, index);
    let conflicted = repo.index()?.has_conflicts();
    if conflicted {
        return err(
            "The stashed changes conflict with the files in your working copy. \
             The conflicts are marked in the affected files, and the stash is kept in the list so nothing is lost.",
        );
    }
    applied.map_err(map_apply)?;
    repo.stash_drop(index)?;
    if !quiet {
        undo::record(&repo, format!("Pop stash \"{}\"", pretty(&msg)), Action::RestoreStash { oid: id.to_string(), message: msg, reset_worktree: clean });
    }
    Ok(())
}

pub fn drop(repo_path: &str, index: usize) -> Result<()> {
    let mut repo = open(repo_path)?;
    let (id, msg) = entry_at(&mut repo, index)?;
    repo.stash_drop(index)?;
    undo::record(&repo, format!("Delete stash \"{}\"", pretty(&msg)), Action::RestoreStash { oid: id.to_string(), message: msg, reset_worktree: false });
    Ok(())
}

/// Delete several stashes, identified by commit id (indexes shift as entries
/// are dropped), as one undoable step. Undo puts them back in their original
/// relative order, on top of the list.
pub fn drop_many(repo_path: &str, oids: &[String]) -> Result<usize> {
    let mut repo = open(repo_path)?;
    let mut entries: Vec<(usize, Oid, String)> = vec![];
    repo.stash_foreach(|i, msg, id| {
        if oids.iter().any(|o| *o == id.to_string()) {
            entries.push((i, *id, msg.to_string()));
        }
        true
    })?;
    if entries.len() != oids.len() {
        return err("Some of those stashes no longer exist. Nothing was deleted.");
    }
    // Drop from the highest index down so the remaining indexes stay valid.
    entries.sort_by_key(|e| std::cmp::Reverse(e.0));
    for (i, _, _) in &entries {
        repo.stash_drop(*i)?;
    }
    // Undo runs a batch backwards; record newest-first so the oldest is
    // restored first and ends up lowest in the list.
    entries.reverse();
    let n = entries.len();
    let label = match entries.first() {
        Some((_, _, msg)) if n == 1 => format!("Delete stash \"{}\"", pretty(msg)),
        _ => format!("Delete {n} stashes"),
    };
    let mut actions: Vec<Action> = entries
        .into_iter()
        .map(|(_, id, message)| Action::RestoreStash { oid: id.to_string(), message, reset_worktree: false })
        .collect();
    if n == 1 {
        undo::record(&repo, label, actions.pop().unwrap());
    } else if n > 1 {
        undo::record(&repo, label, Action::Batch { actions });
    }
    Ok(n)
}

/// Create a branch from a stash and apply it there (like `git stash branch`).
pub fn branch(repo_path: &str, index: usize, name: &str) -> Result<()> {
    let mut repo = open(repo_path)?;
    if !super::tracked_clean(&repo)? {
        return err("Commit or stash your changes before creating a branch from a stash.");
    }
    let (id, _) = entry_at(&mut repo, index)?;
    let base = repo.find_commit(id)?.parent_id(0)?;
    drop_repo(repo);
    super::branches::create_branch(repo_path, name, Some(&base.to_string()), true)?;
    pop(repo_path, index, false)
}

fn drop_repo(_r: Repository) {}

/// A changed file outside a partial stash: its working-tree bytes (None if
/// deleted) and its index entry (None if not in the index).
struct Kept {
    path: String,
    bytes: Option<Vec<u8>>,
    entry: Option<git2::IndexEntry>,
}

fn snapshot_others(repo: &Repository, paths: &[String]) -> Result<Vec<Kept>> {
    let chosen = |p: &str| paths.iter().any(|s| p == s || p.starts_with(&format!("{}/", s.trim_end_matches('/'))));
    let mut o = git2::StatusOptions::new();
    o.include_untracked(true).recurse_untracked_dirs(true).include_ignored(false).exclude_submodules(true);
    let root = repo.workdir().ok_or_else(|| Error::Msg("This repository has no working tree.".into()))?.to_path_buf();
    let index = repo.index()?;
    let mut out = vec![];
    for e in repo.statuses(Some(&mut o))?.iter() {
        let Ok(path) = e.path() else { continue };
        if chosen(path) {
            continue;
        }
        let full = root.join(path);
        out.push(Kept {
            path: path.to_string(),
            bytes: std::fs::read(&full).ok(),
            entry: index.get_path(std::path::Path::new(path), 0),
        });
    }
    Ok(out)
}

fn restore_others(repo: &Repository, keep: Vec<Kept>) -> Result<()> {
    let root = repo.workdir().ok_or_else(|| Error::Msg("This repository has no working tree.".into()))?.to_path_buf();
    let mut index = repo.index()?;
    for k in keep {
        let full = root.join(&k.path);
        match &k.bytes {
            Some(b) => {
                if let Some(d) = full.parent() {
                    std::fs::create_dir_all(d)?;
                }
                std::fs::write(&full, b)?;
            }
            None => {
                let _ = std::fs::remove_file(&full);
            }
        }
        match k.entry {
            Some(e) => index.add(&e)?,
            None => {
                let _ = index.remove_path(std::path::Path::new(&k.path));
            }
        }
    }
    index.write()?;
    Ok(())
}
