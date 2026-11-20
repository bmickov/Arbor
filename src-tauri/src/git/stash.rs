//! Stashes: save, apply, pop, drop — all undoable.

use super::undo::{self, Action};
use super::{err, is_clean, open, short, signature, Error, Result};
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
    let msg = message.filter(|m| !m.trim().is_empty());
    let id = if paths.is_empty() {
        let mut flags = StashFlags::DEFAULT;
        if include_untracked {
            flags |= StashFlags::INCLUDE_UNTRACKED;
        }
        if keep_index {
            flags |= StashFlags::KEEP_INDEX;
        }
        repo.stash_save2(&sig, msg, Some(flags)).map_err(|e| {
            if e.code() == git2::ErrorCode::NotFound {
                Error::Msg("There are no local changes to stash.".into())
            } else {
                e.into()
            }
        })?
    } else {
        save_partial(&repo, repo_path, &sig, msg, include_untracked, keep_index, paths)?
    };
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

/// Stash only the changes in `paths` (files or folders), as
/// `git stash push -- paths` does, leaving every other change alone.
///
/// libgit2 can do this, but it puts every untracked file in the stash, takes
/// seconds with a few thousand paths, and resets the whole working tree, so
/// the stash commits are built here (as `snapshot` does) and then just the
/// chosen files are put back as they are at HEAD.
fn save_partial(repo: &Repository, repo_path: &str, sig: &git2::Signature, msg: Option<&str>, include_untracked: bool, keep_index: bool, paths: &[String]) -> Result<Oid> {
    if super::unborn(repo) {
        return err("Stashing needs a first commit to compare your changes with. Make the first commit, then try again.");
    }
    let mut index = repo.index()?;
    if index.has_conflicts() {
        return err("Some files still have conflicts. Resolve them (or abort the merge) first.");
    }
    let root = repo.workdir().ok_or_else(|| Error::Msg("This repository has no working tree.".into()))?.to_path_buf();
    let head = repo.head()?.peel_to_commit()?;
    let head_tree = head.tree()?;
    let branch = super::head_branch(repo).unwrap_or_else(|| "(no branch)".into());
    let head_line = format!("{} {}", short(head.id()), super::summary(&head));

    // What changed among the chosen paths. A chosen file that was staged as
    // a rename brings the file it was renamed from along.
    let chosen = chooser(paths);
    let mut so = git2::StatusOptions::new();
    so.include_untracked(true).recurse_untracked_dirs(true).include_ignored(false).exclude_submodules(true).renames_head_to_index(true);
    let mut staged: Vec<String> = vec![];
    let mut worktree: Vec<String> = vec![];
    let mut untracked: Vec<String> = vec![];
    for e in repo.statuses(Some(&mut so))?.iter() {
        let Ok(path) = e.path() else { continue };
        // A staged rename is one entry; the list shows it by its new name.
        let renamed = e.head_to_index().and_then(|d| {
            let new = d.new_file().path()?.to_string_lossy().into_owned();
            let old = d.old_file().path()?.to_string_lossy().into_owned();
            (new != old).then_some((new, old))
        });
        if !chosen(path) && !renamed.as_ref().is_some_and(|(n, o)| chosen(n) || chosen(o)) {
            continue;
        }
        let st = e.status();
        if st.contains(git2::Status::WT_NEW) {
            if include_untracked {
                untracked.push(path.to_string());
            }
            continue;
        }
        if st.intersects(git2::Status::INDEX_NEW | git2::Status::INDEX_MODIFIED | git2::Status::INDEX_DELETED | git2::Status::INDEX_RENAMED | git2::Status::INDEX_TYPECHANGE) {
            match &renamed {
                Some((n, o)) => {
                    staged.push(n.clone());
                    staged.push(o.clone());
                }
                None => staged.push(path.to_string()),
            }
        }
        if st.intersects(git2::Status::WT_MODIFIED | git2::Status::WT_DELETED | git2::Status::WT_TYPECHANGE | git2::Status::WT_RENAMED) {
            worktree.push(renamed.as_ref().map(|(n, _)| n.clone()).unwrap_or_else(|| path.to_string()));
        }
    }
    if staged.is_empty() && worktree.is_empty() && untracked.is_empty() {
        return err("There are no local changes to stash.");
    }

    // Index commit: HEAD with the chosen files as they're staged.
    let mut i_idx = git2::Index::new()?;
    i_idx.read_tree(&head_tree)?;
    for p in &staged {
        match index.get_path(std::path::Path::new(p), 0) {
            Some(e) => i_idx.add(&e)?,
            None => {
                let _ = i_idx.remove_path(std::path::Path::new(p));
            }
        }
    }
    let i_tree = i_idx.write_tree_to(repo)?;

    // The other two go through a second handle with a throwaway index, so
    // files are read with the repository's filters (line endings, LFS).
    let dir = tempfile::tempdir()?;
    let scratch = |file: &str, start: Option<Oid>, add: &dyn Fn(&mut git2::Index) -> std::result::Result<(), git2::Error>| -> Result<Oid> {
        let other = open(repo_path)?;
        let mut idx = git2::Index::open(&dir.path().join(file))?;
        if let Some(t) = start {
            idx.read_tree(&other.find_tree(t)?)?;
        }
        other.set_index(&mut idx)?;
        let mut idx = other.index()?;
        add(&mut idx)?;
        Ok(idx.write_tree_to(&other)?)
    };
    // Working-tree commit: the index commit plus the chosen files as they are on disk.
    let w_tree = scratch("work", Some(i_tree), &|idx| {
        for p in &worktree {
            if root.join(p).symlink_metadata().is_ok() {
                idx.add_path(std::path::Path::new(p))?;
            } else {
                let _ = idx.remove_path(std::path::Path::new(p));
            }
        }
        Ok(())
    })?;
    let u_tree = if untracked.is_empty() {
        None
    } else {
        Some(scratch("untracked", None, &|idx| {
            for p in &untracked {
                idx.add_path(std::path::Path::new(p))?;
            }
            Ok(())
        })?)
    };

    let i = repo.find_commit(repo.commit(None, sig, sig, &format!("index on {branch}: {head_line}\n"), &repo.find_tree(i_tree)?, &[&head])?)?;
    let u = match u_tree {
        Some(t) => Some(repo.find_commit(repo.commit(None, sig, sig, &format!("untracked files on {branch}: {head_line}\n"), &repo.find_tree(t)?, &[])?)?),
        None => None,
    };
    let mut parents = vec![&head, &i];
    if let Some(u) = &u {
        parents.push(u);
    }
    let message = match msg {
        Some(m) => format!("On {branch}: {m}"),
        None => format!("WIP on {branch}: {head_line}"),
    };
    let w = repo.commit(None, sig, sig, &format!("{message}\n"), &repo.find_tree(w_tree)?, &parents)?;
    restore_entry(repo, w, &message)?;

    // Now take the chosen changes out: the index back to HEAD for those
    // files (unless keeping the index), the files back to match it.
    if !keep_index {
        for p in &staged {
            match head_tree.get_path(std::path::Path::new(p)) {
                Ok(t) => index.add(&git2::IndexEntry {
                    ctime: git2::IndexTime::new(0, 0),
                    mtime: git2::IndexTime::new(0, 0),
                    dev: 0,
                    ino: 0,
                    mode: t.filemode() as u32,
                    uid: 0,
                    gid: 0,
                    file_size: 0,
                    id: t.id(),
                    flags: 0,
                    flags_extended: 0,
                    path: p.as_bytes().to_vec(),
                })?,
                Err(_) => {
                    let _ = index.remove_path(std::path::Path::new(p));
                }
            }
        }
        index.write()?;
    }
    let mut back: Vec<&String> = staged.iter().chain(worktree.iter()).collect();
    back.sort();
    back.dedup();
    let mut co = git2::build::CheckoutBuilder::new();
    co.force().disable_pathspec_match(true);
    let mut any = false;
    for p in &back {
        if index.get_path(std::path::Path::new(p.as_str()), 0).is_some() {
            co.path(p.as_str());
            any = true;
        } else {
            remove_file(&root, p);
        }
    }
    if any {
        repo.checkout_index(Some(&mut index), Some(&mut co))?;
    }
    for p in &untracked {
        remove_file(&root, p);
    }
    Ok(w)
}

