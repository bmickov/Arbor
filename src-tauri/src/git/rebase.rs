//! Rebasing, including interactive rebase (reorder, squash, fixup, reword, drop).
//!
//! Commits are replayed in memory. Nothing in the repository changes until
//! every commit has been replayed cleanly; if one conflicts, the operation
//! stops and the branch is left exactly as it was.

use super::branches::{checkout_branch, resolve};
use super::undo::{self, Action};
use super::{advance_head, err, head_branch, oid, open, require_tracked_clean, short, signature, Result};
use git2::{Commit, Oid, Repository, Sort};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub oid: String,
    /// pick | reword | squash | fixup | drop
    pub action: String,
    /// New message for reword (and optionally squash/fixup).
    pub message: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RebaseOutcome {
    /// up-to-date | fast-forward | rebased | unchanged
    pub result: &'static str,
    pub head: String,
}

fn conflict_msg(c: &Commit) -> super::Error {
    super::Error::Msg(format!(
        "Commit {} \"{}\" conflicts when replayed, so nothing was changed. \
         Try merging instead, or resolve it with git on the command line.",
        short(c.id()),
        super::summary(c)
    ))
}

/// Tree produced by applying `c`'s changes on top of `onto`.
fn pick_tree(repo: &Repository, c: &Commit, onto: &Commit) -> Result<Oid> {
    if c.parent_count() > 1 {
        return err(format!(
            "{} is a merge commit. Rebasing across merges isn't supported; merge instead.",
            short(c.id())
        ));
    }
    if c.parent_count() == 1 && c.parent_id(0)? == onto.id() {
        return Ok(c.tree_id());
    }
    let mut idx = repo.cherrypick_commit(c, onto, 0, None)?;
    if idx.has_conflicts() {
        return Err(conflict_msg(c));
    }
    Ok(idx.write_tree_to(repo)?)
}

fn replay(repo: &Repository, onto: Option<Commit>, steps: &[Step]) -> Result<Oid> {
    let sig = signature(repo)?;
    let signing = super::sign::wanted(repo, None);
    let mut current: Option<Commit> = onto;
    let mut replayed_any = false;
    for step in steps {
        let c = repo.find_commit(oid(&step.oid)?)?;
        match step.action.as_str() {
            "drop" => continue,
            "pick" | "reword" => {
                let reword = step.action == "reword";
                let msg = if reword {
                    git2::message_prettify(step.message.as_deref().unwrap_or(""), Some(b'#'))?
                } else {
                    c.message().unwrap_or("").to_string()
                };
                if reword && msg.trim().is_empty() {
                    return err("A commit message can't be empty.");
                }
                let next = match &current {
                    None => {
                        // Only the root commit can start a history with no base.
                        if c.parent_count() != 0 {
                            return err("The first commit in this history can't be moved.");
                        }
                        if reword {
                            let id = super::sign::create(repo, None, &c.author(), &sig, &msg, &c.tree()?, &[], signing)?;
                            repo.find_commit(id)?
                        } else {
                            c.clone()
                        }
                    }
                    Some(cur) => {
                        if !reword && c.parent_count() == 1 && c.parent_id(0)? == cur.id() {
                            c.clone()
                        } else {
                            let tree = pick_tree(repo, &c, cur)?;
                            if tree == cur.tree_id() && c.parent_count() == 1 && c.tree_id() != c.parent(0)?.tree_id() {
                                // Its changes are already there; skip it like git does.
                                continue;
                            }
                            let tree = repo.find_tree(tree)?;
                            let id = super::sign::create(repo, None, &c.author(), &sig, &msg, &tree, &[cur], signing)?;
                            repo.find_commit(id)?
                        }
                    }
                };
                current = Some(next);
                replayed_any = true;
            }
            "squash" | "fixup" => {
                let Some(cur) = current.clone().filter(|_| replayed_any) else {
                    return err("Squash and fixup need an earlier commit to combine with.");
                };
                let tree = repo.find_tree(pick_tree(repo, &c, &cur)?)?;
                let msg = match (&step.message, step.action.as_str()) {
                    (Some(m), _) if !m.trim().is_empty() => git2::message_prettify(m, Some(b'#'))?,
                    (_, "squash") => format!(
                        "{}\n\n{}",
                        cur.message().unwrap_or("").trim_end(),
                        c.message().unwrap_or("").trim_end()
                    ) + "\n",
                    _ => cur.message().unwrap_or("").to_string(),
                };
                let parents: Vec<Commit> = cur.parents().collect();
                let prefs: Vec<&Commit> = parents.iter().collect();
                let id = super::sign::create(repo, None, &cur.author(), &sig, &msg, &tree, &prefs, signing)?;
                current = Some(repo.find_commit(id)?);
            }
            other => return err(format!("Unknown rebase action '{other}'.")),
        }
    }
    current.map(|c| c.id()).ok_or_else(|| super::Error::Msg("Every commit would be dropped.".into()))
}

