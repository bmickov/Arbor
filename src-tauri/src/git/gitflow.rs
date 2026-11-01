//! Git-flow: feature, release and hotfix branches with the git-flow
//! conventions. Settings use git-flow's own config keys (gitflow.branch.*,
//! gitflow.prefix.*), so Arbor and the git-flow command line agree.

use super::branches::{checkout_branch, create_branch, delete_branch};
use super::merge::{merge_into, MergeOutcome};
use super::{err, head_branch, open, require_tracked_clean, Result};
use git2::{BranchType, Repository};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FlowConfig {
    /// The production branch ("main" or "master").
    pub main: String,
    /// The integration branch ("develop").
    pub develop: String,
    pub feature: String,
    pub release: String,
    pub hotfix: String,
    /// Prefix for release and hotfix tags ("v", or "").
    pub version_tag: String,
}

fn get(cfg: &git2::Config, key: &str) -> Option<String> {
    cfg.get_string(key).ok()
}

/// The repository's git-flow setup, or None when it isn't set up.
pub fn config(repo_path: &str) -> Result<Option<FlowConfig>> {
    let repo = open(repo_path)?;
    Ok(read(&repo))
}

fn read(repo: &Repository) -> Option<FlowConfig> {
    let cfg = repo.config().ok()?.snapshot().ok()?;
    let main = get(&cfg, "gitflow.branch.master").or_else(|| get(&cfg, "gitflow.branch.main"))?;
    let develop = get(&cfg, "gitflow.branch.develop")?;
    Some(FlowConfig {
        main,
        develop,
        feature: get(&cfg, "gitflow.prefix.feature").unwrap_or_else(|| "feature/".into()),
        release: get(&cfg, "gitflow.prefix.release").unwrap_or_else(|| "release/".into()),
        hotfix: get(&cfg, "gitflow.prefix.hotfix").unwrap_or_else(|| "hotfix/".into()),
        version_tag: get(&cfg, "gitflow.prefix.versiontag").unwrap_or_default(),
    })
}

/// Suggested settings: the existing production branch, develop, standard prefixes.
pub fn suggest(repo_path: &str) -> Result<FlowConfig> {
    let repo = open(repo_path)?;
    if let Some(c) = read(&repo) {
        return Ok(c);
    }
    let main = ["main", "master"]
        .iter()
        .find(|b| repo.find_branch(b, BranchType::Local).is_ok())
        .map(|s| s.to_string())
        .or_else(|| head_branch(&repo))
        .unwrap_or_else(|| "main".into());
    Ok(FlowConfig {
        main,
        develop: "develop".into(),
        feature: "feature/".into(),
        release: "release/".into(),
        hotfix: "hotfix/".into(),
        version_tag: String::new(),
    })
}

/// Save the settings and create the develop branch from main if it's missing.
pub fn init(repo_path: &str, c: &FlowConfig) -> Result<()> {
    let repo = open(repo_path)?;
    if c.main.trim().is_empty() || c.develop.trim().is_empty() || c.main == c.develop {
        return err("The production and development branches need two different names.");
    }
    if repo.find_branch(&c.main, BranchType::Local).is_err() {
        return err(format!("There's no branch named {}.", c.main));
    }
    let mut cfg = repo.config()?.open_level(git2::ConfigLevel::Local)?;
    cfg.set_str("gitflow.branch.master", &c.main)?;
    cfg.set_str("gitflow.branch.develop", &c.develop)?;
    cfg.set_str("gitflow.prefix.feature", &c.feature)?;
    cfg.set_str("gitflow.prefix.release", &c.release)?;
    cfg.set_str("gitflow.prefix.hotfix", &c.hotfix)?;
    cfg.set_str("gitflow.prefix.support", "support/")?;
    cfg.set_str("gitflow.prefix.bugfix", "bugfix/")?;
    cfg.set_str("gitflow.prefix.versiontag", &c.version_tag)?;
    if repo.find_branch(&c.develop, BranchType::Local).is_err() {
        create_branch(repo_path, &c.develop, Some(&c.main), false)?;
    }
    Ok(())
}

fn need(repo: &Repository) -> Result<FlowConfig> {
    read(repo).ok_or_else(|| super::Error::Msg("Set up git-flow for this repository first.".into()))
}

fn prefix<'a>(c: &'a FlowConfig, kind: &str) -> Result<&'a str> {
    Ok(match kind {
        "feature" => &c.feature,
        "release" => &c.release,
        "hotfix" => &c.hotfix,
        _ => return err(format!("Unknown git-flow branch type '{kind}'.")),
    })
}