/// Delete a file from the working tree, and the folders it leaves empty.
fn remove_file(root: &std::path::Path, path: &str) {
    let Ok(full) = super::in_workdir(root, path) else { return };
    let _ = std::fs::remove_file(&full);
    let mut dir = full.parent();
    while let Some(d) = dir {
        if d == root || !d.starts_with(root) || std::fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
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

/// Whether a path is one of `paths` or inside one of them. A set lookup
/// per folder of the path, so thousands of paths against thousands of
/// changes stays quick.
fn chooser(paths: &[String]) -> impl Fn(&str) -> bool {
    let set: std::collections::HashSet<String> = paths.iter().map(|p| p.trim_end_matches('/').to_string()).collect();
    move |p: &str| {
        if set.contains(p) {
            return true;
        }
        let mut rest = p;
        while let Some(i) = rest.rfind('/') {
            rest = &rest[..i];
            if set.contains(rest) {
                return true;
            }
        }
        false
    }
}




/// Save a snapshot: every current change (staged, unstaged and untracked)
/// stored as a stash, while the working tree and index stay exactly as they
/// are. Nothing is written to them: the stash commits are built the way
/// `git stash` builds them (index, untracked files, working tree) from
/// scratch indexes, so files are never rewritten (which matters for Git LFS
/// files, which libgit2 can't smudge back).
pub fn snapshot(repo_path: &str, name: Option<&str>) -> Result<String> {
    let repo = open(repo_path)?;
    if super::unborn(&repo) {
        return err("Snapshots need a first commit to compare your changes with. Make the first commit, then try again.");
    }
    if repo.index()?.has_conflicts() {
        return err("Some files still have conflicts. Resolve them (or abort the merge) first.");
    }
    let sig = signature(&repo)?;
    let head = repo.head()?.peel_to_commit()?;
    let branch = super::head_branch(&repo).unwrap_or_else(|| "(no branch)".into());

    // Index: what's staged now.
    let index_tree = repo.index()?.write_tree()?;

    // Untracked (not ignored) files, from the real index's point of view.
    let mut so = git2::StatusOptions::new();
    so.include_untracked(true).recurse_untracked_dirs(true).include_ignored(false);
    let statuses = repo.statuses(Some(&mut so))?;
    let path_of = |s: &git2::StatusEntry| s.path().ok().map(str::to_string);
    let untracked: Vec<String> = statuses.iter().filter(|s| s.status().contains(git2::Status::WT_NEW)).filter_map(|s| path_of(&s)).collect();
    // Tracked files changed in the working tree: only these are read again,
    // so unchanged files (Git LFS ones especially) aren't re-hashed.
    let changed = git2::Status::WT_MODIFIED | git2::Status::WT_DELETED | git2::Status::WT_TYPECHANGE | git2::Status::WT_RENAMED;
    let changed: std::collections::HashSet<String> = statuses.iter().filter(|s| s.status().intersects(changed)).filter_map(|s| path_of(&s)).collect();

    // Build the other two trees through a second handle on the repository
    // with a throwaway index, so the real index is never touched.
    let dir = tempfile::tempdir()?;
    let scratch = |file: &str, start: Option<git2::Oid>, add: &dyn Fn(&mut git2::Index) -> std::result::Result<(), git2::Error>| -> Result<git2::Oid> {
        let other = open(repo_path)?;
        let mut idx = git2::Index::open(&dir.path().join(file))?;
        if let Some(t) = start {
            idx.read_tree(&other.find_tree(t)?)?;
        }
        other.set_index(&mut idx)?;
        let mut idx = other.index()?;
        add(&mut idx)?;
        Ok(idx.write_tree_to(&other)?)
    };
    // Working tree: the index plus every tracked file as it is on disk.
    let work_tree = scratch("work", Some(index_tree), &|idx| {
        let mut only_changed = |p: &std::path::Path, _: &[u8]| if changed.contains(&p.to_string_lossy().replace('\\', "/")) { 0 } else { 1 };
        idx.update_all(["*"].iter(), Some(&mut only_changed))
    })?;
    let untracked_tree = if untracked.is_empty() {
        None
    } else {
        Some(scratch("untracked", None, &|idx| {
            for p in &untracked {
                idx.add_path(std::path::Path::new(p))?;
            }
            Ok(())
        })?)
    };

    if index_tree == head.tree_id() && work_tree == head.tree_id() && untracked_tree.is_none() {
        return err("There are no changes to snapshot.");
    }

    let when = chrono::Local::now().format("%Y-%m-%d %H:%M");
    let name = name.map(str::trim).filter(|n| !n.is_empty());
    let label = match name {
        Some(n) => format!("Snapshot: {n}"),
        None => format!("Snapshot {when}"),
    };
    let head_line = format!("{} {}", short(head.id()), super::summary(&head));
    let i = repo.commit(None, &sig, &sig, &format!("index on {branch}: {head_line}\n"), &repo.find_tree(index_tree)?, &[&head])?;
    let i = repo.find_commit(i)?;
    let u = match untracked_tree {
        Some(t) => Some(repo.find_commit(repo.commit(None, &sig, &sig, &format!("untracked files on {branch}: {head_line}\n"), &repo.find_tree(t)?, &[])?)?),
        None => None,
    };
    let mut parents = vec![&head, &i];
    if let Some(u) = &u {
        parents.push(u);
    }
    let message = format!("On {branch}: {label}");
    let w = repo.commit(None, &sig, &sig, &format!("{message}\n"), &repo.find_tree(work_tree)?, &parents)?;

    // refs/stash and its reflog, which is the stash list. Setting the ref
    // only logs it when a reflog already exists, so add the entry if needed.
    repo.reference("refs/stash", w, true, &message)?;
    let mut reflog = repo.reflog("refs/stash")?;
    if reflog.get(0).map(|e| e.id_new()) != Some(w) {
        reflog.append(w, &sig, Some(&message))?;
        reflog.write()?;
    }
    Ok(label)
}

/// Go back to a stash (typically a snapshot): save what's there now as a new
/// snapshot first, so nothing is lost, then clear the working copy and
/// restore the stash's changes, staged ones staged again. Returns the label
/// of the snapshot taken of the current state, if there was anything to keep.
pub fn restore_snapshot(repo_path: &str, stash_oid: &str) -> Result<Option<String>> {
    let target = super::oid(stash_oid)?;
    let label = {
        let repo = open(repo_path)?;
        let summary = super::summary(&repo.find_commit(target)?);
        // "On main: Snapshot: first try" → "first try"
        let rest = summary.split_once(": ").map(|(_, n)| n).unwrap_or(&summary);
        let name = rest.strip_prefix("Snapshot: ").unwrap_or(rest).trim().to_string();
        match snapshot(repo_path, Some(&format!("before going back to “{name}”"))) {
            Ok(l) => Some(l),
            Err(Error::Msg(m)) if m.starts_with("There are no changes") => None,
            Err(e) => return Err(e),
        }
    };
    let mut repo = open(repo_path)?;
    let mut index = None;
    repo.stash_foreach(|i, _, id| {
        if *id == target {
            index = Some(i);
            false
        } else {
            true
        }
    })?;
    let index = index.ok_or_else(|| Error::Msg("That stash no longer exists.".into()))?;
    undo::reset_worktree_hard(&repo)?;
    apply_stash(&mut repo, index).map_err(map_apply)?;
    Ok(label)
}
