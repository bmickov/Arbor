//! Git LFS. libgit2 doesn't run git's filters, so for LFS files Arbor hands
//! the work to `git` and `git lfs`: staging goes through `git add` (which
//! turns big files into LFS pointers), and after anything that rewrites the
//! working tree `git lfs checkout` puts real file contents back in place of
//! pointers. Tracking patterns and file locks use `git lfs` too.

use super::{err, open, Error, Result};
use crate::platform::quiet_command;
use git2::Repository;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Stdio;

fn git(repo: &Repository) -> std::process::Command {
    let mut c = quiet_command("git");
    c.current_dir(repo.workdir().unwrap_or(repo.path()));
    // Never wait for a password on a terminal nobody can see.
    c.env("GIT_TERMINAL_PROMPT", "0").stdin(Stdio::null());
    c
}

fn run(repo: &Repository, args: &[&str]) -> Result<String> {
    let out = git(repo).args(args).output().map_err(|e| Error::Msg(format!("Couldn't run git: {e}. Git LFS needs git and git-lfs installed.")))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return err(if why.is_empty() { format!("git {} failed.", args.join(" ")) } else { why });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The repository uses LFS: some .gitattributes line says filter=lfs.
pub fn used(repo: &Repository) -> bool {
    let Some(root) = repo.workdir() else { return false };
    let check = |p: &Path| std::fs::read_to_string(p).map(|s| s.contains("filter=lfs")).unwrap_or(false);
    if check(&root.join(".gitattributes")) {
        return true;
    }
    // Nested .gitattributes files, a couple of levels down.
    let mut dirs = vec![(root.to_path_buf(), 0)];
    while let Some((d, depth)) = dirs.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name();
            if name == ".git" || name == "node_modules" || name == "target" {
                continue;
            }
            if p.is_dir() && depth < 2 {
                dirs.push((p, depth + 1));
            } else if name == ".gitattributes" && check(&p) {
                return true;
            }
        }
    }
    false
}

pub fn installed() -> bool {
    static INSTALLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *INSTALLED.get_or_init(|| quiet_command("git").args(["lfs", "version"]).stdin(Stdio::null()).output().map(|o| o.status.success()).unwrap_or(false))
}

/// libgit2 compares LFS files byte for byte with their pointers, so when
/// git's cached file times are stale a downloaded LFS file looks modified.
/// `git update-index --refresh` re-checks those through the LFS filter.
pub fn refresh_if_needed(repo: &Repository) -> bool {
    if !used(repo) || !installed() {
        return false;
    }
    let mut o = git2::StatusOptions::new();
    o.include_untracked(false).exclude_submodules(true);
    let Ok(st) = repo.statuses(Some(&mut o)) else { return false };
    let suspect = st.iter().any(|e| e.status().is_wt_modified() && e.path().map(|p| is_lfs(repo, p)).unwrap_or(false));
    if suspect {
        let _ = git(repo).args(["update-index", "-q", "--refresh"]).output();
    }
    suspect
}

/// The path is stored with LFS.
pub fn is_lfs(repo: &Repository, path: &str) -> bool {
    repo.get_attr(Path::new(path), "filter", git2::AttrCheckFlags::default()).ok().flatten() == Some("lfs")
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LfsInfo {
    pub used: bool,
    pub installed: bool,
    /// Tracked patterns from .gitattributes ("*.psd", "assets/**").
    pub patterns: Vec<String>,
    /// LFS files in the working tree that are still pointers (content not downloaded).
    pub missing: usize,
    pub files: usize,
}

pub fn info(repo_path: &str) -> Result<LfsInfo> {
    let repo = open(repo_path)?;
    let used = used(&repo);
    let installed = installed();
    let mut patterns = vec![];
    let (mut files, mut missing) = (0, 0);
    if used && installed {
        // "Listing tracked patterns\n    *.psd (.gitattributes)\n"
        for l in run(&repo, &["lfs", "track"]).unwrap_or_default().lines() {
            let l = l.trim();
            if let Some(p) = l.strip_suffix(')').and_then(|l| l.rsplit_once(" (")).map(|(p, _)| p.trim()) {
                if !p.is_empty() && !l.starts_with("Listing") {
                    patterns.push(p.to_string());
                }
            }
        }
        // "abc123 * file" (downloaded) or "abc123 - file" (pointer only).
        for l in run(&repo, &["lfs", "ls-files"]).unwrap_or_default().lines() {
            files += 1;
            if l.split_whitespace().nth(1) == Some("-") {
                missing += 1;
            }
        }
    }
    Ok(LfsInfo { used, installed, patterns, missing, files })
}

fn require(repo: &Repository) -> Result<()> {
    if !installed() {
        return err("This repository stores files with Git LFS. Install Git LFS (git-lfs) so Arbor can handle them.");
    }
    let _ = repo;
    Ok(())
}

/// Stage `paths` through `git add`, so LFS files become pointers.
pub fn add(repo: &Repository, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    require(repo)?;
    let mut args = vec!["add", "--"];
    args.extend(paths.iter().map(String::as_str));
    run(repo, &args)?;
    Ok(())
}

/// Of `paths`, those stored with LFS (when the repository uses LFS at all).
pub fn lfs_paths(repo: &Repository, paths: &[String]) -> Vec<String> {
    if !used(repo) {
        return vec![];
    }
    paths.iter().filter(|p| is_lfs(repo, p)).cloned().collect()
}

/// Put real contents in place of pointer files (after a checkout, reset,
/// merge, stash or discard). Quiet no-op when LFS isn't used or installed.
pub fn checkout(repo_path: &str) -> Result<()> {
    let repo = open(repo_path)?;
    if !used(&repo) || !installed() {
        return Ok(());
    }
    run(&repo, &["lfs", "checkout"])?;
    Ok(())
}

/// Download the LFS files for what's checked out and put them in place.
pub fn pull(repo_path: &str) -> Result<()> {
    let repo = open(repo_path)?;
    if !used(&repo) {
        return Ok(());
    }
    require(&repo)?;
    run(&repo, &["lfs", "pull"])?;
    Ok(())
}

pub fn track(repo_path: &str, pattern: &str, untrack: bool) -> Result<()> {
    let repo = open(repo_path)?;
    require(&repo)?;
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return err("Enter a file pattern, like *.psd.");
    }
    if !untrack {
        // Make sure the LFS filters are set up for this user.
        let _ = run(&repo, &["lfs", "install", "--local"]);
    }
    run(&repo, &["lfs", if untrack { "untrack" } else { "track" }, pattern])?;
    Ok(())
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LockOwner {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Lock {
    pub id: String,
    pub path: String,
    pub owner: Option<LockOwner>,
    #[serde(alias = "locked_at")]
    pub locked_at: Option<String>,
}

/// File locks on the server (needs a remote that supports LFS locking).
pub fn locks(repo_path: &str) -> Result<Vec<Lock>> {
    let repo = open(repo_path)?;
    require(&repo)?;
    let out = run(&repo, &["lfs", "locks", "--json"])?;
    Ok(serde_json::from_str(out.trim()).unwrap_or_default())
}

pub fn lock(repo_path: &str, path: &str, unlock: bool, force: bool) -> Result<()> {
    let repo = open(repo_path)?;
    require(&repo)?;
    let mut args = vec!["lfs", if unlock { "unlock" } else { "lock" }];
    if unlock && force {
        args.push("--force");
    }
    args.push(path);
    run(&repo, &args)?;
    Ok(())
}
