//! Branches, tags, remotes and checkout.

use super::undo::{self, Action};
use super::{err, head_oid, oid, open, short, Error, Result};
use git2::{BranchType, Oid, Repository};
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LocalBranch {
    pub name: String,
    pub oid: String,
    pub is_head: bool,
    pub upstream: Option<String>,
    /// The upstream branch was deleted on the remote.
    pub upstream_gone: bool,
    pub ahead: usize,
    pub behind: usize,
    pub summary: String,
    pub time: i64,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBranch {
    /// Name without the remote ("feature/x").
    pub name: String,
    /// "origin/feature/x"
    pub full_name: String,
    pub oid: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    pub name: String,
    pub url: String,
    pub branches: Vec<RemoteBranch>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub name: String,
    /// The commit the tag points at.
    pub oid: String,
    pub annotated: bool,
    pub message: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Refs {
    pub local: Vec<LocalBranch>,
    pub remotes: Vec<Remote>,
    pub tags: Vec<Tag>,
}

pub fn refs(repo_path: &str) -> Result<Refs> {
    let repo = open(repo_path)?;
    let mut local = vec![];
    for b in repo.branches(Some(BranchType::Local))? {
        let (b, _) = b?;
        let Some(name) = b.name()?.map(|s| s.to_string()) else { continue };
        let Some(target) = b.get().target() else { continue };
        let configured = repo
            .config()
            .ok()
            .and_then(|c| c.get_string(&format!("branch.{name}.merge")).ok())
            .is_some();
        let (upstream, gone, ahead, behind) = match b.upstream() {
            Ok(u) => {
                let un = u.name().ok().flatten().map(|s| s.to_string());
                let (a, bh) = u.get().target().map(|ut| repo.graph_ahead_behind(target, ut).unwrap_or((0, 0))).unwrap_or((0, 0));
                (un, false, a, bh)
            }
            Err(_) => (None, configured, 0, 0),
        };
        let c = repo.find_commit(target)?;
        local.push(LocalBranch {
            name,
            oid: target.to_string(),
            is_head: b.is_head(),
            upstream,
            upstream_gone: gone,
            ahead,
            behind,
            summary: super::summary(&c),
            time: c.time().seconds(),
        });
    }
    local.sort_by_key(|a| a.name.to_lowercase());

    let mut remotes = vec![];
    for name in repo.remotes()?.iter().flatten().flatten() {
        let url = repo.find_remote(name).ok().and_then(|r| r.url().ok().map(|u| u.to_string())).unwrap_or_default();
        remotes.push(Remote { name: name.to_string(), url, branches: vec![] });
    }
    for b in repo.branches(Some(BranchType::Remote))? {
        let (b, _) = b?;
        let Some(full) = b.name()?.map(|s| s.to_string()) else { continue };
        if full.ends_with("/HEAD") {
            continue;
        }
        let Some(target) = b.get().target() else { continue };
        if let Some(r) = remotes.iter_mut().filter(|r| full.starts_with(&format!("{}/", r.name))).max_by_key(|r| r.name.len()) {
            r.branches.push(RemoteBranch { name: full[r.name.len() + 1..].to_string(), full_name: full.clone(), oid: target.to_string() });
        }
    }
    for r in remotes.iter_mut() {
        r.branches.sort_by_key(|a| a.name.to_lowercase());
    }

    let mut tags = vec![];
    for name in repo.tag_names(None)?.iter().flatten().flatten() {
        let Ok(r) = repo.find_reference(&format!("refs/tags/{name}")) else { continue };
        let Ok(obj) = r.peel(git2::ObjectType::Commit) else { continue };
        let tag_obj = r.target().and_then(|t| repo.find_tag(t).ok());
        tags.push(Tag {
            name: name.to_string(),
            oid: obj.id().to_string(),
            annotated: tag_obj.is_some(),
            message: tag_obj.and_then(|t| t.message().ok().flatten().map(|m| m.trim().to_string())),
        });
    }
    tags.sort_by(|a, b| natural_cmp(&b.name, &a.name));
    Ok(Refs { local, remotes, tags })
}

/// Sort "v1.10" after "v1.9".
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> Vec<(String, u64)> {
        let mut out = vec![];
        let mut text = String::new();
        let mut num = String::new();
        for ch in s.chars() {
            if ch.is_ascii_digit() {
                num.push(ch);
            } else {
                if !num.is_empty() {
                    out.push((std::mem::take(&mut text), num.parse().unwrap_or(0)));
                    num.clear();
                }
                text.push(ch);
            }
        }
        out.push((text, num.parse().unwrap_or(0)));
        out
    }
    parts(a).cmp(&parts(b))
}

fn validate_name(name: &str) -> Result<()> {
    if !git2::Branch::name_is_valid(name).unwrap_or(false) {
        return err(format!("'{name}' isn't a valid branch name."));
    }
    Ok(())
}

/// A name a new (or renamed) local branch can take: valid, unused, and not
/// clashing with the folder-like way git stores branch names ("feature"
/// can't sit beside "feature/a"). `except` is a branch being renamed.
fn check_new_branch(repo: &Repository, name: &str, except: Option<&str>) -> Result<()> {
    validate_name(name)?;
    let mut existing = vec![];
    for b in repo.branches(Some(BranchType::Local))? {
        if let Some(n) = b?.0.name().ok().flatten() {
            if Some(n) != except {
                existing.push(n.to_string());
            }
        }
    }
    if existing.iter().any(|n| n == name) {
        return err(format!("A branch named '{name}' already exists."));
    }
    if let Some(n) = existing.iter().find(|n| n.starts_with(&format!("{name}/"))) {
        return err(format!("There's already a branch '{n}', so '{name}' can't be a branch too: git stores branch names like folders. Try '{name}-2' or '{name}/main'."));
    }
    if let Some(n) = existing.iter().find(|n| name.starts_with(&format!("{n}/"))) {
        return err(format!("'{n}' is already a branch, so '{name}' can't be one too: git stores branch names like folders. Try '{n}-{}'.", &name[n.len() + 1..]));
    }
    Ok(())
}

/// The worktree (other than this one) that has `name` checked out.
fn checked_out_elsewhere(repo: &Repository, name: &str) -> Option<String> {
    let want = format!("refs/heads/{name}");
    let here = repo.workdir().and_then(|p| std::fs::canonicalize(p).ok());
    let on = |r: &Repository| r.find_reference("HEAD").ok().and_then(|h| h.symbolic_target().ok().flatten().map(str::to_string));
    let mut others: Vec<std::path::PathBuf> = vec![];
    // From a linked worktree, the main checkout isn't in the worktree list.
    if repo.is_worktree() {
        if let Some(main) = repo.commondir().parent() {
            others.push(main.to_path_buf());
        }
    }
    for wt in repo.worktrees().ok()?.iter().flatten().flatten() {
        if let Ok(w) = repo.find_worktree(wt) {
            others.push(w.path().to_path_buf());
        }
    }
    for path in others {
        if std::fs::canonicalize(&path).ok() == here {
            continue;
        }
        let Ok(r) = Repository::open(&path) else { continue };
        if on(&r).as_deref() == Some(want.as_str()) {
            return Some(path.display().to_string().trim_end_matches(['/', '\\']).to_string());
        }
    }
    None
}

/// Where HEAD points now: "refs/heads/x", or a commit id when detached.
fn head_target(repo: &Repository) -> String {
    if !repo.head_detached().unwrap_or(false) {
        if let Ok(h) = repo.find_reference("HEAD") {
            if let Ok(Some(t)) = h.symbolic_target() {
                return t.to_string();
            }
        }
    }
    head_oid(repo).map(|o| o.to_string()).unwrap_or_default()
}

/// Check out a full ref name ("refs/heads/x") or a commit id.
pub fn checkout_target(repo: &Repository, target: &str) -> Result<()> {
    let mut co = git2::build::CheckoutBuilder::new();
    co.safe();
    if target.starts_with("refs/") {
        let r = repo.find_reference(target)?;
        let commit = r.peel_to_commit()?;
        repo.checkout_tree(commit.as_object(), Some(&mut co)).map_err(|e| checkout_err(repo, e))?;
        repo.set_head(target)?;
    } else {
        let commit = repo.find_commit(oid(target)?)?;
        repo.checkout_tree(commit.as_object(), Some(&mut co)).map_err(|e| checkout_err(repo, e))?;
        repo.set_head_detached(commit.id())?;
    }
    Ok(())
}

fn checkout_err(repo: &Repository, e: git2::Error) -> Error {
    use git2::RepositoryState as S;
    let operation = match repo.state() {
        S::Merge => Some("A merge"),
        S::Revert | S::RevertSequence => Some("A revert"),
        S::CherryPick | S::CherryPickSequence => Some("A cherry-pick"),
        S::Rebase | S::RebaseInteractive | S::RebaseMerge => Some("A rebase"),
        S::ApplyMailbox | S::ApplyMailboxOrRebase => Some("Applying patches"),
        S::Bisect | S::Clean => None,
    };
    let conflicted = repo.index().map(|i| i.has_conflicts()).unwrap_or(false);
    if let (Some(what), true) = (operation, conflicted || e.code() == git2::ErrorCode::Conflict) {
        return Error::Msg(format!("{what} is in progress. Finish it (or abort it) before switching to another branch or commit."));
    }
    if conflicted {
        return Error::Msg("Some files still have conflicts. Resolve them, or discard those changes, before switching.".into());
    }
    if e.code() == git2::ErrorCode::Conflict {
        Error::Msg(format!(
            "Your local changes would be overwritten. Commit or stash them first.\n({})",
            e.message()
        ))
    } else {
        e.into()
    }
}

/// Switching away in the middle of Arbor's own rebase would strand it.
fn ensure_no_paused_rebase(repo: &Repository) -> Result<()> {
    if super::rebase::paused(repo).is_some() {
        return err("A rebase is paused. Continue or abort it before switching to another branch or commit.");
    }
    Ok(())
}

fn checkout_recorded(repo: &Repository, target: &str, label: String) -> Result<()> {
    ensure_no_paused_rebase(repo)?;
    let previous = head_target(repo);
    checkout_target(repo, target)?;
    if let Some(now) = head_oid(repo) {
        undo::record(repo, label, Action::Checkout { previous, expect: now.to_string() });
    }
    Ok(())
}

pub fn checkout_branch(repo_path: &str, name: &str) -> Result<()> {
    let repo = open(repo_path)?;
    if repo.find_branch(name, BranchType::Local).is_err() {
        return err(format!("There's no branch called '{name}'."));
    }
    if let Some(at) = checked_out_elsewhere(&repo, name) {
        return err(format!("'{name}' is already checked out in the worktree at {at}. Open that worktree instead, or switch it to another branch first."));
    }
    checkout_recorded(&repo, &format!("refs/heads/{name}"), format!("Check out {name}"))
}

pub fn checkout_commit(repo_path: &str, id: &str) -> Result<()> {
    let repo = open(repo_path)?;
    let o = oid(id)?;
    checkout_recorded(&repo, &o.to_string(), format!("Check out {}", short(o)))
}

/// Check out a remote branch, creating a local tracking branch when needed.
pub fn checkout_remote(repo_path: &str, full_name: &str, local_name: Option<&str>) -> Result<String> {
    let repo = open(repo_path)?;
    ensure_no_paused_rebase(&repo)?;
    let rb = repo.find_branch(full_name, BranchType::Remote)?;
    let target = rb.get().target().ok_or_else(|| Error::Msg("Remote branch has no commit.".into()))?;
    let default_name = full_name.split_once('/').map(|(_, n)| n).unwrap_or(full_name);
    let name = local_name.unwrap_or(default_name).to_string();
    validate_name(&name)?;
    if repo.find_branch(&name, BranchType::Local).is_err() {
        let c = repo.find_commit(target)?;
        let mut b = repo.branch(&name, &c, false)?;
        b.set_upstream(Some(full_name))?;
    }
    checkout_recorded(&repo, &format!("refs/heads/{name}"), format!("Check out {name}"))?;
    Ok(name)
}

pub fn create_branch(repo_path: &str, name: &str, from: Option<&str>, checkout: bool) -> Result<()> {
    let repo = open(repo_path)?;
    check_new_branch(&repo, name, None)?;
    let start = match from {
        Some(f) => repo.revparse_single(f)?.peel_to_commit()?,
        None => repo
            .head()
            .map_err(|_| Error::Msg("Make a first commit before creating branches.".into()))?
            .peel_to_commit()?,
    };
    repo.branch(name, &start, false)?;
    undo::record(&repo, format!("Create branch {name}"), Action::DeleteBranch { name: name.into(), expect: start.id().to_string() });
    if checkout {
        checkout_recorded(&repo, &format!("refs/heads/{name}"), format!("Check out {name}"))?;
    }
    Ok(())
}

/// True when deleting the branch loses nothing: its tip is in HEAD or in its upstream.
fn merged_or_pushed(repo: &Repository, b: &git2::Branch, tip: Oid) -> bool {
    let merged = match head_oid(repo) {
        Some(h) => h == tip || repo.graph_descendant_of(h, tip).unwrap_or(false),
        None => false,
    };
    let pushed = b
        .upstream()
        .ok()
        .and_then(|u| u.get().target())
        .map(|u| u == tip || repo.graph_descendant_of(u, tip).unwrap_or(false))
        .unwrap_or(false);
    merged || pushed
}

pub fn delete_branch(repo_path: &str, name: &str, force: bool) -> Result<()> {
    let repo = open(repo_path)?;
    let mut b = repo.find_branch(name, BranchType::Local)?;
    if b.is_head() {
        return err(format!("You can't delete '{name}' while it's checked out."));
    }
    if let Some(at) = checked_out_elsewhere(&repo, name) {
        return err(format!("'{name}' is checked out in the worktree at {at}. Switch that worktree to another branch, or remove it, first."));
    }
    let tip = b.get().target().unwrap();
    if !force && !merged_or_pushed(&repo, &b, tip) {
        return err(format!("NOT_MERGED:'{name}' has commits that aren't merged into the current branch."));
    }
    let upstream = b.upstream().ok().and_then(|u| u.name().ok().flatten().map(|s| s.to_string()));
    b.delete()?;
    undo::record(&repo, format!("Delete branch {name}"), Action::RecreateBranch { name: name.into(), oid: tip.to_string(), upstream });
    Ok(())
}

/// Which of `names` have commits that aren't in HEAD or in their upstream,
/// i.e. would lose work if deleted. Unknown names are skipped.
pub fn unmerged(repo_path: &str, names: &[String]) -> Result<Vec<String>> {
    let repo = open(repo_path)?;
    let mut out = vec![];
    for name in names {
        if let Ok(b) = repo.find_branch(name, BranchType::Local) {
            if let Some(tip) = b.get().target() {
                if !merged_or_pushed(&repo, &b, tip) {
                    out.push(name.clone());
                }
            }
        }
    }
    Ok(out)
}

/// Delete several local branches as one undoable step. Everything is checked
/// before anything is deleted: the checked-out branch is refused, and unless
/// `force` is set, so are unmerged branches (the error names them all, after
/// "NOT_MERGED:" and a newline-separated list).
pub fn delete_branches(repo_path: &str, names: &[String], force: bool) -> Result<usize> {
    let repo = open(repo_path)?;
    let mut found = Vec::with_capacity(names.len());
    let mut unmerged = vec![];
    for name in names {
        let b = repo.find_branch(name, BranchType::Local)?;
        if b.is_head() {
            return err(format!("You can't delete '{name}' while it's checked out."));
        }
        if let Some(at) = checked_out_elsewhere(&repo, name) {
            return err(format!("'{name}' is checked out in the worktree at {at}. Switch that worktree to another branch, or remove it, first."));
        }
        let tip = b.get().target().unwrap();
        if !force && !merged_or_pushed(&repo, &b, tip) {
            unmerged.push(name.clone());
        }
        found.push((name.clone(), b, tip));
    }
    if !unmerged.is_empty() {
        return err(format!("NOT_MERGED:{}", unmerged.join("\n")));
    }
    let mut actions = vec![];
    for (name, mut b, tip) in found {
        let upstream = b.upstream().ok().and_then(|u| u.name().ok().flatten().map(|s| s.to_string()));
        b.delete()?;
        actions.push(Action::RecreateBranch { name, oid: tip.to_string(), upstream });
    }
    let n = actions.len();
    if n == 1 {
        undo::record(&repo, format!("Delete branch {}", names[0]), actions.pop().unwrap());
    } else if n > 1 {
        undo::record(&repo, format!("Delete {n} branches"), Action::Batch { actions });
    }
    Ok(n)
}

pub fn rename_branch(repo_path: &str, from: &str, to: &str) -> Result<()> {
    let repo = open(repo_path)?;
    if from != to {
        check_new_branch(&repo, to, Some(from))?;
    }
    let mut b = repo.find_branch(from, BranchType::Local)?;
    b.rename(to, false)?;
    undo::record(&repo, format!("Rename {from} to {to}"), Action::RenameBranch { from: from.into(), to: to.into() });
    Ok(())
}

pub fn set_upstream(repo_path: &str, branch: &str, upstream: Option<&str>) -> Result<()> {
    let repo = open(repo_path)?;
    let mut b = repo.find_branch(branch, BranchType::Local)?;
    let previous = b.upstream().ok().and_then(|u| u.name().ok().flatten().map(|s| s.to_string()));
    b.set_upstream(upstream)?;
    undo::record(&repo, format!("Change upstream of {branch}"), Action::SetUpstream { branch: branch.into(), previous });
    Ok(())
}

pub fn create_tag(repo_path: &str, name: &str, target: &str, message: Option<&str>) -> Result<()> {
    let repo = open(repo_path)?;
    if !git2::Reference::is_valid_name(&format!("refs/tags/{name}")) || name.trim() != name || name.is_empty() {
        return err(format!("'{name}' isn't a valid tag name."));
    }
    if repo.find_reference(&format!("refs/tags/{name}")).is_ok() {
        return err(format!("A tag named '{name}' already exists."));
    }
    let obj = repo.revparse_single(target).map_err(|_| Error::Msg(format!("Couldn't find '{target}' to tag.")))?;
    match message.filter(|m| !m.trim().is_empty()) {
        Some(m) => {
            let sig = super::signature(&repo)?;
            repo.tag(name, &obj, &sig, m, false)?;
        }
        None => {
            repo.tag_lightweight(name, &obj, false)?;
        }
    }
    undo::record(&repo, format!("Create tag {name}"), Action::DeleteTag { name: name.into() });
    Ok(())
}

pub fn delete_tag(repo_path: &str, name: &str) -> Result<()> {
    let repo = open(repo_path)?;
    let r = repo.find_reference(&format!("refs/tags/{name}"))?;
    let target = r.target().ok_or_else(|| Error::Msg("Tag has no target.".into()))?;
    repo.tag_delete(name)?;
    undo::record(&repo, format!("Delete tag {name}"), Action::RecreateTag { name: name.into(), target: target.to_string() });
    Ok(())
}

pub fn add_remote(repo_path: &str, name: &str, url: &str) -> Result<()> {
    let repo = open(repo_path)?;
    repo.remote(name, url)?;
    undo::record(&repo, format!("Add remote {name}"), Action::RemoveRemote { name: name.into() });
    Ok(())
}

pub fn remove_remote(repo_path: &str, name: &str) -> Result<()> {
    let repo = open(repo_path)?;
    let url = repo.find_remote(name)?.url().unwrap_or("").to_string();
    repo.remote_delete(name)?;
    undo::record(&repo, format!("Remove remote {name}"), Action::RecreateRemote { name: name.into(), url });
    Ok(())
}

/// Resolve a branch name, remote branch name, tag or commit id to a commit.
pub fn resolve(repo: &Repository, spec: &str) -> Result<Oid> {
    if let Ok(b) = repo.find_branch(spec, BranchType::Local) {
        if let Some(t) = b.get().target() {
            return Ok(t);
        }
    }
    Ok(repo.revparse_single(spec)?.peel_to_commit()?.id())
}
