//! Rebasing, including interactive rebase (reorder, squash, fixup, reword, drop).
//!
//! Commits are replayed in memory. When one conflicts, the rebase pauses:
//! the conflict is put in the working tree to resolve, then Continue, Skip
//! or Abort. The branch only moves when the rebase finishes.

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
    /// up-to-date | fast-forward | rebased | unchanged | conflicts (paused)
    pub result: &'static str,
    pub head: String,
    /// Conflicted files when paused.
    pub conflicts: Vec<String>,
}

/// Tree produced by applying `c`'s changes on top of `onto`; None when they conflict.
fn pick_tree(repo: &Repository, c: &Commit, onto: &Commit) -> Result<Option<Oid>> {
    if c.parent_count() > 1 {
        return err(format!(
            "{} is a merge commit. Rebasing across merges isn't supported; merge instead.",
            short(c.id())
        ));
    }
    if c.parent_count() == 1 && c.parent_id(0)? == onto.id() {
        return Ok(Some(c.tree_id()));
    }
    let mut idx = repo.cherrypick_commit(c, onto, 0, None)?;
    if idx.has_conflicts() {
        return Ok(None);
    }
    Ok(Some(idx.write_tree_to(repo)?))
}

enum Replayed {
    Done(Oid),
    /// Step `at` conflicts on top of `current`.
    Conflict { at: usize, current: Oid, replayed_any: bool },
}

/// The message a step's new commit gets.
fn step_message(step: &Step, c: &Commit, cur: Option<&Commit>) -> Result<String> {
    Ok(match step.action.as_str() {
        "reword" => {
            let m = git2::message_prettify(step.message.as_deref().unwrap_or(""), Some(b'#'))?;
            if m.trim().is_empty() {
                return err("A commit message can't be empty.");
            }
            m
        }
        "squash" | "fixup" => match (&step.message, step.action.as_str(), cur) {
            (Some(m), _, _) if !m.trim().is_empty() => git2::message_prettify(m, Some(b'#'))?,
            (_, "squash", Some(cur)) => format!(
                "{}\n\n{}",
                cur.message().unwrap_or("").trim_end(),
                c.message().unwrap_or("").trim_end()
            ) + "\n",
            (_, _, Some(cur)) => cur.message().unwrap_or("").to_string(),
            _ => c.message().unwrap_or("").to_string(),
        },
        _ => c.message().unwrap_or("").to_string(),
    })
}

/// Make the commit for `step` with `tree` on top of `cur` (or folded into it
/// for squash/fixup). None when the step turned out empty and is skipped.
fn commit_step(repo: &Repository, step: &Step, c: &Commit, cur: &Commit, tree: Oid, signing: bool) -> Result<Option<Oid>> {
    let sig = signature(repo)?;
    let msg = step_message(step, c, Some(cur))?;
    let tree = repo.find_tree(tree)?;
    let id = if step.action == "squash" || step.action == "fixup" {
        let parents: Vec<Commit> = cur.parents().collect();
        let prefs: Vec<&Commit> = parents.iter().collect();
        super::sign::create(repo, None, &cur.author(), &sig, &msg, &tree, &prefs, signing)?
    } else {
        if tree.id() == cur.tree_id() && c.parent_count() == 1 && c.tree_id() != c.parent(0)?.tree_id() {
            // Its changes are already there; skip it like git does.
            return Ok(None);
        }
        super::sign::create(repo, None, &c.author(), &sig, &msg, &tree, &[cur], signing)?
    };
    Ok(Some(id))
}

