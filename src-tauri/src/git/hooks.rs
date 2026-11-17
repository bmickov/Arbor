//! Running the repository's commit hooks. libgit2 doesn't run hooks itself,
//! so Arbor runs pre-commit and commit-msg the way `git commit` would.

use super::{Error, Result};
use git2::Repository;
use std::path::PathBuf;

fn hooks_dir(repo: &Repository) -> PathBuf {
    let wd = repo.workdir().map(|p| p.to_path_buf()).unwrap_or_else(|| repo.path().to_path_buf());
    if let Ok(cfg) = repo.config() {
        if let Ok(p) = cfg.get_path("core.hooksPath") {
            return if p.is_absolute() { p } else { wd.join(p) };
        }
    }
    repo.path().join("hooks")
}

fn executable(p: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

fn run(repo: &Repository, name: &str, args: &[&str]) -> Result<()> {
    let hook = hooks_dir(repo).join(name);
    if !executable(&hook) {
        return Ok(());
    }
    let wd = repo.workdir().unwrap();
    let out = crate::platform::hook_command(&hook)
        .map_err(Error::Msg)?
        .args(args)
        .current_dir(wd)
        .env("GIT_DIR", repo.path())
        .env("GIT_INDEX_FILE", repo.path().join("index"))
        .output()?;
    if !out.status.success() {
        let mut msg = String::from_utf8_lossy(&out.stdout).to_string();
        msg.push_str(&String::from_utf8_lossy(&out.stderr));
        return Err(Error::Msg(format!("The {name} hook stopped the commit.\n\n{}", msg.trim())));
    }
    Ok(())
}

pub fn pre_commit(repo: &Repository) -> Result<()> {
    run(repo, "pre-commit", &[])
}

/// Runs commit-msg, which may rewrite the message. Returns the final message.
pub fn commit_msg(repo: &Repository, message: &str) -> Result<String> {
    let hook = hooks_dir(repo).join("commit-msg");
    if !executable(&hook) {
        return Ok(message.to_string());
    }
    let f = repo.path().join("COMMIT_EDITMSG");
    std::fs::write(&f, message)?;
    run(repo, "commit-msg", &[f.to_str().unwrap_or("")])?;
    Ok(std::fs::read_to_string(&f)?)
}

pub fn post_commit(repo: &Repository) {
    let _ = run(repo, "post-commit", &[]);
}