/// Rebase the current branch onto `onto` (a branch, remote branch, tag or commit).
pub fn rebase(repo_path: &str, onto: &str) -> Result<RebaseOutcome> {
    let repo = open(repo_path)?;
    if repo.state() != git2::RepositoryState::Clean {
        return err("Finish or abort the operation in progress first.");
    }
    require_tracked_clean(&repo, "rebase")?;
    let head = repo.head()?.peel_to_commit()?;
    let onto_id = resolve(&repo, onto)?;
    let base = repo.merge_base(head.id(), onto_id)?;
    let here = head_branch(&repo).unwrap_or_else(|| short(head.id()));
    if base == onto_id {
        return Ok(RebaseOutcome { result: "up-to-date", head: head.id().to_string() });
    }
    if base == head.id() {
        advance_head(&repo, onto_id, &format!("rebase onto {onto}: fast-forward"))?;
        undo::record_head_move(&repo, format!("Rebase {here} onto {onto}"), Some(head.id()), "hard");
        return Ok(RebaseOutcome { result: "fast-forward", head: onto_id.to_string() });
    }
    let mut walk = repo.revwalk()?;
    walk.push(head.id())?;
    walk.hide(onto_id)?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)?;
    let steps: Vec<Step> = walk
        .map(|o| o.map(|o| Step { oid: o.to_string(), action: "pick".into(), message: None }))
        .collect::<std::result::Result<_, _>>()?;
    let onto_commit = repo.find_commit(onto_id)?;
    let new = replay(&repo, Some(onto_commit), &steps)?;
    advance_head(&repo, new, &format!("rebase onto {onto}"))?;
    undo::record(
        &repo,
        format!("Rebase {here} onto {onto}"),
        Action::ResetHead { oid: Some(head.id().to_string()), mode: "hard".into(), expect: new.to_string(), branch: head_branch(&repo) },
    );
    Ok(RebaseOutcome { result: "rebased", head: new.to_string() })
}

/// Rebase `branch` onto `onto`, checking `branch` out first when needed.
pub fn rebase_branch(repo_path: &str, branch: &str, onto: &str) -> Result<RebaseOutcome> {
    let repo = open(repo_path)?;
    if head_branch(&repo).as_deref() != Some(branch) {
        require_tracked_clean(&repo, "switch branches")?;
        checkout_branch(repo_path, branch)?;
    }
    rebase(repo_path, onto)
}

/// Commits after `base` up to HEAD along first parents, oldest first.
fn commits_after(repo: &Repository, base: Option<Oid>) -> Result<Vec<Oid>> {
    let mut out = vec![];
    let mut c = repo.head()?.peel_to_commit()?;
    loop {
        if Some(c.id()) == base {
            break;
        }
        out.push(c.id());
        if c.parent_count() == 0 {
            if base.is_some() {
                return err("That commit isn't part of the current branch.");
            }
            break;
        }
        c = c.parent(0)?;
    }
    out.reverse();
    Ok(out)
}

/// Rewrite the commits after `base` (exclusive) as described by `steps`,
/// listed oldest first. `base` None means the whole history.
pub fn interactive(repo_path: &str, base: Option<&str>, steps: &[Step], label: &str) -> Result<RebaseOutcome> {
    let repo = open(repo_path)?;
    if repo.state() != git2::RepositoryState::Clean {
        return err("Finish or abort the operation in progress first.");
    }
    require_tracked_clean(&repo, "rewrite history")?;
    let head = repo.head()?.peel_to_commit()?;
    let base_id = base.map(oid).transpose()?;
    let expected = commits_after(&repo, base_id)?;
    let mut given: Vec<Oid> = steps.iter().map(|s| oid(&s.oid)).collect::<Result<_>>()?;
    let mut sorted = expected.clone();
    sorted.sort();
    given.sort();
    if sorted != given {
        return err("The branch changed since the list was loaded. Refresh and try again.");
    }
    let onto = base_id.map(|b| repo.find_commit(b)).transpose()?;
    let new = replay(&repo, onto, steps)?;
    if new == head.id() {
        return Ok(RebaseOutcome { result: "unchanged", head: new.to_string() });
    }
    advance_head(&repo, new, &format!("rebase -i: {label}"))?;
    undo::record(
        &repo,
        label.to_string(),
        Action::ResetHead { oid: Some(head.id().to_string()), mode: "hard".into(), expect: new.to_string(), branch: head_branch(&repo) },
    );
    Ok(RebaseOutcome { result: "rebased", head: new.to_string() })
}
