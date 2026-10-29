//! Git operations for Arbor, built on libgit2 (via the `git2` crate).
//!
//! Every public function takes the repository path, opens the repository,
//! does its work and returns plain serializable data. Keeping the backend
//! stateless means the UI never holds a stale handle and every command is
//! safe to run on a worker thread.

pub mod blame;
pub mod branches;
pub mod commit;
pub mod diff;
pub mod hooks;
pub mod log;
pub mod merge;
pub mod patch;
pub mod rebase;
pub mod remote;
pub mod repo;
pub mod stash;
pub mod status;
pub mod undo;

use git2::{Oid, Repository, Signature};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Git(#[from] git2::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Msg(String),
    /// The remote asked for a username and password the app doesn't have.
    #[error("AUTH_REQUIRED:{0}")]
    AuthRequired(String),
}

impl Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Msg(msg.into()))
}

pub fn open(path: &str) -> Result<Repository> {
    Ok(Repository::open(path)?)
}

pub fn oid(s: &str) -> Result<Oid> {
    Oid::from_str(s).map_err(|_| Error::Msg(format!("'{s}' is not a valid commit id")))
}

/// The signature used for new commits, from git config (user.name/user.email).
pub fn signature(repo: &Repository) -> Result<Signature<'static>> {
    repo.signature().map_err(|_| {
        Error::Msg(
            "Set your name and email first: git config --global user.name \"Your Name\" \
             and git config --global user.email you@example.com"
                .into(),
        )
    })
}

/// True when the working tree and index match HEAD (untracked files count as changes).
pub fn is_clean(repo: &Repository) -> Result<bool> {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true).include_ignored(false);
    Ok(repo.statuses(Some(&mut opts))?.is_empty())
}

/// True when tracked files have no staged or unstaged changes. Untracked files are ignored.
pub fn tracked_clean(repo: &Repository) -> Result<bool> {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(false).include_ignored(false);
    Ok(repo.statuses(Some(&mut opts))?.is_empty())
}

pub fn require_tracked_clean(repo: &Repository, what: &str) -> Result<()> {
    if !tracked_clean(repo)? {
        return err(format!(
            "Commit or stash your changes before you {what}."
        ));
    }
    Ok(())
}

pub fn head_oid(repo: &Repository) -> Option<Oid> {
    repo.head().ok().and_then(|h| h.target())
}

/// Name of the branch HEAD points at ("main"), or None when detached.
pub fn head_branch(repo: &Repository) -> Option<String> {
    if repo.head_detached().unwrap_or(false) {
        return None;
    }
    match repo.head() {
        Ok(h) => h.shorthand().ok().map(|s| s.to_string()),
        // Unborn branch: read the symbolic target of HEAD.
        Err(_) => repo
            .find_reference("HEAD")
            .ok()
            .and_then(|r| r.symbolic_target().ok().flatten().map(|s| s.trim_start_matches("refs/heads/").to_string())),
    }
}

pub fn short(oid: Oid) -> String {
    oid.to_string()[..7].to_string()
}

pub fn first_line(msg: &str) -> String {
    msg.lines().next().unwrap_or("").to_string()
}

/// Hard-reset HEAD (and its branch) to `target`, updating index and working tree.
pub fn hard_reset_to(repo: &Repository, target: Oid) -> Result<()> {
    let obj = repo.find_object(target, None)?;
    repo.reset(&obj, git2::ResetType::Hard, None)?;
    Ok(())
}

/// Move HEAD (and its branch) to `new`, updating index and working tree
/// with a safe checkout that refuses to overwrite local changes.
pub fn advance_head(repo: &Repository, new: Oid, reflog: &str) -> Result<()> {
    let commit = repo.find_commit(new)?;
    let mut co = git2::build::CheckoutBuilder::new();
    co.safe();
    repo.checkout_tree(commit.as_object(), Some(&mut co))?;
    if repo.head_detached().unwrap_or(false) {
        repo.set_head_detached(new)?;
    } else {
        let head = repo.find_reference("HEAD")?;
        let target = head
            .symbolic_target()?
            .ok_or_else(|| Error::Msg("HEAD isn't on a branch.".into()))?
            .to_string();
        repo.reference(&target, new, true, reflog)?;
    }
    Ok(())
}

/// A commit's first line ("" when it can't be decoded).
pub fn summary(c: &git2::Commit) -> String {
    c.summary().ok().flatten().unwrap_or("").to_string()
}