/// Start a feature (from develop), release (from develop) or hotfix (from
/// main) branch named prefix+name, and check it out. Returns the branch name.
pub fn start(repo_path: &str, kind: &str, name: &str) -> Result<String> {
    let repo = open(repo_path)?;
    let c = need(&repo)?;
    let name = name.trim();
    if name.is_empty() {
        return err("Give it a name.");
    }
    let branch = format!("{}{}", prefix(&c, kind)?, name);
    let base = if kind == "hotfix" { &c.main } else { &c.develop };
    if kind != "feature" {
        // git-flow allows one release and one hotfix at a time.
        let p = prefix(&c, kind)?;
        if let Some(b) = repo.branches(Some(BranchType::Local))?.flatten().find_map(|(b, _)| b.name().ok().flatten().filter(|n| n.starts_with(p)).map(str::to_string)) {
            return err(format!("Finish {b} before starting another {kind}."));
        }
    }
    require_tracked_clean(&repo, "start a new branch")?;
    create_branch(repo_path, &branch, Some(base), true)?;
    Ok(branch)
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FinishOutcome {
    /// "finished", or "conflicts" when a merge stopped (resolve, commit, then finish again).
    pub result: &'static str,
    pub conflicts: Vec<String>,
    /// Which branch the conflicting merge is into.
    pub merging_into: Option<String>,
    pub tag: Option<String>,
}

fn merged(repo: &Repository, branch: &str, into: &str) -> Result<bool> {
    let b = repo.find_branch(branch, BranchType::Local)?.get().peel_to_commit()?.id();
    let i = repo.find_branch(into, BranchType::Local)?.get().peel_to_commit()?.id();
    Ok(b == i || repo.graph_descendant_of(i, b)?)
}

fn step(repo_path: &str, branch: &str, into: &str) -> Result<Option<MergeOutcome>> {
    let repo = open(repo_path)?;
    if merged(&repo, branch, into)? {
        return Ok(None);
    }
    let m = merge_into(repo_path, branch, into, true)?;
    Ok(Some(m))
}

/// Finish a git-flow branch the way git-flow does:
/// feature → develop; release/hotfix → main (tagged) and develop.
/// Each merge is skipped when already done, so after resolving a conflict
/// (and committing the merge) finishing again carries on where it stopped.
pub fn finish(repo_path: &str, branch: &str, tag_message: Option<&str>, keep_branch: bool) -> Result<FinishOutcome> {
    let repo = open(repo_path)?;
    let c = need(&repo)?;
    if repo.state() != git2::RepositoryState::Clean {
        return err("Commit or abort the merge in progress first.");
    }
    require_tracked_clean(&repo, "finish a branch")?;
    let (kind, name) = if let Some(n) = branch.strip_prefix(&c.feature) {
        ("feature", n)
    } else if let Some(n) = branch.strip_prefix(&c.release) {
        ("release", n)
    } else if let Some(n) = branch.strip_prefix(&c.hotfix) {
        ("hotfix", n)
    } else {
        return err(format!("{branch} isn't a git-flow feature, release or hotfix branch."));
    };
    let conflict = |m: MergeOutcome, into: &str| FinishOutcome {
        result: "conflicts",
        conflicts: m.conflicts,
        merging_into: Some(into.to_string()),
        tag: None,
    };
    let mut tag = None;
    if kind == "feature" {
        if let Some(m) = step(repo_path, branch, &c.develop)? {
            if m.result == "conflicts" {
                return Ok(conflict(m, &c.develop));
            }
        }
    } else {
        if let Some(m) = step(repo_path, branch, &c.main)? {
            if m.result == "conflicts" {
                return Ok(conflict(m, &c.main));
            }
        }
        // Tag the release on main.
        let t = format!("{}{}", c.version_tag, name);
        let repo = open(repo_path)?;
        if repo.find_reference(&format!("refs/tags/{t}")).is_err() {
            let msg = tag_message.filter(|m| !m.trim().is_empty()).map(str::to_string).unwrap_or_else(|| format!("{} {name}", if kind == "release" { "Release" } else { "Hotfix" }));
            super::branches::create_tag(repo_path, &t, &c.main, Some(&msg))?;
        }
        tag = Some(t.clone());
        // Then bring main (with the tag) back into develop.
        if let Some(m) = step(repo_path, &c.main, &c.develop)? {
            if m.result == "conflicts" {
                return Ok(FinishOutcome { tag, ..conflict(m, &c.develop) });
            }
        }
    }
    // End up on develop, like git-flow.
    let land = &c.develop;
    if head_branch(&open(repo_path)?).as_deref() != Some(land.as_str()) {
        checkout_branch(repo_path, land)?;
    }
    if !keep_branch {
        delete_branch(repo_path, branch, false)?;
    }
    Ok(FinishOutcome { result: "finished", conflicts: vec![], merging_into: None, tag })
}
