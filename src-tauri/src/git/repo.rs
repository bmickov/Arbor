//! Opening, creating and cloning repositories, plus the recent-repos list.

use super::{err, Error, Result};
use git2::Repository;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    /// Absolute path to the working tree.
    pub path: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RecentRepo {
    pub path: String,
    pub name: String,
    pub opened: i64,
}

fn info(repo: &Repository) -> Result<RepoInfo> {
    let wd = repo
        .workdir()
        .ok_or_else(|| Error::Msg("Bare repositories aren't supported.".into()))?;
    let path = wd.to_string_lossy().trim_end_matches('/').to_string();
    let name = Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    Ok(RepoInfo { path, name })
}

/// Open the repository containing `path` (searching parent folders).
pub fn open_repo(path: &str) -> Result<RepoInfo> {
    let repo = Repository::discover(path)
        .map_err(|_| Error::Msg(format!("{path} isn't inside a Git repository.")))?;
    let i = info(&repo)?;
    remember(&i);
    Ok(i)
}

pub fn init_repo(path: &str) -> Result<RepoInfo> {
    std::fs::create_dir_all(path)?;
    let mut opts = git2::RepositoryInitOptions::new();
    opts.initial_head("main");
    let repo = Repository::init_opts(path, &opts)?;
    let i = info(&repo)?;
    remember(&i);
    Ok(i)
}

pub fn clone_repo(url: &str, dest: &str, creds: Option<super::remote::Creds>, progress: super::remote::Progress) -> Result<RepoInfo> {
    if Path::new(dest).exists() && std::fs::read_dir(dest)?.next().is_some() {
        return err(format!("{dest} already exists and isn't empty."));
    }
    super::remote::check_local_url(url, "")?;
    let config = git2::Config::open_default()?;
    let mut fo = git2::FetchOptions::new();
    fo.remote_callbacks(super::remote::callbacks(config, creds, progress));
    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fo);
    let repo = builder.clone(url, Path::new(dest)).map_err(super::remote::map_auth)?;
    let i = info(&repo)?;
    remember(&i);
    Ok(i)
}

fn recent_file() -> Option<PathBuf> {
    super::config_dir().map(|d| d.join("recent.json"))
}

pub fn recent() -> Vec<RecentRepo> {
    let Some(f) = recent_file() else { return vec![] };
    let list: Vec<RecentRepo> = std::fs::read_to_string(f)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    list.into_iter().filter(|r| Path::new(&r.path).join(".git").exists()).collect()
}

fn remember(i: &RepoInfo) {
    super::library::touch(&i.path, &i.name);
    let Some(f) = recent_file() else { return };
    let mut list = recent();
    list.retain(|r| r.path != i.path);
    list.insert(0, RecentRepo { path: i.path.clone(), name: i.name.clone(), opened: chrono::Utc::now().timestamp() });
    list.truncate(20);
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(f, serde_json::to_string_pretty(&list).unwrap_or_default());
}

pub fn forget(path: &str) {
    let Some(f) = recent_file() else { return };
    let mut list = recent();
    list.retain(|r| r.path != path);
    let _ = std::fs::write(f, serde_json::to_string_pretty(&list).unwrap_or_default());
}
