//! Working copy status: what's staged, what isn't, and what's conflicted.

use super::{head_branch, open, Result};
use git2::{Repository, RepositoryState, Status, StatusOptions};
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    pub old_path: Option<String>,
    /// added | modified | deleted | renamed | typechange | untracked | conflicted
    pub status: &'static str,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WorkingStatus {
    pub staged: Vec<FileEntry>,
    pub unstaged: Vec<FileEntry>,
    pub conflicted: Vec<FileEntry>,
    /// clean | merge | rebase | cherrypick | revert | other
    pub state: &'static str,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub head_summary: Option<String>,
    pub detached: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    /// Message git prepared for a merge in progress (MERGE_MSG).
    pub merge_message: Option<String>,
    /// A rebase Arbor paused on a conflict.
    pub paused_rebase: Option<super::rebase::PausedInfo>,
    /// A bisect in progress.
    pub bisect: Option<super::bisect::BisectState>,
}

fn state_name(s: RepositoryState) -> &'static str {
    match s {
        RepositoryState::Clean => "clean",
        RepositoryState::Merge => "merge",
        RepositoryState::Rebase | RepositoryState::RebaseInteractive | RepositoryState::RebaseMerge => "rebase",
        RepositoryState::CherryPick | RepositoryState::CherryPickSequence => "cherrypick",
        RepositoryState::Revert | RepositoryState::RevertSequence => "revert",
        _ => "other",
    }
}

pub fn status(repo_path: &str) -> Result<WorkingStatus> {
    let mut repo = open(repo_path)?;
    if super::lfs::refresh_if_needed(&repo) {
        // git rewrote the index; don't use libgit2's cached copy.
        repo = open(repo_path)?;
    }
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);
    let statuses = repo.statuses(Some(&mut opts))?;

    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let mut conflicted = Vec::new();

    for e in statuses.iter() {
        let s = e.status();
        let path = e.path().unwrap_or("").to_string();
        if s.contains(Status::CONFLICTED) {
            conflicted.push(FileEntry { path, old_path: None, status: "conflicted" });
            continue;
        }
        let index_kind = if s.contains(Status::INDEX_NEW) {
            Some("added")
        } else if s.contains(Status::INDEX_DELETED) {
            Some("deleted")
        } else if s.contains(Status::INDEX_RENAMED) {
            Some("renamed")
        } else if s.contains(Status::INDEX_TYPECHANGE) {
            Some("typechange")
        } else if s.contains(Status::INDEX_MODIFIED) {
            Some("modified")
        } else {
            None
        };
        if let Some(kind) = index_kind {
            let (p, old) = match e.head_to_index() {
                Some(d) => {
                    let new = d.new_file().path().map(|p| p.to_string_lossy().to_string()).unwrap_or(path.clone());
                    let old = d.old_file().path().map(|p| p.to_string_lossy().to_string());
                    (new.clone(), if kind == "renamed" { old.filter(|o| *o != new) } else { None })
                }
                None => (path.clone(), None),
            };
            staged.push(FileEntry { path: p, old_path: old, status: kind });
        }
        let wt_kind = if s.contains(Status::WT_NEW) {
            Some("untracked")
        } else if s.contains(Status::WT_DELETED) {
            Some("deleted")
        } else if s.contains(Status::WT_RENAMED) {
            Some("renamed")
        } else if s.contains(Status::WT_TYPECHANGE) {
            Some("typechange")
        } else if s.contains(Status::WT_MODIFIED) {
            Some("modified")
        } else {
            None
        };
        if let Some(kind) = wt_kind {
            let (p, old) = match e.index_to_workdir() {
                Some(d) if kind == "renamed" => {
                    let new = d.new_file().path().map(|p| p.to_string_lossy().to_string()).unwrap_or(path.clone());
                    let old = d.old_file().path().map(|p| p.to_string_lossy().to_string());
                    (new, old)
                }
                _ => (path.clone(), None),
            };
            unstaged.push(FileEntry { path: p, old_path: old, status: kind });
        }
    }
    staged.sort_by(|a, b| a.path.cmp(&b.path));
    unstaged.sort_by(|a, b| a.path.cmp(&b.path));
    conflicted.sort_by(|a, b| a.path.cmp(&b.path));

    let (upstream, ahead, behind) = upstream_info(&repo);
    let head = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let merge_message = std::fs::read_to_string(repo.path().join("MERGE_MSG"))
        .ok()
        .map(|m| m.lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join("\n").trim().to_string());

    Ok(WorkingStatus {
        staged,
        unstaged,
        conflicted,
        state: if super::rebase::paused(&repo).is_some() { "rebase" } else { state_name(repo.state()) },
        branch: head_branch(&repo),
        head: head.as_ref().map(|c| c.id().to_string()),
        head_summary: head.as_ref().map(super::summary),
        detached: repo.head_detached().unwrap_or(false),
        upstream,
        ahead,
        behind,
        merge_message,
        paused_rebase: super::rebase::paused_info(&repo),
        bisect: super::bisect::state_of(&repo).ok().flatten(),
    })
}

fn upstream_info(repo: &Repository) -> (Option<String>, usize, usize) {
    let Some(name) = head_branch(repo) else { return (None, 0, 0) };
    let Ok(b) = repo.find_branch(&name, git2::BranchType::Local) else { return (None, 0, 0) };
    let Ok(up) = b.upstream() else { return (None, 0, 0) };
    let up_name = up.name().ok().flatten().map(|s| s.to_string());
    match (b.get().target(), up.get().target()) {
        (Some(l), Some(u)) => {
            let (a, bh) = repo.graph_ahead_behind(l, u).unwrap_or((0, 0));
            (up_name, a, bh)
        }
        _ => (up_name, 0, 0),
    }
}
