//! Stacked branches: a branch can be marked as built on another ("parent")
//! branch. When the parent gets new commits (or is rebased), the branch is
//! "restacked": its own commits are replayed on the parent's new tip.
//!
//! Stored in the repository's config, next to the branch's other settings:
//! branch.<name>.arborParent and branch.<name>.arborBase (the parent commit
//! the branch's own commits start from).

use super::rebase::{start_with, RebaseOutcome, Step};
use super::branches::checkout_branch;
use super::{err, head_branch, open, require_tracked_clean, short, Result};
use git2::{BranchType, Oid, Repository, Sort};
use serde::Serialize;

fn key(branch: &str, what: &str) -> String {
    format!("branch.{branch}.{what}")
}

fn tip(repo: &Repository, branch: &str) -> Result<Oid> {
    Ok(repo.find_branch(branch, BranchType::Local)?.get().peel_to_commit()?.id())
}

pub fn parent_of(repo: &Repository, branch: &str) -> Option<String> {
    let p = repo.config().ok()?.get_string(&key(branch, "arborparent")).ok()?;
    repo.find_branch(&p, BranchType::Local).ok().map(|_| p)
}

pub(crate) fn record_base(repo: &Repository, branch: &str, base: &str) -> Result<()> {
    let mut cfg = repo.config()?.open_level(git2::ConfigLevel::Local)?;
    cfg.set_str(&key(branch, "arborbase"), base)?;
    Ok(())
}

/// Mark `branch` as stacked on `parent` (None removes it from its stack).
pub fn set_parent(repo_path: &str, branch: &str, parent: Option<&str>) -> Result<()> {
    let repo = open(repo_path)?;
    let mut cfg = repo.config()?.open_level(git2::ConfigLevel::Local)?;
    match parent {
        Some(p) => {
            if p == branch {
                return err("A branch can't be stacked on itself.");
            }
            // No loops: the parent mustn't (eventually) sit on this branch.
            let mut up = Some(p.to_string());
            while let Some(u) = up {
                if u == branch {
                    return err(format!("{p} is already stacked on {branch}."));
                }
                up = parent_of(&repo, &u);
            }
            let base = repo.merge_base(tip(&repo, branch)?, tip(&repo, p)?)?;
            cfg.set_str(&key(branch, "arborparent"), p)?;
            cfg.set_str(&key(branch, "arborbase"), &base.to_string())?;
        }
        None => {
            let _ = cfg.remove(&key(branch, "arborparent"));
            let _ = cfg.remove(&key(branch, "arborbase"));
        }
    }
    Ok(())
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Stacked {
    pub branch: String,
    pub parent: String,
    /// Commits on the parent that this branch isn't built on yet.
    pub behind: usize,
    /// This branch's own commits.
    pub own: usize,
}

pub fn list(repo_path: &str) -> Result<Vec<Stacked>> {
    let repo = open(repo_path)?;
    let mut out = vec![];
    for (b, _) in repo.branches(Some(BranchType::Local))?.flatten() {
        let Some(name) = b.name().ok().flatten().map(str::to_string) else { continue };
        let Some(parent) = parent_of(&repo, &name) else { continue };
        let (t, p) = (tip(&repo, &name)?, tip(&repo, &parent)?);
        let base = own_base(&repo, &name, t, p)?;
        let (own, _) = repo.graph_ahead_behind(t, base)?;
        let (behind, _) = repo.graph_ahead_behind(p, repo.merge_base(t, p)?)?;
        out.push(Stacked { branch: name, parent, behind, own });
    }
    out.sort_by(|a, b| a.branch.cmp(&b.branch));
    Ok(out)
}

/// Where the branch's own commits start: the recorded base while it's still
/// in the branch's history, otherwise where it meets the parent.
fn own_base(repo: &Repository, branch: &str, t: Oid, p: Oid) -> Result<Oid> {
    let recorded = repo
        .config()?
        .get_string(&key(branch, "arborbase"))
        .ok()
        .and_then(|s| Oid::from_str(&s).ok())
        .filter(|b| *b == t || repo.graph_descendant_of(t, *b).unwrap_or(false));
    Ok(match recorded {
        Some(b) => b,
        None => repo.merge_base(t, p)?,
    })
}

/// Replay `branch`'s own commits on its parent's tip. Checks the branch out.
/// Pauses on conflicts like any rebase (Continue/Skip/Abort).
pub fn restack(repo_path: &str, branch: &str) -> Result<RebaseOutcome> {
    let repo = open(repo_path)?;
    let Some(parent) = parent_of(&repo, branch) else { return err(format!("{branch} isn't stacked on another branch.")) };
    super::rebase::require_not_paused(&repo)?;
    let (t, p) = (tip(&repo, branch)?, tip(&repo, &parent)?);
    if t == p || repo.graph_descendant_of(t, p)? {
        record_base(&repo, branch, &p.to_string())?;
        return Ok(RebaseOutcome { result: "up-to-date", head: t.to_string(), conflicts: vec![] });
    }
    let base = own_base(&repo, branch, t, p)?;
    if head_branch(&repo).as_deref() != Some(branch) {
        require_tracked_clean(&repo, "switch branches")?;
        checkout_branch(repo_path, branch)?;
    }
    let repo = open(repo_path)?;
    require_tracked_clean(&repo, "restack")?;
    let mut walk = repo.revwalk()?;
    walk.push(t)?;
    walk.hide(base)?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)?;
    let mut steps = vec![];
    for id in walk {
        let id = id?;
        if repo.find_commit(id)?.parent_count() > 1 {
            return err(format!("{branch} contains a merge commit ({}); restack it by merging {parent} instead.", short(id)));
        }
        steps.push(Step { oid: id.to_string(), action: "pick".into(), message: None });
    }
    let onto = repo.find_commit(p)?;
    if steps.is_empty() {
        // No commits of its own: just move it to the parent.
        super::advance_head(&repo, p, &format!("restack {branch} onto {parent}"))?;
        record_base(&repo, branch, &p.to_string())?;
        return Ok(RebaseOutcome { result: "rebased", head: p.to_string(), conflicts: vec![] });
    }
    start_with(
        &repo,
        format!("Restack {branch} onto {parent}"),
        parent.clone(),
        Some(onto),
        steps,
        &format!("restack {branch} onto {parent}"),
        Some((branch.to_string(), p.to_string())),
    )
}

/// The branches stacked (directly or not) on `branch`, parents before children.
pub fn descendants(repo_path: &str, branch: &str) -> Result<Vec<String>> {
    let all = list(repo_path)?;
    let mut out = vec![];
    let mut queue = vec![branch.to_string()];
    while let Some(b) = queue.pop() {
        for s in all.iter().filter(|s| s.parent == b) {
            if !out.contains(&s.branch) {
                out.push(s.branch.clone());
                queue.insert(0, s.branch.clone());
            }
        }
    }
    Ok(out)
}
