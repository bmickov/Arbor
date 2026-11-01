//! Bisect: find the commit that introduced a bug by testing commits half
//! way between one known to be good and one known to be bad.
//!
//! State is kept the way git keeps it (refs/bisect/bad, refs/bisect/good-*,
//! refs/bisect/skip-*, BISECT_START, BISECT_LOG), so `git bisect` on the
//! command line sees the same session.

use super::{err, head_branch, head_oid, open, require_tracked_clean, short, Result};
use git2::{Oid, Repository, Sort};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BisectState {
    pub bad: Option<String>,
    pub good: Vec<String>,
    pub skipped: Vec<String>,
    /// The commit checked out to test.
    pub current: Option<String>,
    /// Commits that could still be the first bad one.
    pub remaining: usize,
    /// Roughly how many more tests it takes.
    pub steps: usize,
    /// The first bad commit, once found.
    pub found: Option<String>,
    /// Only skipped commits are left: one of these is the first bad one.
    pub ambiguous: Vec<String>,
    /// Where to go back to when bisect ends.
    pub original: String,
}

fn ids_with_prefix(repo: &Repository, prefix: &str) -> Vec<Oid> {
    repo.references_glob(&format!("refs/bisect/{prefix}*"))
        .map(|refs| refs.flatten().filter_map(|r| r.target()).collect())
        .unwrap_or_default()
}

fn bad(repo: &Repository) -> Option<Oid> {
    repo.find_reference("refs/bisect/bad").ok().and_then(|r| r.target())
}

fn active(repo: &Repository) -> bool {
    repo.path().join("BISECT_START").exists()
}

fn log(repo: &Repository, line: &str) -> Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(repo.path().join("BISECT_LOG"))?;
    writeln!(f, "{line}")?;
    Ok(())
}

/// Commits reachable from bad but not from any good one, newest first.
fn candidates(repo: &Repository) -> Result<Vec<Oid>> {
    let Some(b) = bad(repo) else { return Ok(vec![]) };
    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TOPOLOGICAL)?;
    walk.push(b)?;
    for g in ids_with_prefix(repo, "good-") {
        walk.hide(g)?;
    }
    Ok(walk.collect::<std::result::Result<_, _>>()?)
}

/// The candidate that splits the rest most evenly (like git): the one whose
/// ancestors within the candidates are closest to half of them.
fn midpoint(repo: &Repository, cands: &[Oid], skipped: &HashSet<Oid>) -> Result<Option<Oid>> {
    let n = cands.len();
    let pick: Vec<Oid> = cands.iter().copied().filter(|c| !skipped.contains(c) && Some(*c) != bad(repo)).collect();
    if pick.is_empty() {
        return Ok(None);
    }
    if n > 2000 {
        // Big ranges: the commit half way down the (topological) list is close enough.
        let target = cands[n / 2];
        return Ok(Some(pick.iter().copied().min_by_key(|c| {
            let i = cands.iter().position(|x| x == c).unwrap_or(0) as i64;
            let t = cands.iter().position(|x| *x == target).unwrap_or(0) as i64;
            (i - t).abs()
        }).unwrap()));
    }
    let set: HashSet<Oid> = cands.iter().copied().collect();
    let mut parents: HashMap<Oid, Vec<Oid>> = HashMap::new();
    for c in cands {
        let commit = repo.find_commit(*c)?;
        parents.insert(*c, commit.parent_ids().filter(|p| set.contains(p)).collect());
    }
    let mut best = None;
    let mut best_score = usize::MAX;
    for c in &pick {
        // How many candidates are this commit or its ancestors.
        let mut seen = HashSet::new();
        let mut stack = vec![*c];
        while let Some(x) = stack.pop() {
            if seen.insert(x) {
                stack.extend(parents.get(&x).into_iter().flatten().copied());
            }
        }
        let score = (n as i64 - 2 * seen.len() as i64).unsigned_abs() as usize;
        if score < best_score {
            best_score = score;
            best = Some(*c);
        }
    }
    Ok(best)
}

pub fn state(repo_path: &str) -> Result<Option<BisectState>> {
    let repo = open(repo_path)?;
    state_of(&repo)
}

pub fn state_of(repo: &Repository) -> Result<Option<BisectState>> {
    if !active(repo) {
        return Ok(None);
    }
    let original = std::fs::read_to_string(repo.path().join("BISECT_START")).unwrap_or_default().trim().to_string();
    let goods = ids_with_prefix(repo, "good-");
    let skips: HashSet<Oid> = ids_with_prefix(repo, "skip-").into_iter().collect();
    let cands = if bad(repo).is_some() && !goods.is_empty() { candidates(repo)? } else { vec![] };
    let unskipped: Vec<Oid> = cands.iter().copied().filter(|c| !skips.contains(c)).collect();
    let found = (unskipped.len() == 1 && Some(unskipped[0]) == bad(repo)).then(|| unskipped[0].to_string());
    let ambiguous = if found.is_none() && !cands.is_empty() && unskipped.iter().all(|c| Some(*c) == bad(repo)) {
        cands.iter().map(|c| c.to_string()).collect()
    } else {
        vec![]
    };
    let remaining = unskipped.len();
    Ok(Some(BisectState {
        bad: bad(repo).map(|b| b.to_string()),
        good: goods.iter().map(|g| g.to_string()).collect(),
        skipped: skips.iter().map(|s| s.to_string()).collect(),
        current: head_oid(repo).map(|h| h.to_string()),
        remaining,
        steps: if remaining <= 1 { 0 } else { (usize::BITS - (remaining - 1).leading_zeros()) as usize },
        found,
        ambiguous,
        original,
    }))
}

