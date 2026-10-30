//! Who commits: the author name and email Git will use in a repository, and
//! switching it per repository (Tower's "user profiles"). Also reads Git's
//! own commit message template (`commit.template`).

use super::{open, Result};
use git2::ConfigLevel;
use serde::Serialize;

#[derive(Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub name: Option<String>,
    pub email: Option<String>,
    /// Set in this repository's own config (rather than inherited from global).
    pub local: bool,
}

pub fn identity(repo_path: &str) -> Result<Identity> {
    let repo = open(repo_path)?;
    let cfg = repo.config()?.snapshot()?;
    let local = repo
        .config()?
        .open_level(ConfigLevel::Local)
        .map(|l| l.get_string("user.email").is_ok() || l.get_string("user.name").is_ok())
        .unwrap_or(false);
    Ok(Identity { name: cfg.get_string("user.name").ok(), email: cfg.get_string("user.email").ok(), local })
}

/// Use `name`/`email` for commits in this repository, or (None) go back to
/// the global identity by removing the repository's own setting.
pub fn set_identity(repo_path: &str, who: Option<(&str, &str)>) -> Result<()> {
    let repo = open(repo_path)?;
    let mut local = repo.config()?.open_level(ConfigLevel::Local)?;
    match who {
        Some((name, email)) => {
            local.set_str("user.name", name.trim())?;
            local.set_str("user.email", email.trim())?;
        }
        None => {
            for k in ["user.name", "user.email"] {
                // Missing keys are fine: there's nothing to remove.
                let _ = local.remove(k);
            }
        }
    }
    Ok(())
}

/// The contents of Git's `commit.template` file, if one is configured.
pub fn commit_template(repo_path: &str) -> Result<Option<String>> {
    let repo = open(repo_path)?;
    let Ok(path) = repo.config()?.get_path("commit.template") else { return Ok(None) };
    let full = if path.is_absolute() { path } else { repo.workdir().map(|w| w.join(&path)).unwrap_or(path) };
    Ok(std::fs::read_to_string(full).ok().filter(|t| !t.trim().is_empty()))
}