fn replay(repo: &Repository, onto: Option<Commit>, steps: &[Step], mut replayed_any: bool) -> Result<Replayed> {
    let sig = signature(repo)?;
    let signing = super::sign::wanted(repo, None);
    let mut current: Option<Commit> = onto;
    for (at, step) in steps.iter().enumerate() {
        let c = repo.find_commit(oid(&step.oid)?)?;
        match step.action.as_str() {
            "drop" => continue,
            "pick" | "reword" | "squash" | "fixup" => {
                let fold = step.action == "squash" || step.action == "fixup";
                if fold && !replayed_any {
                    return err("Squash and fixup need an earlier commit to combine with.");
                }
                let Some(cur) = current.clone() else {
                    // Only the root commit can start a history with no base.
                    if c.parent_count() != 0 {
                        return err("The first commit in this history can't be moved.");
                    }
                    let next = if step.action == "reword" {
                        let msg = step_message(step, &c, None)?;
                        let id = super::sign::create(repo, None, &c.author(), &sig, &msg, &c.tree()?, &[], signing)?;
                        repo.find_commit(id)?
                    } else {
                        c.clone()
                    };
                    current = Some(next);
                    replayed_any = true;
                    continue;
                };
                if step.action == "pick" && c.parent_count() == 1 && c.parent_id(0)? == cur.id() {
                    current = Some(c.clone());
                    replayed_any = true;
                    continue;
                }
                let Some(tree) = pick_tree(repo, &c, &cur)? else {
                    return Ok(Replayed::Conflict { at, current: cur.id(), replayed_any });
                };
                if let Some(next) = commit_step(repo, step, &c, &cur, tree, signing)? {
                    current = Some(repo.find_commit(next)?);
                    replayed_any = true;
                }
            }
            other => return err(format!("Unknown rebase action '{other}'.")),
        }
    }
    current.map(|c| Replayed::Done(c.id())).ok_or_else(|| super::Error::Msg("Every commit would be dropped.".into()))
}

