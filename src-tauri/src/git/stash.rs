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
    let id = repo.stash_save2(&sig, msg, Some(flags)).map_err(|e| {
        if e.code() == git2::ErrorCode::NotFound {
            Error::Msg("There are no local changes to stash.".into())
        } else {
            e.into()
        }
    })?;
    let label = match msg {
        Some(m) => format!("Stash \"{m}\""),
        None => "Stash changes".to_string(),
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
    repo.stash_apply(index, Some(&mut apply_opts())).map_err(map_apply)?;
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
    let applied = repo.stash_apply(index, Some(&mut apply_opts()));
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
