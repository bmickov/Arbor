//! The undo journal.
//!
//! Every operation that changes the repository records how to reverse
//! itself. Entries are stored in `.git/arbor/undo.json`, so undo history
//! survives restarts. Undo only runs when the repository still looks the way
//! the operation left it; otherwise it refuses rather than guess.

use super::{err, hard_reset_to, oid, Error, Result};
use git2::{build::CheckoutBuilder, Oid, Repository, ResetType};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX_ENTRIES: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Action {
    /// Put the index back to a tree recorded before the operation.
    RestoreIndex { tree: String },
    /// Move HEAD's branch back to `oid` (or delete the branch when `oid` is
    /// None, which reverses the first commit on an unborn branch).
    ResetHead {
        oid: Option<String>,
        mode: String,
        /// HEAD must still be here for undo to proceed.
        expect: String,
        branch: Option<String>,
    },
    Checkout { previous: String, expect: String },
    RecreateBranch { name: String, oid: String, upstream: Option<String> },
    DeleteBranch { name: String, expect: String },
    RenameBranch { from: String, to: String },
    RecreateTag { name: String, target: String },
    DeleteTag { name: String },
    /// Write file contents back to the working tree.
    RestoreFiles { files: Vec<SavedFile> },
    /// Put a stash entry back on the stash list; optionally clear the working
    /// tree changes it produced (only recorded when the tree was clean before).
    RestoreStash { oid: String, message: String, reset_worktree: bool },
    /// Re-apply and drop the newest stash (reverses "stash changes").
    PopStash { expect: String },
    /// Discard everything in the working tree (reverses an apply onto a clean tree).
    ResetWorktree,
    /// Throw away a merge that stopped on conflicts (or its result) and return to `oid`.
    AbortMerge { oid: String },
    RemoveRemote { name: String },
    RecreateRemote { name: String, url: String },
    SetUpstream { branch: String, previous: Option<String> },
    /// Several actions recorded as one step (e.g. deleting several branches);
    /// undone in reverse order.
    Batch { actions: Vec<Action> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedFile {
    pub path: String,
    /// Blob holding the old content; None means the file did not exist.
    pub blob: Option<String>,
    pub mode: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub label: String,
    pub time: i64,
    pub action: Action,
}

fn journal_path(repo: &Repository) -> PathBuf {
    repo.path().join("arbor").join("undo.json")
}

pub fn load(repo: &Repository) -> Vec<Entry> {
    std::fs::read_to_string(journal_path(repo))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(repo: &Repository, entries: &[Entry]) -> Result<()> {
    let p = journal_path(repo);
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(&p, serde_json::to_string_pretty(entries)?)?;
    Ok(())
}

pub fn record(repo: &Repository, label: impl Into<String>, action: Action) {
    let mut entries = load(repo);
    entries.push(Entry { label: label.into(), time: chrono::Utc::now().timestamp(), action });
    if entries.len() > MAX_ENTRIES {
        let drop = entries.len() - MAX_ENTRIES;
        entries.drain(..drop);
    }
    // Failing to write the journal must never fail the operation itself.
    let _ = save(repo, &entries);
}

/// Snapshot the index as a tree so it can be restored later. None when the
/// index has conflicts (a conflicted index can't be written as a tree).
pub fn index_tree(repo: &Repository) -> Option<String> {
    let mut index = repo.index().ok()?;
    if index.has_conflicts() {
        return None;
    }
    index.write_tree().ok().map(|t| t.to_string())
}

/// Record `ResetHead` after an operation moved HEAD from `before` to the current HEAD.
pub fn record_head_move(repo: &Repository, label: impl Into<String>, before: Option<Oid>, mode: &str) {
    if let Some(after) = super::head_oid(repo) {
        if Some(after) == before {
            return;
        }
        record(
            repo,
            label,
            Action::ResetHead {
                oid: before.map(|o| o.to_string()),
                mode: mode.into(),
                expect: after.to_string(),
                branch: super::head_branch(repo),
            },
        );
    }
}

/// Save the current working-tree content of `paths` as blobs.
pub fn save_files(repo: &Repository, paths: &[String]) -> Result<Vec<SavedFile>> {
    let root = repo.workdir().ok_or_else(|| Error::Msg("bare repository".into()))?;
    let mut out = Vec::new();
    for p in paths {
        let full = root.join(p);
        if full.is_file() {
            let data = std::fs::read(&full)?;
            let blob = repo.blob(&data)?;
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(&full)?.permissions().mode()
            };
            #[cfg(not(unix))]
            let mode = 0o644;
            out.push(SavedFile { path: p.clone(), blob: Some(blob.to_string()), mode });
        } else {
            out.push(SavedFile { path: p.clone(), blob: None, mode: 0 });
        }
    }
    Ok(out)
}

pub fn list(repo_path: &str) -> Result<Vec<Entry>> {
    let repo = super::open(repo_path)?;
    let mut e = load(&repo);
    e.reverse();
    Ok(e)
}

/// Reverse the most recent operation. Returns its label.
pub fn undo(repo_path: &str) -> Result<String> {
    let repo = super::open(repo_path)?;
    let mut entries = load(&repo);
    let Some(entry) = entries.pop() else {
        return err("Nothing to undo.");
    };
    apply(&repo, &entry.action)?;
    save(&repo, &entries)?;
    Ok(entry.label)
}

fn check_head(repo: &Repository, expect: &str) -> Result<()> {
    let now = super::head_oid(repo).map(|o| o.to_string()).unwrap_or_default();
    if now != expect {
        return err("The repository changed since this operation, so it can't be undone safely.");
    }
    Ok(())
}

fn apply(repo: &Repository, action: &Action) -> Result<()> {
    match action {
        Action::RestoreIndex { tree } => {
            let tree = repo.find_tree(oid(tree)?)?;
            let mut index = repo.index()?;
            index.read_tree(&tree)?;
            index.write()?;
        }
        Action::ResetHead { oid: target, mode, expect, branch } => {
            check_head(repo, expect)?;
            match target {
                Some(t) => {
                    let obj = repo.find_object(oid(t)?, None)?;
                    let kind = match mode.as_str() {
                        "hard" => {
                            if !super::tracked_clean(repo)? {
                                return err("Commit or stash your changes before undoing this.");
                            }
                            ResetType::Hard
                        }
                        "mixed" => ResetType::Mixed,
                        _ => ResetType::Soft,
                    };
                    repo.reset(&obj, kind, None)?;
                }
                None => {
                    // Undo the first commit: delete the branch, keep the index.
                    let name = branch.clone().ok_or_else(|| Error::Msg("No branch to reset.".into()))?;
                    repo.find_reference(&format!("refs/heads/{name}"))?.delete()?;
                }
            }
        }
        Action::Checkout { previous, expect } => {
            check_head(repo, expect)?;
            super::branches::checkout_target(repo, previous)?;
        }
        Action::RecreateBranch { name, oid: o, upstream } => {
            let c = repo.find_commit(oid(o)?)?;
            let mut b = repo.branch(name, &c, false)?;
            if let Some(u) = upstream {
                let _ = b.set_upstream(Some(u));
            }
        }
        Action::DeleteBranch { name, expect } => {
            let mut b = repo.find_branch(name, git2::BranchType::Local)?;
            if b.get().target().map(|o| o.to_string()).as_deref() != Some(expect.as_str()) {
                return err(format!("Branch '{name}' has new commits, so it wasn't deleted."));
            }
            if b.is_head() {
                return err(format!("Switch away from '{name}' before undoing its creation."));
            }
            b.delete()?;
        }
        Action::RenameBranch { from, to } => {
            let mut b = repo.find_branch(to, git2::BranchType::Local)?;
            b.rename(from, false)?;
        }
        Action::RecreateTag { name, target } => {
            repo.reference(&format!("refs/tags/{name}"), oid(target)?, false, "arbor: undo delete tag")?;
        }
        Action::DeleteTag { name } => {
            repo.tag_delete(name)?;
        }
        Action::RestoreFiles { files } => {
            let root = repo.workdir().ok_or_else(|| Error::Msg("bare repository".into()))?;
            for f in files {
                let full = root.join(&f.path);
                match &f.blob {
                    Some(b) => {
                        let blob = repo.find_blob(oid(b)?)?;
                        if let Some(dir) = full.parent() {
                            std::fs::create_dir_all(dir)?;
                        }
                        std::fs::write(&full, blob.content())?;
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            if f.mode != 0 {
                                let _ = std::fs::set_permissions(&full, std::fs::Permissions::from_mode(f.mode));
                            }
                        }
                    }
                    None => {
                        if full.is_file() {
                            std::fs::remove_file(&full)?;
                        }
                    }
                }
            }
        }
        Action::RestoreStash { oid: o, message, reset_worktree } => {
            if *reset_worktree {
                reset_worktree_hard(repo)?;
            }
            super::stash::restore_entry(repo, oid(o)?, message)?;
        }
        Action::PopStash { expect } => {
            let top = super::stash::top_oid(repo)?;
            if top.map(|o| o.to_string()).as_deref() != Some(expect.as_str()) {
                return err("The stash list changed, so the stash wasn't restored.");
            }
            let mut repo = Repository::open(repo.path())?;
            let mut opts = git2::StashApplyOptions::new();
            opts.reinstantiate_index();
            repo.stash_pop(0, Some(&mut opts))?;
        }
        Action::ResetWorktree => reset_worktree_hard(repo)?,
        Action::AbortMerge { oid: o } => {
            check_head(repo, o)?;
            hard_reset_to(repo, oid(o)?)?;
            repo.cleanup_state()?;
        }
        Action::RemoveRemote { name } => repo.remote_delete(name)?,
        Action::RecreateRemote { name, url } => {
            repo.remote(name, url)?;
        }
        Action::SetUpstream { branch, previous } => {
            let mut b = repo.find_branch(branch, git2::BranchType::Local)?;
            b.set_upstream(previous.as_deref())?;
        }
        Action::Batch { actions } => {
            for a in actions.iter().rev() {
                apply(repo, a)?;
            }
        }
    }
    Ok(())
}

/// Reset tracked files to HEAD and remove untracked files.
pub fn reset_worktree_hard(repo: &Repository) -> Result<()> {
    if let Some(h) = super::head_oid(repo) {
        hard_reset_to(repo, h)?;
    }
    let mut co = CheckoutBuilder::new();
    co.force().remove_untracked(true);
    repo.checkout_head(Some(&mut co))?;
    Ok(())
}