// ---- Pausing on conflicts ----
//
// When a commit conflicts, HEAD is detached at the commits replayed so far,
// the conflicting commit is applied to the working tree with conflict
// markers, and where we are is saved in .git/arbor-rebase.json. The branch
// itself doesn't move until the rebase finishes, so Abort is always safe.

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Paused {
    /// Undo label, also shown in the banner ("Rebase dev onto main").
    pub label: String,
    /// The branch being rebased (refs/heads/…), None when HEAD was detached.
    pub branch: Option<String>,
    pub orig_head: String,
    /// What the replayed commits sit on, for the conflict wizard ("main").
    pub onto_label: String,
    /// The step that conflicted, and those after it.
    pub step: Step,
    pub rest: Vec<Step>,
    pub replayed_any: bool,
    /// Progress: this is step `done + 1` of `total`.
    pub done: usize,
    pub total: usize,
    /// A stacked branch being restacked: (branch, new base) to record when done.
    #[serde(default)]
    pub stack: Option<(String, String)>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PausedInfo {
    pub label: String,
    pub branch: Option<String>,
    pub step: usize,
    pub total: usize,
    pub action: String,
    pub commit: String,
    pub summary: String,
}

fn state_file(repo: &Repository) -> std::path::PathBuf {
    repo.path().join("arbor-rebase.json")
}

pub fn paused(repo: &Repository) -> Option<Paused> {
    let text = std::fs::read_to_string(state_file(repo)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn paused_info(repo: &Repository) -> Option<PausedInfo> {
    let p = paused(repo)?;
    let c = repo.find_commit(oid(&p.step.oid).ok()?).ok()?;
    Some(PausedInfo {
        label: p.label,
        branch: p.branch.map(|b| b.trim_start_matches("refs/heads/").to_string()),
        step: p.done + 1,
        total: p.total,
        action: p.step.action,
        commit: short(c.id()),
        summary: super::summary(&c),
    })
}

/// Refuse to start something new while a rebase is paused.
pub fn require_not_paused(repo: &Repository) -> Result<()> {
    if state_file(repo).exists() {
        return err("A rebase is paused on a conflict. Continue or abort it first.");
    }
    Ok(())
}

fn conflicted_paths(repo: &Repository) -> Result<Vec<String>> {
    Ok(repo
        .index()?
        .conflicts()?
        .filter_map(|c| c.ok())
        .filter_map(|c| c.our.or(c.their).or(c.ancestor))
        .map(|e| String::from_utf8_lossy(&e.path).to_string())
        .collect())
}

/// Stop at `p.step`: HEAD detached at `current`, its changes applied with markers.
fn pause(repo: &Repository, p: Paused, current: Oid) -> Result<RebaseOutcome> {
    let cur = repo.find_commit(current)?;
    let mut co = git2::build::CheckoutBuilder::new();
    co.force();
    repo.checkout_tree(cur.as_object(), Some(&mut co))?;
    repo.set_head_detached(current)?;
    let c = repo.find_commit(oid(&p.step.oid)?)?;
    let mut opts = git2::CherrypickOptions::new();
    repo.cherrypick(&c, Some(&mut opts))?;
    // Git's own cherry-pick state would make this look like a cherry-pick.
    for f in ["CHERRY_PICK_HEAD", "MERGE_MSG"] {
        let _ = std::fs::remove_file(repo.path().join(f));
    }
    std::fs::write(repo.path().join("REBASE_HEAD"), format!("{}\n", c.id()))?;
    std::fs::write(state_file(repo), serde_json::to_string_pretty(&p)?)?;
    Ok(RebaseOutcome { result: "conflicts", head: current.to_string(), conflicts: conflicted_paths(repo)? })
}

/// Point the branch (or detached HEAD) at `new` and check it out.
fn finish(repo: &Repository, p: &Paused, new: Oid, reflog: &str) -> Result<RebaseOutcome> {
    let commit = repo.find_commit(new)?;
    let mut co = git2::build::CheckoutBuilder::new();
    // Coming back from a pause the working tree is ours to replace; otherwise
    // don't overwrite anything (like any checkout).
    if state_file(repo).exists() {
        co.force();
    } else {
        co.safe();
    }
    repo.checkout_tree(commit.as_object(), Some(&mut co))?;
    match &p.branch {
        Some(b) => {
            repo.reference(b, new, true, reflog)?;
            repo.set_head(b)?;
        }
        None => repo.set_head_detached(new)?,
    }
    let _ = std::fs::remove_file(state_file(repo));
    let _ = std::fs::remove_file(repo.path().join("REBASE_HEAD"));
    repo.cleanup_state()?;
    if let Some((branch, base)) = &p.stack {
        super::stack::record_base(repo, branch, base)?;
    }
    undo::record(
        repo,
        p.label.clone(),
        Action::ResetHead { oid: Some(p.orig_head.clone()), mode: "hard".into(), expect: new.to_string(), branch: head_branch(repo) },
    );
    Ok(RebaseOutcome { result: "rebased", head: new.to_string(), conflicts: vec![] })
}

/// Run `steps` on top of `onto`, finishing or pausing at a conflict.
fn run(repo: &Repository, mut p: Paused, onto: Option<Commit>, steps: Vec<Step>, replayed_any: bool, reflog: &str) -> Result<RebaseOutcome> {
    match replay(repo, onto, &steps, replayed_any)? {
        Replayed::Done(new) => {
            if new.to_string() == p.orig_head && !state_file(repo).exists() {
                let _ = std::fs::remove_file(state_file(repo));
                return Ok(RebaseOutcome { result: "unchanged", head: new.to_string(), conflicts: vec![] });
            }
            finish(repo, &p, new, reflog)
        }
        Replayed::Conflict { at, current, replayed_any } => {
            p.done = p.total - steps.len() + at;
            p.step = steps[at].clone();
            p.rest = steps[at + 1..].to_vec();
            p.replayed_any = replayed_any;
            pause(repo, p, current)
        }
    }
}

fn start(repo: &Repository, label: String, onto_label: String, onto: Option<Commit>, steps: Vec<Step>, reflog: &str) -> Result<RebaseOutcome> {
    start_with(repo, label, onto_label, onto, steps, reflog, None)
}

/// Replay `steps` (oldest first) on top of `onto` for the checked-out
/// branch, pausing on conflicts. `stack` is recorded when it finishes.
pub fn start_with(
    repo: &Repository,
    label: String,
    onto_label: String,
    onto: Option<Commit>,
    steps: Vec<Step>,
    reflog: &str,
    stack: Option<(String, String)>,
) -> Result<RebaseOutcome> {
    let head = repo.head()?.peel_to_commit()?;
    let branch = if repo.head_detached().unwrap_or(false) { None } else { repo.head()?.name().ok().map(str::to_string) };
    let p = Paused {
        label,
        branch,
        orig_head: head.id().to_string(),
        onto_label,
        step: steps.first().cloned().unwrap_or(Step { oid: head.id().to_string(), action: "pick".into(), message: None }),
        rest: vec![],
        replayed_any: false,
        done: 0,
        total: steps.len(),
        stack,
    };
    run(repo, p, onto, steps, false, reflog)
}

/// After resolving the conflicts: commit the paused step and carry on.
pub fn continue_rebase(repo_path: &str) -> Result<RebaseOutcome> {
    let repo = open(repo_path)?;
    let Some(p) = paused(&repo) else { return err("No rebase is paused.") };
    let mut index = repo.index()?;
    if index.has_conflicts() {
        return err("Resolve all conflicts first.");
    }
    // Everything changed in the working tree counts, like `git add -u`.
    index.update_all(["*"].iter(), None)?;
    index.write()?;
    let tree = index.write_tree()?;
    let cur = repo.head()?.peel_to_commit()?;
    let c = repo.find_commit(oid(&p.step.oid)?)?;
    let signing = super::sign::wanted(&repo, None);
    let next = match commit_step(&repo, &p.step, &c, &cur, tree, signing)? {
        Some(n) => repo.find_commit(n)?,
        None => cur,
    };
    let reflog = format!("rebase (continue): {}", p.label);
    let rest = p.rest.clone();
    run(&repo, p, Some(next), rest, true, &reflog)
}

/// Leave the paused commit out and carry on.
pub fn skip_rebase(repo_path: &str) -> Result<RebaseOutcome> {
    let repo = open(repo_path)?;
    let Some(p) = paused(&repo) else { return err("No rebase is paused.") };
    let cur = repo.head()?.peel_to_commit()?;
    let reflog = format!("rebase (skip): {}", p.label);
    let rest = p.rest.clone();
    let any = p.replayed_any;
    run(&repo, p, Some(cur), rest, any, &reflog)
}

/// Put everything back as it was before the rebase.
pub fn abort_rebase(repo_path: &str) -> Result<()> {
    let repo = open(repo_path)?;
    let Some(p) = paused(&repo) else { return err("No rebase is paused.") };
    let orig = repo.find_commit(oid(&p.orig_head)?)?;
    let mut co = git2::build::CheckoutBuilder::new();
    co.force().remove_untracked(false);
    repo.checkout_tree(orig.as_object(), Some(&mut co))?;
    match &p.branch {
        Some(b) => repo.set_head(b)?,
        None => repo.set_head_detached(orig.id())?,
    }
    // Make sure the index matches too (no conflict entries left).
    repo.reset(orig.as_object(), git2::ResetType::Hard, None)?;
    let _ = std::fs::remove_file(state_file(&repo));
    let _ = std::fs::remove_file(repo.path().join("REBASE_HEAD"));
    repo.cleanup_state()?;
    Ok(())
}

/// Rebase the current branch onto `onto` (a branch, remote branch, tag or commit).
pub fn rebase(repo_path: &str, onto: &str) -> Result<RebaseOutcome> {
    let repo = open(repo_path)?;
    if repo.state() != git2::RepositoryState::Clean {
        return err("Finish or abort the operation in progress first.");
    }
    require_not_paused(&repo)?;
    require_tracked_clean(&repo, "rebase")?;
    let head = repo.head()?.peel_to_commit()?;
    let onto_id = resolve(&repo, onto)?;
    let base = repo.merge_base(head.id(), onto_id)?;
    let here = head_branch(&repo).unwrap_or_else(|| short(head.id()));
    if base == onto_id {
        return Ok(RebaseOutcome { result: "up-to-date", head: head.id().to_string(), conflicts: vec![] });
    }
    if base == head.id() {
        advance_head(&repo, onto_id, &format!("rebase onto {onto}: fast-forward"))?;
        undo::record_head_move(&repo, format!("Rebase {here} onto {onto}"), Some(head.id()), "hard");
        return Ok(RebaseOutcome { result: "fast-forward", head: onto_id.to_string(), conflicts: vec![] });
    }
    let mut walk = repo.revwalk()?;
    walk.push(head.id())?;
    walk.hide(onto_id)?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)?;
    let steps: Vec<Step> = walk
        .map(|o| o.map(|o| Step { oid: o.to_string(), action: "pick".into(), message: None }))
        .collect::<std::result::Result<_, _>>()?;
    let onto_commit = repo.find_commit(onto_id)?;
    start(&repo, format!("Rebase {here} onto {onto}"), onto.to_string(), Some(onto_commit), steps, &format!("rebase onto {onto}"))
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
    require_not_paused(&repo)?;
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
    let _ = head;
    let onto = base_id.map(|b| repo.find_commit(b)).transpose()?;
    let onto_label = base_id.map(short).unwrap_or_else(|| "the start".into());
    start(&repo, label.to_string(), onto_label, onto, steps.to_vec(), &format!("rebase -i: {label}"))
}

/// Fold the staged changes into `target`, an earlier commit on the current
/// branch (Tower's "fixup"), and replay the commits after it.
///
/// Everything happens in memory: the branch moves and the staged changes are
/// now part of `target`, but the working tree is never touched, so unstaged
/// changes and untracked files stay exactly as they were. Refuses (changing
/// nothing) when later commits would conflict with the folded changes.
pub fn fold_into(repo_path: &str, target: &str) -> Result<RebaseOutcome> {
    let repo = open(repo_path)?;
    if repo.state() != git2::RepositoryState::Clean {
        return err("Finish or abort the operation in progress first.");
    }
    require_not_paused(&repo)?;
    let head = repo.head()?.peel_to_commit()?;
    let target = repo.find_commit(oid(target)?)?;
    if target.id() != head.id() && !repo.graph_descendant_of(head.id(), target.id())? {
        return err("That commit isn't on the current branch.");
    }
    let mut index = repo.index()?;
    if index.has_conflicts() {
        return err("Resolve the conflicts first.");
    }
    let staged = index.write_tree()?;
    if staged == head.tree_id() {
        return err("Nothing is staged. Stage the changes you want to fold in first.");
    }
    let label = format!("Fold staged changes into “{}”", super::summary(&target));
    let sig = signature(&repo)?;

    // The staged changes as a commit on top of HEAD, then: target, that
    // commit as a fixup of it, and the rest of the branch after target.
    let fix = repo.commit(None, &sig, &sig, &format!("fixup! {}", super::summary(&target)), &repo.find_tree(staged)?, &[&head])?;
    let mut walk = repo.revwalk()?;
    walk.push(head.id())?;
    for p in target.parent_ids() {
        walk.hide(p)?;
    }
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)?;
    let mut steps = vec![];
    for o in walk {
        let o = o?;
        let c = repo.find_commit(o)?;
        if c.parent_count() > 1 {
            return err(format!("{} is a merge commit; folding changes in across merges isn't supported.", short(o)));
        }
        steps.push(Step { oid: o.to_string(), action: "pick".into(), message: None });
        if o == target.id() {
            steps.push(Step { oid: fix.to_string(), action: "fixup".into(), message: None });
        }
    }
    let onto = match target.parent_count() {
        0 => None,
        _ => Some(target.parent(0)?),
    };
    let new = match replay(&repo, onto, &steps, false)? {
        Replayed::Done(new) => new,
        Replayed::Conflict { at, .. } => {
            let c = repo.find_commit(oid(&steps[at].oid)?)?;
            return err(format!(
                "The staged changes can't be folded into “{}”: they conflict with “{}”, a later commit. Nothing was changed.",
                super::summary(&target),
                super::summary(&c),
            ));
        }
    };
    // The branch ends where it would have anyway: HEAD plus the staged changes.
    if repo.find_commit(new)?.tree_id() != staged {
        return err("Folding the changes in would also change later commits' results. Nothing was changed.");
    }
    let reflog = format!("fold staged changes into {}", short(target.id()));
    match head_branch(&repo) {
        Some(b) => {
            repo.reference(&format!("refs/heads/{b}"), new, true, &reflog)?;
        }
        None => repo.set_head_detached(new)?,
    }
    // The index already holds exactly the new HEAD's tree; just record it.
    index.write()?;
    undo::record(
        &repo,
        label,
        Action::ResetHead { oid: Some(head.id().to_string()), mode: "soft".into(), expect: new.to_string(), branch: head_branch(&repo) },
    );
    Ok(RebaseOutcome { result: "rebased", head: new.to_string(), conflicts: vec![] })
}
