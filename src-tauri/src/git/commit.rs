//! Committing, amending, cherry-picking, reverting and resetting.

use super::sign;
use super::undo::{self, Action};
use super::{advance_head, err, head_oid, hooks, oid, open, require_tracked_clean, short, signature, Result};
use git2::{Oid, Repository, ResetType};
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CommitResult {
    pub oid: String,
    pub short: String,
    /// Conflicted files when a cherry-pick or revert stopped to be resolved.
    pub conflicts: Vec<String>,
}

fn tidy(msg: &str) -> Result<String> {
    let m = git2::message_prettify(msg, Some(b'#'))?;
    if m.trim().is_empty() {
        return err("Write a commit message first.");
    }
    Ok(m)
}

pub fn commit(repo_path: &str, message: &str, amend: bool, skip_hooks: bool) -> Result<CommitResult> {
    commit_signed(repo_path, message, amend, skip_hooks, None)
}

/// Commit, signing per `sign` (None follows commit.gpgsign).
pub fn commit_signed(repo_path: &str, message: &str, amend: bool, skip_hooks: bool, sign: Option<bool>) -> Result<CommitResult> {
    let mut repo = open(repo_path)?;
    if super::rebase::paused(&repo).is_some() {
        return err("A rebase is paused. Resolve the conflicts, then choose Continue.");
    }
    let merging = repo.state() == git2::RepositoryState::Merge;
    let state = repo.state();
    // A cherry-pick or revert that stopped on conflicts: one parent, and a
    // cherry-pick keeps the original author.
    let picking = matches!(state, git2::RepositoryState::CherryPick | git2::RepositoryState::Revert);
    let picked_author = if state == git2::RepositoryState::CherryPick {
        std::fs::read_to_string(repo.path().join("CHERRY_PICK_HEAD"))
            .ok()
            .and_then(|s| Oid::from_str(s.trim()).ok())
            .and_then(|id| repo.find_commit(id).ok())
            .map(|c| c.author().to_owned())
    } else {
        None
    };
    let mut merge_heads: Vec<Oid> = vec![];
    if merging {
        repo.mergehead_foreach(|id| {
            merge_heads.push(*id);
            true
        })?;
    }
    let repo = repo;
    let mut index = repo.index()?;
    if index.has_conflicts() {
        return err("Resolve all conflicts before committing.");
    }
    let mut msg = tidy(message)?;
    if !skip_hooks {
        hooks::pre_commit(&repo)?;
        msg = tidy(&hooks::commit_msg(&repo, &msg)?)?;
        // The pre-commit hook may have staged changes.
        index = repo.index()?;
        index.read(true)?;
    }
    let tree = repo.find_tree(index.write_tree()?)?;
    let sig = signature(&repo)?;
    let before = head_oid(&repo);
    let sign = sign::wanted(&repo, sign);

    let new = if amend {
        let head = repo.head()?.peel_to_commit()?;
        if sign {
            let parents: Vec<git2::Commit> = head.parents().collect();
            let refs: Vec<&git2::Commit> = parents.iter().collect();
            let reflog = format!("commit (amend): {}", super::first_line(&msg));
            sign::create(&repo, Some(&reflog), &head.author(), &sig, &msg, &tree, &refs, true)?
        } else {
            head.amend(Some("HEAD"), None, Some(&sig), None, Some(&msg), Some(&tree))?
        }
    } else {
        let mut parents = vec![];
        if let Some(h) = before {
            parents.push(repo.find_commit(h)?);
        }
        if merging {
            for h in merge_heads {
                parents.push(repo.find_commit(h)?);
            }
        } else if parents.is_empty() && tree.is_empty() {
            // The first commit: refuse an empty one, as later commits do.
            return err("Nothing is staged. Stage some changes, then commit.");
        } else if let Some(p) = parents.first() {
            if p.tree_id() == tree.id() {
                return err("Nothing is staged. Stage some changes, then commit.");
            }
        }
        let refs: Vec<&git2::Commit> = parents.iter().collect();
        let kind = if refs.is_empty() { " (initial)" } else if merging { " (merge)" } else if picking { " (cherry-pick)" } else { "" };
        let reflog = format!("commit{kind}: {}", super::first_line(&msg));
        let author = picked_author.unwrap_or_else(|| sig.clone());
        let id = sign::create(&repo, Some(&reflog), &author, &sig, &msg, &tree, &refs, sign)?;
        if merging || picking {
            repo.cleanup_state()?;
        }
        id
    };
    if !skip_hooks {
        hooks::post_commit(&repo);
    }
    let label = if amend {
        format!("Amend commit {}", super::first_line(&msg))
    } else {
        format!("Commit {}", super::first_line(&msg))
    };
    record_commit_undo(&repo, label, before, new);
    Ok(CommitResult { oid: new.to_string(), short: short(new), conflicts: vec![] })
}

fn record_commit_undo(repo: &Repository, label: String, before: Option<Oid>, after: Oid) {
    undo::record(
        repo,
        label,
        Action::ResetHead {
            oid: before.map(|o| o.to_string()),
            mode: "soft".into(),
            expect: after.to_string(),
            branch: super::head_branch(repo),
        },
    );
}

