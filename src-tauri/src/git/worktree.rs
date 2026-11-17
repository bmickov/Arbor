//! Worktrees (more than one branch checked out at once, each in its own
//! folder) and submodules (repositories inside a repository).

use super::remote::{callbacks, map_auth, Creds, Progress};
use super::{err, head_branch, open, short, Error, Result};
use git2::{BranchType, FetchOptions, Repository, SubmoduleIgnore, SubmoduleStatus, SubmoduleUpdateOptions, WorktreeAddOptions, WorktreeLockStatus, WorktreePruneOptions};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    /// Git's name for it (None for the main worktree).
    pub name: Option<String>,
    pub path: String,
    pub branch: Option<String>,
    /// Short id of the checked-out commit.
    pub head: Option<String>,
    pub summary: String,
    pub is_main: bool,
    /// The repository open in this tab is this worktree.
    pub is_current: bool,
    /// Uncommitted changes (tracked files).
    pub changes: usize,
    pub locked: Option<String>,
    /// Its folder is gone; prune it to tidy up.
    pub missing: bool,
}

fn same(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

fn main_workdir(repo: &Repository) -> Option<PathBuf> {
    if repo.is_worktree() {
        // commondir is the main repository's .git folder.
        let common = repo.commondir();
        let main = Repository::open(common.parent()?).ok()?;
        main.workdir().map(Path::to_path_buf)
    } else {
        repo.workdir().map(Path::to_path_buf)
    }
}

fn describe(path: &Path, name: Option<String>, is_main: bool, here: &Path) -> WorktreeInfo {
    let wt = Repository::open(path).ok();
    let head = wt.as_ref().and_then(|r| r.head().ok()).and_then(|h| h.peel_to_commit().ok());
    let changes = wt
        .as_ref()
        .and_then(|r| {
            let mut o = git2::StatusOptions::new();
            o.include_untracked(false).exclude_submodules(true);
            r.statuses(Some(&mut o)).ok().map(|s| s.len())
        })
        .unwrap_or(0);
    WorktreeInfo {
        name,
        path: path.to_string_lossy().trim_end_matches(['/', '\\']).to_string(),
        branch: wt.as_ref().and_then(head_branch),
        head: head.as_ref().map(|c| short(c.id())),
        summary: head.as_ref().map(super::summary).unwrap_or_default(),
        is_main,
        is_current: same(path, here),
        changes,
        locked: None,
        missing: !path.exists(),
    }
}

pub fn list(repo_path: &str) -> Result<Vec<WorktreeInfo>> {
    let repo = open(repo_path)?;
    let here = repo.workdir().map(Path::to_path_buf).unwrap_or_default();
    let mut out = vec![];
    if let Some(main) = main_workdir(&repo) {
        out.push(describe(&main, None, true, &here));
    }
    for name in repo.worktrees()?.iter().flatten().flatten() {
        let Ok(wt) = repo.find_worktree(name) else { continue };
        let mut info = describe(wt.path(), Some(name.to_string()), false, &here);
        if let Ok(WorktreeLockStatus::Locked(reason)) = wt.is_locked() {
            info.locked = Some(reason.unwrap_or_default());
        }
        info.missing |= wt.validate().is_err();
        out.push(info);
    }
    Ok(out)
}

/// A worktree name git doesn't have yet, from the folder name.
fn free_name(repo: &Repository, path: &Path) -> String {
    let base: String = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "worktree".into())
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' })
        .collect();
    let taken: Vec<String> = repo.worktrees().map(|w| w.iter().flatten().flatten().map(str::to_string).collect()).unwrap_or_default();
    let mut name = base.clone();
    let mut n = 2;
    while taken.contains(&name) || repo.commondir().join("worktrees").join(&name).exists() {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

/// Check out `branch` in a new folder. With `create`, the branch is made
/// first from `start` (default HEAD).
pub fn add(repo_path: &str, path: &str, branch: &str, create: bool, start: Option<&str>) -> Result<String> {
    let repo = open(repo_path)?;
    let dest = PathBuf::from(path);
    if dest.exists() && std::fs::read_dir(&dest).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return err(format!("{path} already exists and isn't empty. Pick a new folder."));
    }
    let branch = branch.trim();
    if branch.is_empty() {
        return err("Pick a branch for the new worktree.");
    }
    let local = if create {
        if repo.find_branch(branch, BranchType::Local).is_ok() {
            return err(format!("A branch named {branch} already exists."));
        }
        let at = repo.revparse_single(start.unwrap_or("HEAD"))?.peel_to_commit()?;
        let mut b = repo.branch(branch, &at, false)?;
        // Starting from a remote branch tracks it, like git does.
        if let Some(s) = start {
            if repo.find_branch(s, BranchType::Remote).is_ok() {
                let _ = b.set_upstream(Some(s));
            }
        }
        b
    } else {
        repo.find_branch(branch, BranchType::Local)
            .map_err(|_| Error::Msg(format!("There's no local branch named {branch}.")))?
    };
    // A branch can only be checked out in one worktree.
    for w in list(repo_path)? {
        if w.branch.as_deref() == Some(branch) {
            return err(format!("{branch} is already checked out in {}.", w.path));
        }
    }
    let name = free_name(&repo, &dest);
    if dest.exists() {
        // libgit2 wants to create the folder itself.
        std::fs::remove_dir(&dest)?;
    }
    let reference = local.into_reference();
    let mut opts = WorktreeAddOptions::new();
    opts.reference(Some(&reference));
    let wt = repo.worktree(&name, &dest, Some(&opts))?;
    Ok(wt.path().to_string_lossy().trim_end_matches(['/', '\\']).to_string())
}

/// Delete a worktree's folder and git's record of it. Its branch stays.
pub fn remove(repo_path: &str, name: &str, force: bool) -> Result<()> {
    let repo = open(repo_path)?;
    let known = repo.worktrees()?.iter().flatten().flatten().any(|w| w == name);
    if !known {
        return err(format!("There's no worktree called '{name}' any more. Refresh to see the current list."));
    }
    let wt = repo.find_worktree(name)?;
    if wt.path().exists() {
        if let Ok(r) = Repository::open(wt.path()) {
            if same(wt.path(), repo.workdir().unwrap_or(Path::new(""))) {
                return err("That's the worktree open in this tab. Open another one to remove it.");
            }
            if !force && !super::is_clean(&r)? {
                return err(format!("DIRTY:{} has changes that aren't committed.", wt.path().display()));
            }
        }
    }
    if !force {
        if let Ok(WorktreeLockStatus::Locked(reason)) = wt.is_locked() {
            return err(format!("LOCKED:This worktree is locked{}.", reason.map(|r| format!(": {r}")).unwrap_or_default()));
        }
    }
    let mut opts = WorktreePruneOptions::new();
    opts.valid(true).locked(true).working_tree(true);
    wt.prune(Some(&mut opts))?;
    Ok(())
}

/// Forget worktrees whose folders were deleted.
pub fn prune(repo_path: &str) -> Result<usize> {
    let repo = open(repo_path)?;
    let mut n = 0;
    for name in repo.worktrees()?.iter().flatten().flatten() {
        if let Ok(wt) = repo.find_worktree(name) {
            if wt.validate().is_err() && wt.is_prunable(None).unwrap_or(false) {
                wt.prune(None)?;
                n += 1;
            }
        }
    }
    Ok(n)
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SubmoduleInfo {
    pub name: String,
    pub path: String,
    /// Absolute path of its folder.
    pub full_path: String,
    pub url: Option<String>,
    pub branch: Option<String>,
    /// The commit this repository records for it.
    pub recorded: Option<String>,
    /// The commit checked out in its folder.
    pub checked_out: Option<String>,
    /// "uninitialized" (not cloned yet), "moved" (a different commit is
    /// checked out than recorded), "dirty" (changed files inside), or "ok".
    pub state: String,
    pub dirty: bool,
}

pub fn submodules(repo_path: &str) -> Result<Vec<SubmoduleInfo>> {
    let repo = open(repo_path)?;
    let root = repo.workdir().map(Path::to_path_buf).unwrap_or_default();
    let mut out = vec![];
    for sm in repo.submodules()? {
        let name = sm.name().unwrap_or("").to_string();
        let st = repo.submodule_status(&name, SubmoduleIgnore::None).unwrap_or(SubmoduleStatus::empty());
        let recorded = sm.index_id().or(sm.head_id());
        let checked_out = sm.workdir_id();
        let dirty = st.intersects(SubmoduleStatus::WD_INDEX_MODIFIED | SubmoduleStatus::WD_WD_MODIFIED | SubmoduleStatus::WD_UNTRACKED);
        let state = if st.contains(SubmoduleStatus::WD_UNINITIALIZED) || checked_out.is_none() {
            "uninitialized"
        } else if recorded.is_some() && recorded != checked_out {
            "moved"
        } else if dirty {
            "dirty"
        } else {
            "ok"
        };
        out.push(SubmoduleInfo {
            path: sm.path().to_string_lossy().replace('\\', "/"),
            full_path: root.join(sm.path()).to_string_lossy().to_string(),
            url: sm.url().ok().flatten().map(str::to_string),
            branch: sm.branch().ok().flatten().map(str::to_string),
            recorded: recorded.map(short),
            checked_out: checked_out.map(short),
            state: state.into(),
            dirty,
            name,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn update_all(repo: &Repository, only: &[String], creds: &Option<Creds>, progress: &Progress) -> Result<usize> {
    let mut n = 0;
    for mut sm in repo.submodules()? {
        let path = sm.path().to_string_lossy().replace('\\', "/");
        if !only.is_empty() && !only.contains(&path) {
            continue;
        }
        if let Some(p) = progress {
            p(format!("Updating {path}"));
        }
        let mut fo = FetchOptions::new();
        fo.remote_callbacks(callbacks(repo.config()?, creds.clone(), progress.clone()));
        let mut opts = SubmoduleUpdateOptions::new();
        opts.fetch(fo);
        sm.update(true, Some(&mut opts)).map_err(map_auth)?;
        n += 1;
        // And the submodules inside it.
        if let Ok(inner) = sm.open() {
            n += update_all(&inner, &[], creds, progress)?;
        }
    }
    Ok(n)
}

/// Clone (if needed) and check out the recorded commit of the given
/// submodules (all when empty), recursively.
pub fn update_submodules(repo_path: &str, paths: &[String], creds: Option<Creds>, progress: Progress) -> Result<usize> {
    let repo = open(repo_path)?;
    for p in paths {
        if let Some(s) = submodules(repo_path)?.iter().find(|s| &s.path == p) {
            if s.dirty {
                return err(format!("{} has changes that aren't committed. Commit or discard them inside the submodule first.", s.path));
            }
        }
    }
    update_all(&repo, paths, &creds, &progress)
}
