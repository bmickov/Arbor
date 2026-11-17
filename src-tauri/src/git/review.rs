//! Looking back and across: the reflog (to recover lost commits and
//! branches), Branches Review (which branches are merged, stale or gone),
//! and comparing two branches.

use super::{err, open, short, summary, Error, Result};
use git2::{BranchType, Oid, Repository, Sort};
use serde::Serialize;
use std::collections::HashSet;

// ------------------------------------------------------------------ reflog

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReflogEntry {
    pub oid: String,
    pub short: String,
    /// What moved the ref there ("commit: …", "checkout: moving from a to b", …).
    pub message: String,
    pub time: i64,
    /// The commit's own first line, if the commit still exists.
    pub summary: Option<String>,
    pub author: Option<String>,
    /// Not reachable from any branch, tag or remote branch: only the reflog
    /// still points at it, so it's what you'd want to recover.
    pub lost: bool,
}

/// Recent positions of `refname` ("HEAD" or "refs/heads/<branch>"), newest first.
pub fn reflog(repo_path: &str, refname: &str, limit: usize) -> Result<Vec<ReflogEntry>> {
    let repo = open(repo_path)?;
    let log = repo.reflog(refname)?;
    let mut out = vec![];
    for e in log.iter().take(limit) {
        let id = e.id_new();
        if id.is_zero() {
            continue;
        }
        let c = repo.find_commit(id).ok();
        out.push(ReflogEntry {
            oid: id.to_string(),
            short: short(id),
            message: e.message().ok().flatten().unwrap_or("").to_string(),
            time: e.committer().when().seconds(),
            summary: c.as_ref().map(summary),
            author: c.as_ref().and_then(|c| c.author().name().ok().map(|s| s.to_string())),
            lost: false,
        });
    }
    // Mark entries no ref reaches. Walk from every ref until all the reflog's
    // commits have been seen (usually quickly, as they're recent).
    let mut unseen: HashSet<Oid> = out.iter().filter_map(|e| Oid::from_str(&e.oid).ok()).collect();
    if !unseen.is_empty() {
        let mut walk = repo.revwalk()?;
        walk.set_sorting(Sort::TIME)?;
        for r in repo.references()?.flatten() {
            if let Some(t) = r.resolve().ok().and_then(|r| r.target()) {
                // Tags can point at non-commits; ignore those.
                let _ = walk.push(t);
            }
        }
        for id in walk.flatten() {
            unseen.remove(&id);
            if unseen.is_empty() {
                break;
            }
        }
    }
    for e in &mut out {
        e.lost = Oid::from_str(&e.oid).map(|o| unseen.contains(&o)).unwrap_or(false) && e.summary.is_some();
    }
    Ok(out)
}

// -------------------------------------------------------- branches review

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRow {
    pub name: String,
    pub is_head: bool,
    pub is_base: bool,
    /// merged | squashed (its changes are in the base, e.g. via a squash or
    /// rebase merge) | unmerged
    pub merged: &'static str,
    /// Commits on this branch that the base doesn't have, and the reverse.
    pub ahead: usize,
    pub behind: usize,
    pub upstream: Option<String>,
    pub upstream_gone: bool,
    /// Commits to push / pull relative to the upstream.
    pub to_push: usize,
    pub to_pull: usize,
    pub time: i64,
    pub author: String,
    pub summary: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    /// The branch everything is compared with.
    pub base: String,
    pub rows: Vec<ReviewRow>,
}

/// The branch to compare others with: what origin/HEAD points at, else
/// main, master, trunk or develop (local or on origin), else HEAD's branch.
pub fn default_base(repo: &Repository) -> Option<String> {
    if let Ok(r) = repo.find_reference("refs/remotes/origin/HEAD") {
        if let Ok(Some(t)) = r.symbolic_target() {
            let name = t.trim_start_matches("refs/remotes/origin/").to_string();
            if repo.find_branch(&name, BranchType::Local).is_ok() {
                return Some(name);
            }
            return Some(format!("origin/{name}"));
        }
    }
    for n in ["main", "master", "trunk", "develop"] {
        if repo.find_branch(n, BranchType::Local).is_ok() {
            return Some(n.to_string());
        }
    }
    for n in ["main", "master", "trunk", "develop"] {
        if repo.find_branch(&format!("origin/{n}"), BranchType::Remote).is_ok() {
            return Some(format!("origin/{n}"));
        }
    }
    super::head_branch(repo)
}

/// Is everything on `tip` already in `base`? "merged" when `tip` is an
/// ancestor; "squashed" when merging it into `base` would change nothing
/// (its changes arrived another way: squash merge, rebase, cherry-picks).
fn merge_state(repo: &Repository, base: Oid, tip: Oid) -> Result<&'static str> {
    if base == tip || repo.graph_descendant_of(base, tip)? {
        return Ok("merged");
    }
    let Ok(mb) = repo.merge_base(base, tip) else { return Ok("unmerged") };
    let ancestor = repo.find_commit(mb)?.tree()?;
    let ours = repo.find_commit(base)?.tree()?;
    let theirs = repo.find_commit(tip)?.tree()?;
    let mut index = repo.merge_trees(&ancestor, &ours, &theirs, None)?;
    if index.has_conflicts() {
        return Ok("unmerged");
    }
    let result = index.write_tree_to(repo)?;
    Ok(if result == ours.id() { "squashed" } else { "unmerged" })
}