/// The message of HEAD, used to prefill the composer when amending.
pub fn head_message(repo_path: &str) -> Result<Option<String>> {
    let repo = open(repo_path)?;
    let c = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    Ok(c.and_then(|c| c.message().ok().map(|m| m.trim_end().to_string())))
}

pub fn cherry_pick(repo_path: &str, id: &str) -> Result<CommitResult> {
    let repo = open(repo_path)?;
    if repo.state() != git2::RepositoryState::Clean {
        return err("Finish or abort the operation in progress first.");
    }
    super::rebase::require_not_paused(&repo)?;
    require_tracked_clean(&repo, "cherry-pick")?;
    let c = repo.find_commit(oid(id)?)?;
    let head = repo.head()?.peel_to_commit()?;
    let mainline = if c.parent_count() > 1 { 1 } else { 0 };
    let mut idx = repo.cherrypick_commit(&c, &head, mainline, None)?;
    if idx.has_conflicts() {
        // Stop like git: the conflicts go in the working tree to resolve, then commit.
        let mut opts = git2::CherrypickOptions::new();
        if mainline > 0 {
            opts.mainline(mainline);
        }
        repo.cherrypick(&c, Some(&mut opts))?;
        undo::record(&repo, format!("Cherry-pick {}", super::summary(&c)), Action::AbortMerge { oid: head.id().to_string() });
        return Ok(CommitResult { oid: head.id().to_string(), short: short(head.id()), conflicts: conflicted(&repo)? });
    }
    let tree = repo.find_tree(idx.write_tree_to(&repo)?)?;
    if tree.id() == head.tree_id() {
        return err("That commit's changes are already on this branch.");
    }
    let sig = signature(&repo)?;
    let msg = c.message().unwrap_or("");
    let new = sign::create(&repo, None, &c.author(), &sig, msg, &tree, &[&head], sign::wanted(&repo, None))?;
    advance_head(&repo, new, &format!("cherry-pick: {}", super::first_line(msg)))?;
    record_commit_undo(&repo, format!("Cherry-pick {}", super::first_line(msg)), Some(head.id()), new);
    Ok(CommitResult { oid: new.to_string(), short: short(new), conflicts: vec![] })
}

pub fn revert(repo_path: &str, id: &str) -> Result<CommitResult> {
    let repo = open(repo_path)?;
    if repo.state() != git2::RepositoryState::Clean {
        return err("Finish or abort the operation in progress first.");
    }
    super::rebase::require_not_paused(&repo)?;
    require_tracked_clean(&repo, "revert a commit")?;
    let c = repo.find_commit(oid(id)?)?;
    let head = repo.head()?.peel_to_commit()?;
    let mainline = if c.parent_count() > 1 { 1 } else { 0 };
    let mut idx = repo.revert_commit(&c, &head, mainline, None)?;
    if idx.has_conflicts() {
        let mut opts = git2::RevertOptions::new();
        if mainline > 0 {
            opts.mainline(mainline);
        }
        repo.revert(&c, Some(&mut opts))?;
        undo::record(&repo, format!("Revert {}", super::summary(&c)), Action::AbortMerge { oid: head.id().to_string() });
        return Ok(CommitResult { oid: head.id().to_string(), short: short(head.id()), conflicts: conflicted(&repo)? });
    }
    let tree = repo.find_tree(idx.write_tree_to(&repo)?)?;
    let sig = signature(&repo)?;
    let summary = super::summary(&c);
    let msg = format!("Revert \"{summary}\"\n\nThis reverts commit {}.\n", c.id());
    let new = sign::create(&repo, None, &sig, &sig, &msg, &tree, &[&head], sign::wanted(&repo, None))?;
    advance_head(&repo, new, &format!("revert: {summary}"))?;
    record_commit_undo(&repo, format!("Revert {summary}"), Some(head.id()), new);
    Ok(CommitResult { oid: new.to_string(), short: short(new), conflicts: vec![] })
}

/// Move the current branch to `id`. mode: soft | mixed | hard.
pub fn reset(repo_path: &str, id: &str, mode: &str) -> Result<()> {
    let repo = open(repo_path)?;
    let before = head_oid(&repo);
    let target = repo.find_object(oid(id)?, None)?;
    let kind = match mode {
        "soft" => ResetType::Soft,
        "hard" => ResetType::Hard,
        _ => ResetType::Mixed,
    };
    repo.reset(&target, kind, None)?;
    undo::record_head_move(&repo, format!("Reset to {}", short(target.id())), before, mode);
    Ok(())
}

fn conflicted(repo: &Repository) -> Result<Vec<String>> {
    Ok(repo
        .index()?
        .conflicts()?
        .filter_map(|c| c.ok())
        .filter_map(|c| c.our.or(c.their).or(c.ancestor))
        .map(|e| String::from_utf8_lossy(&e.path).to_string())
        .collect())
}
