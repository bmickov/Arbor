//! Merging branches and handling merges that stop on conflicts.

use super::branches::{checkout_branch, resolve};
use super::undo::{self, Action};
use super::{advance_head, err, head_branch, open, require_tracked_clean, short, signature, Result};
use git2::{build::CheckoutBuilder, MergeOptions};
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MergeOutcome {
    /// up-to-date | fast-forward | merged | conflicts
    pub result: &'static str,
    pub conflicts: Vec<String>,
}

/// Merge `source` (branch, remote branch, tag or commit) into the current branch.
pub fn merge(repo_path: &str, source: &str, no_ff: bool) -> Result<MergeOutcome> {
    let repo = open(repo_path)?;
    if repo.state() != git2::RepositoryState::Clean {
        return err("Finish or abort the operation in progress first.");
    }
    require_tracked_clean(&repo, "merge")?;
    let theirs = resolve(&repo, source)?;
    let head = repo.head()?.peel_to_commit()?;
    // Annotate from the branch reference when there is one, so git's merge
    // message names the branch ("Merge branch 'x'") rather than a commit id.
    let reference = repo
        .find_branch(source, git2::BranchType::Local)
        .or_else(|_| repo.find_branch(source, git2::BranchType::Remote))
        .ok()
        .map(|b| b.into_reference());
    let annotated = match &reference {
        Some(r) => repo.reference_to_annotated_commit(r)?,
        None => repo.find_annotated_commit(theirs)?,
    };
    let (analysis, _) = repo.merge_analysis(&[&annotated])?;
    let here = head_branch(&repo).unwrap_or_else(|| short(head.id()));

    if analysis.is_up_to_date() {
        return Ok(MergeOutcome { result: "up-to-date", conflicts: vec![] });
    }
    if analysis.is_fast_forward() && !no_ff {
        advance_head(&repo, theirs, &format!("merge {source}: Fast-forward"))?;
        undo::record_head_move(&repo, format!("Merge {source} into {here}"), Some(head.id()), "hard");
        return Ok(MergeOutcome { result: "fast-forward", conflicts: vec![] });
    }

    let mut mo = MergeOptions::new();
    mo.find_renames(true);
    let mut co = CheckoutBuilder::new();
    co.safe().allow_conflicts(true).conflict_style_merge(true);
    repo.merge(&[&annotated], Some(&mut mo), Some(&mut co))?;

    let mut index = repo.index()?;
    if index.has_conflicts() {
        let conflicts = index
            .conflicts()?
            .filter_map(|c| c.ok())
            .filter_map(|c| c.our.or(c.their).or(c.ancestor))
            .map(|e| String::from_utf8_lossy(&e.path).to_string())
            .collect();
        undo::record(&repo, format!("Merge {source} into {here}"), Action::AbortMerge { oid: head.id().to_string() });
        return Ok(MergeOutcome { result: "conflicts", conflicts });
    }

    let tree = repo.find_tree(index.write_tree()?)?;
    let sig = signature(&repo)?;
    let mut msg = std::fs::read_to_string(repo.path().join("MERGE_MSG"))
        .ok()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| format!("Merge {source}\n"));
    if here != "main" && here != "master" && !msg.contains(" into ") {
        let first = msg.lines().next().unwrap_or("").to_string();
        msg = msg.replacen(&first, &format!("{first} into {here}"), 1);
    }
    let theirs_commit = repo.find_commit(theirs)?;
    let reflog = format!("merge {source}: Merge made by the 'ort' strategy.");
    let new = super::sign::create(&repo, Some(&reflog), &sig, &sig, &msg, &tree, &[&head, &theirs_commit], super::sign::wanted(&repo, None))?;
    repo.cleanup_state()?;
    undo::record(
        &repo,
        format!("Merge {source} into {here}"),
        Action::ResetHead { oid: Some(head.id().to_string()), mode: "hard".into(), expect: new.to_string(), branch: head_branch(&repo) },
    );
    Ok(MergeOutcome { result: "merged", conflicts: vec![] })
}

/// Merge `source` into `target`, checking `target` out first when needed.
pub fn merge_into(repo_path: &str, source: &str, target: &str, no_ff: bool) -> Result<MergeOutcome> {
    let repo = open(repo_path)?;
    if head_branch(&repo).as_deref() != Some(target) {
        require_tracked_clean(&repo, "switch branches")?;
        checkout_branch(repo_path, target)?;
    }
    merge(repo_path, source, no_ff)
}

/// Abandon a merge (or cherry-pick/revert) that stopped on conflicts.
pub fn abort(repo_path: &str) -> Result<()> {
    let repo = open(repo_path)?;
    if let Some(h) = super::head_oid(&repo) {
        super::hard_reset_to(&repo, h)?;
    }
    repo.cleanup_state()?;
    Ok(())
}