pub fn branches_review(repo_path: &str, base: Option<&str>) -> Result<Review> {
    let repo = open(repo_path)?;
    let base_name = match base {
        Some(b) if !b.is_empty() => b.to_string(),
        _ => default_base(&repo).ok_or_else(|| Error::Msg("This repository has no branches yet.".into()))?,
    };
    let base_oid = super::branches::resolve(&repo, &base_name)?;
    let mut rows = vec![];
    for b in repo.branches(Some(BranchType::Local))? {
        let (b, _) = b?;
        let Some(name) = b.name().ok().flatten().map(|s| s.to_string()) else { continue };
        let Some(tip) = b.get().target() else { continue };
        let c = repo.find_commit(tip)?;
        let (ahead, behind) = repo.graph_ahead_behind(tip, base_oid)?;
        let upstream_name = repo
            .branch_upstream_name(&format!("refs/heads/{name}"))
            .ok()
            .and_then(|s| s.as_str().ok().map(|s| s.trim_start_matches("refs/remotes/").to_string()));
        let up = b.upstream().ok().and_then(|u| u.get().target());
        let (to_push, to_pull) = match up {
            Some(u) => repo.graph_ahead_behind(tip, u)?,
            None => (0, 0),
        };
        rows.push(ReviewRow {
            is_head: b.is_head(),
            is_base: name == base_name,
            merged: merge_state(&repo, base_oid, tip)?,
            ahead,
            behind,
            upstream_gone: upstream_name.is_some() && up.is_none(),
            upstream: upstream_name,
            to_push,
            to_pull,
            time: c.time().seconds(),
            author: c.author().name().unwrap_or("").to_string(),
            summary: summary(&c),
            name,
        });
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.time));
    Ok(Review { base: base_name, rows })
}

// ----------------------------------------------------------------- compare

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CompareCommit {
    pub oid: String,
    pub short: String,
    pub summary: String,
    pub author: String,
    pub time: i64,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub base: String,
    pub target: String,
    pub merge_base: Option<String>,
    /// Commits in `target` that `base` doesn't have (newest first), and the reverse.
    pub ahead: Vec<CompareCommit>,
    pub behind: Vec<CompareCommit>,
    /// More commits than were listed.
    pub ahead_total: usize,
    pub behind_total: usize,
    /// What `target` would bring to `base`: the changes since their merge
    /// base (like `git diff base...target`).
    pub files: Vec<super::diff::FileDiff>,
}

fn only_in(repo: &Repository, include: Oid, exclude: Oid, limit: usize) -> Result<(Vec<CompareCommit>, usize)> {
    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
    walk.push(include)?;
    walk.hide(exclude)?;
    let mut out = vec![];
    let mut total = 0;
    for id in walk {
        let id = id?;
        total += 1;
        if out.len() < limit {
            let c = repo.find_commit(id)?;
            out.push(CompareCommit {
                oid: id.to_string(),
                short: short(id),
                summary: summary(&c),
                author: c.author().name().unwrap_or("").to_string(),
                time: c.time().seconds(),
            });
        }
    }
    Ok((out, total))
}

/// The diff a comparison shows: merge base → target, or base → target when
/// they share no history.
fn compare_diff<'a>(repo: &'a Repository, base: Oid, target: Oid, s: super::diff::DiffSettings) -> Result<git2::Diff<'a>> {
    let from = repo.merge_base(base, target).unwrap_or(base);
    let old = repo.find_commit(from)?.tree()?;
    let new = repo.find_commit(target)?.tree()?;
    let mut o = super::diff::options(s);
    let mut d = repo.diff_tree_to_tree(Some(&old), Some(&new), Some(&mut o))?;
    let mut f = git2::DiffFindOptions::new();
    f.renames(true);
    d.find_similar(Some(&mut f))?;
    Ok(d)
}

pub fn compare(repo_path: &str, base: &str, target: &str) -> Result<Comparison> {
    let repo = open(repo_path)?;
    let b = super::branches::resolve(&repo, base)?;
    let t = super::branches::resolve(&repo, target)?;
    if b == t {
        return err(format!("{base} and {target} point at the same commit."));
    }
    let (ahead, ahead_total) = only_in(&repo, t, b, 500)?;
    let (behind, behind_total) = only_in(&repo, b, t, 500)?;
    let diff = compare_diff(&repo, b, t, Default::default())?;
    let mut files = vec![];
    for i in 0..diff.deltas().len() {
        files.push(super::diff::file_diff(&diff, i, false, None)?);
    }
    Ok(Comparison {
        base: base.into(),
        target: target.into(),
        merge_base: repo.merge_base(b, t).ok().map(|o| o.to_string()),
        ahead,
        behind,
        ahead_total,
        behind_total,
        files,
    })
}

/// One file's diff within a comparison.
pub fn compare_file(repo_path: &str, base: &str, target: &str, path: &str, s: super::diff::DiffSettings) -> Result<Option<super::diff::FileDiff>> {
    let repo = open(repo_path)?;
    let b = super::branches::resolve(&repo, base)?;
    let t = super::branches::resolve(&repo, target)?;
    let diff = compare_diff(&repo, b, t, s)?;
    for i in 0..diff.deltas().len() {
        let d = diff.get_delta(i).unwrap();
        let p = d.new_file().path().or(d.old_file().path()).map(|p| p.to_string_lossy().to_string());
        if p.as_deref() == Some(path) {
            return Ok(Some(super::diff::file_diff(&diff, i, true, s.max_lines)?));
        }
    }
    Ok(None)
}