fn checkout_detached(repo: &Repository, id: Oid) -> Result<()> {
    let c = repo.find_commit(id)?;
    let mut co = git2::build::CheckoutBuilder::new();
    co.safe();
    repo.checkout_tree(c.as_object(), Some(&mut co))?;
    repo.set_head_detached(id)?;
    Ok(())
}

/// Check out the next commit to test (nothing when the culprit is found).
fn advance(repo: &Repository) -> Result<()> {
    let skips: HashSet<Oid> = ids_with_prefix(repo, "skip-").into_iter().collect();
    let cands = candidates(repo)?;
    if let Some(next) = midpoint(repo, &cands, &skips)? {
        std::fs::write(repo.path().join("BISECT_EXPECTED_REV"), format!("{next}\n"))?;
        checkout_detached(repo, next)?;
    } else if let Some(b) = bad(repo) {
        // Found (or only skipped commits left): show the first bad commit.
        if head_oid(repo) != Some(b) {
            checkout_detached(repo, b)?;
        }
    }
    Ok(())
}

/// Start: `bad` has the bug (default HEAD), `good` doesn't.
pub fn start(repo_path: &str, bad_rev: Option<&str>, good_rev: &str) -> Result<BisectState> {
    let repo = open(repo_path)?;
    if active(&repo) {
        return err("A bisect is already running. Stop it first.");
    }
    if repo.state() != git2::RepositoryState::Clean {
        return err("Finish or abort the operation in progress first.");
    }
    super::rebase::require_not_paused(&repo)?;
    require_tracked_clean(&repo, "bisect")?;
    let b = repo.revparse_single(bad_rev.unwrap_or("HEAD"))?.peel_to_commit()?.id();
    let g = repo.revparse_single(good_rev)?.peel_to_commit()?.id();
    if b == g || !repo.graph_descendant_of(b, g)? {
        return err(format!("The good commit ({}) has to be an earlier commit in the bad one's history.", short(g)));
    }
    let original = head_branch(&repo).unwrap_or_else(|| head_oid(&repo).map(|h| h.to_string()).unwrap_or_default());
    std::fs::write(repo.path().join("BISECT_START"), format!("{original}\n"))?;
    std::fs::write(repo.path().join("BISECT_TERMS"), "bad\ngood\n")?;
    repo.reference("refs/bisect/bad", b, true, "bisect start")?;
    repo.reference(&format!("refs/bisect/good-{g}"), g, true, "bisect start")?;
    log(&repo, &format!("git bisect start '{b}' '{g}'"))?;
    advance(&repo)?;
    Ok(state_of(&repo)?.unwrap())
}

/// Mark the commit being tested (or `id`) good, bad or skipped, and move on.
pub fn mark(repo_path: &str, verdict: &str, id: Option<&str>) -> Result<BisectState> {
    let repo = open(repo_path)?;
    if !active(&repo) {
        return err("No bisect is running.");
    }
    require_tracked_clean(&repo, "test the next commit")?;
    let c = match id {
        Some(i) => repo.revparse_single(i)?.peel_to_commit()?.id(),
        None => head_oid(&repo).ok_or_else(|| super::Error::Msg("Nothing is checked out.".into()))?,
    };
    match verdict {
        "bad" => {
            repo.reference("refs/bisect/bad", c, true, "bisect bad")?;
        }
        "good" => {
            repo.reference(&format!("refs/bisect/good-{c}"), c, true, "bisect good")?;
        }
        "skip" => {
            repo.reference(&format!("refs/bisect/skip-{c}"), c, true, "bisect skip")?;
        }
        other => return err(format!("Unknown verdict '{other}'.")),
    }
    log(&repo, &format!("git bisect {verdict} {c}"))?;
    advance(&repo)?;
    Ok(state_of(&repo)?.unwrap())
}

/// End the bisect and go back to where it started.
pub fn reset(repo_path: &str) -> Result<()> {
    let repo = open(repo_path)?;
    if !active(&repo) {
        return Ok(());
    }
    let original = std::fs::read_to_string(repo.path().join("BISECT_START")).unwrap_or_default().trim().to_string();
    let mut co = git2::build::CheckoutBuilder::new();
    co.safe();
    if repo.find_branch(&original, git2::BranchType::Local).is_ok() {
        let target = repo.revparse_single(&format!("refs/heads/{original}"))?;
        repo.checkout_tree(&target, Some(&mut co))?;
        repo.set_head(&format!("refs/heads/{original}"))?;
    } else if let Ok(id) = Oid::from_str(&original) {
        checkout_detached(&repo, id)?;
    }
    let refs: Vec<String> = repo.references_glob("refs/bisect/*")?.flatten().filter_map(|r| r.name().ok().map(str::to_string)).collect();
    for name in refs {
        repo.find_reference(&name)?.delete()?;
    }
    for f in ["BISECT_START", "BISECT_LOG", "BISECT_TERMS", "BISECT_EXPECTED_REV", "BISECT_ANCESTORS_OK", "BISECT_NAMES"] {
        let _ = std::fs::remove_file(repo.path().join(f));
    }
    Ok(())
}
