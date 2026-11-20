//! Git operations for Arbor, built on libgit2 (via the `git2` crate).
//!
//! Every public function takes the repository path, opens the repository,
//! does its work and returns plain serializable data. Keeping the backend
//! stateless means the UI never holds a stale handle and every command is
//! safe to run on a worker thread.

pub mod bisect;
pub mod blame;
pub mod branches;
pub mod commit;
pub mod conflict;
pub mod diff;
pub mod files;
pub mod gitflow;
pub mod hooks;
pub mod identity;
pub mod insight;
pub mod lfs;
pub mod library;
pub mod log;
pub mod merge;
pub mod patch;
pub mod rebase;
pub mod remote;
pub mod repo;
pub mod review;
pub mod sign;
pub mod stack;
pub mod stash;
pub mod status;
pub mod tools;
pub mod tree;
pub mod undo;
pub mod worktree;

use git2::{Oid, Repository, Signature};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}", git_message(.0))]
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

/// Process-wide libgit2 settings. Call once, before any thread starts.
///
/// libgit2 only caches trees and commits up to 4 KB by default, so a folder
/// with a hundred files (a tree of a few KB) is unpacked again on every visit;
/// walking history re-reads the same big trees thousands of times. Caching
/// them makes history, file history, blame and search several times faster.
pub fn init() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let _ = git2::opts::set_cache_object_limit(git2::ObjectType::Tree, 4 << 20);
        let _ = git2::opts::set_cache_object_limit(git2::ObjectType::Commit, 64 << 10);
    });
}

/// No commit yet: HEAD names a branch that doesn't exist. (libgit2's own
/// `is_empty` only recognises this when the branch is called master.)
pub fn unborn(repo: &Repository) -> bool {
    matches!(repo.head(), Err(e) if e.code() == git2::ErrorCode::UnbornBranch || e.code() == git2::ErrorCode::NotFound)
}

/// A libgit2 error as people should read it: plain words for the common
/// cases, otherwise its message without the "; class=…; code=…" that its
/// Display adds, starting with a capital letter.
pub fn git_message(e: &git2::Error) -> String {
    use git2::ErrorCode as C;
    let m = e.message().trim();
    match e.code() {
        C::UnbornBranch => return "This repository has no commits yet. Make the first commit, then try again.".into(),
        C::Unmerged => return "Some files still have conflicts. Resolve them (or abort the merge) first.".into(),
        C::Locked => {
            return "Another Git program is working in this repository (a .git/…lock file exists). Try again in a moment; if no Git program is running, delete the lock file.".into()
        }
        _ => {}
    }
    // "revspec 'nope' not found" → "Couldn't find 'nope'."
    if let Some(rest) = m.strip_prefix("revspec '") {
        if let Some(what) = rest.strip_suffix("' not found") {
            return format!("Couldn't find '{what}'. It may have been deleted or renamed; refresh and try again.");
        }
    }
    if m.contains("cannot create a tree from a not fully merged index") {
        return "Some files still have conflicts. Resolve them (or abort the merge) first.".into();
    }
    let mut c = m.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => format!("Git error ({:?})", e.code()),
    }
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

/// Where Arbor keeps its settings (`~/.config/arbor`). `ARBOR_CONFIG_DIR`
/// overrides it, which the tests use to stay out of the real config.
pub fn config_dir() -> Option<std::path::PathBuf> {
    if let Some(d) = std::env::var_os("ARBOR_CONFIG_DIR") {
        return Some(d.into());
    }
    dirs::config_dir().map(|d| d.join("arbor"))
}

/// A file named the way git names it (relative, forward slashes) as a path
/// inside the working tree `root`. Refuses names that could reach outside it
/// or into the repository's own folder: absolute paths, `.` and `..`, `.git`,
/// and paths through a folder that's a symlink. Paths come from git itself,
/// but also from the undo journal on disk and from the UI, so whatever goes
/// wrong upstream, deleting or writing a file never lands anywhere else.
pub fn in_workdir(root: &std::path::Path, rel: &str) -> Result<std::path::PathBuf> {
    use std::path::Component;
    let bad = || Error::Msg(format!("'{rel}' isn't a file in this repository's working tree."));
    let comps: Vec<Component> = std::path::Path::new(rel).components().collect();
    if comps.is_empty() {
        return Err(bad());
    }
    let mut full = root.to_path_buf();
    for (i, c) in comps.iter().enumerate() {
        let Component::Normal(name) = c else { return Err(bad()) };
        if name.to_string_lossy().eq_ignore_ascii_case(".git") {
            return Err(bad());
        }
        full.push(name);
        if i + 1 < comps.len() && full.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(bad());
        }
    }
    Ok(full)
}
