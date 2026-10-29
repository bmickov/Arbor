//! Fetch, pull and push, with SSH agent/key and credential-helper auth.

use super::undo;
use super::{advance_head, err, head_branch, open, Error, Result};
use git2::{AutotagOption, BranchType, CertificateCheckStatus, Config, Cred, CredentialType, FetchOptions, FetchPrune, PushOptions, RemoteCallbacks, Repository};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Deserialize, Clone, Debug)]
pub struct Creds {
    pub username: String,
    pub password: String,
}

/// Receives human-readable progress lines ("Receiving objects 40%").
pub type Progress = Option<Arc<dyn Fn(String) + Send + Sync>>;

thread_local! {
    static WANTED_PASSWORD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Turn libgit2 authentication failures into AUTH_REQUIRED, so the UI can ask
/// for a username and password (or token) and retry.
pub fn map_auth(e: git2::Error) -> Error {
    let wanted = WANTED_PASSWORD.with(|w| w.get());
    if wanted && (e.code() == git2::ErrorCode::Auth || e.class() == git2::ErrorClass::Http || e.message().to_lowercase().contains("auth")) {
        return Error::AuthRequired(e.message().to_string());
    }
    if e.code() == git2::ErrorCode::Auth || e.message().contains("authentication") {
        return Error::Msg(format!(
            "Authentication failed: {}\n\nFor SSH remotes, make sure your key is loaded with ssh-add.",
            e.message()
        ));
    }
    e.into()
}

pub fn callbacks(config: Config, creds: Option<Creds>, progress: Progress) -> RemoteCallbacks<'static> {
    WANTED_PASSWORD.with(|w| w.set(false));
    let mut cb = RemoteCallbacks::new();
    let home = dirs::home_dir().unwrap_or_default();
    let keys: Vec<_> = ["id_ed25519", "id_ecdsa", "id_rsa"].iter().map(|k| home.join(".ssh").join(k)).filter(|p| p.exists()).collect();
    let mut tried_agent = false;
    let mut key_idx = 0usize;
    let mut tried_creds = false;
    let mut tried_helper = false;
    let mut attempts = 0;
    cb.credentials(move |url, user_from_url, allowed| {
        attempts += 1;
        if attempts > 10 {
            return Err(git2::Error::from_str("authentication failed"));
        }
        let user = user_from_url.unwrap_or("git");
        if allowed.contains(CredentialType::SSH_KEY) {
            if !tried_agent {
                tried_agent = true;
                if let Ok(c) = Cred::ssh_key_from_agent(user) {
                    return Ok(c);
                }
            }
            while key_idx < keys.len() {
                let k = &keys[key_idx];
                key_idx += 1;
                let pass = creds.as_ref().map(|c| c.password.as_str());
                if let Ok(c) = Cred::ssh_key(user, None, k, pass) {
                    return Ok(c);
                }
            }
        }
        if allowed.contains(CredentialType::USER_PASS_PLAINTEXT) {
            WANTED_PASSWORD.with(|w| w.set(true));
            if let (Some(c), false) = (&creds, tried_creds) {
                tried_creds = true;
                return Cred::userpass_plaintext(&c.username, &c.password);
            }
            if !tried_helper && creds.is_none() {
                tried_helper = true;
                if let Ok(c) = Cred::credential_helper(&config, url, user_from_url) {
                    return Ok(c);
                }
            }
        }
        if allowed.contains(CredentialType::USERNAME) {
            return Cred::username(user);
        }
        if allowed.contains(CredentialType::DEFAULT) && attempts < 3 {
            return Cred::default();
        }
        Err(git2::Error::from_str("authentication failed"))
    });
    cb.certificate_check(|_, _| Ok(CertificateCheckStatus::CertificatePassthrough));
    if let Some(p) = progress {
        let p1 = p.clone();
        let mut last = 0usize;
        cb.transfer_progress(move |s| {
            let total = s.total_objects().max(1);
            let pct = s.received_objects() * 100 / total;
            if pct != last {
                last = pct;
                p1(format!("Receiving objects {pct}%"));
            }
            true
        });
        let p2 = p.clone();
        cb.push_transfer_progress(move |cur, total, _| {
            if total > 0 {
                p2(format!("Uploading objects {}%", cur * 100 / total));
            }
        });
        cb.sideband_progress(move |data| {
            let line = String::from_utf8_lossy(data);
            if let Some(l) = line.split(['\r', '\n']).filter(|l| !l.trim().is_empty()).last() {
                p(l.trim().to_string());
            }
            true
        });
    }
    cb
}

fn fetch_one(repo: &Repository, name: &str, prune: bool, creds: Option<Creds>, progress: Progress) -> Result<()> {
    let mut remote = repo.find_remote(name)?;
    let mut fo = FetchOptions::new();
    fo.remote_callbacks(callbacks(repo.config()?, creds, progress));
    fo.download_tags(AutotagOption::Auto);
    if prune {
        fo.prune(FetchPrune::On);
    }
    let specs: [&str; 0] = [];
    remote.fetch(&specs, Some(&mut fo), None).map_err(map_auth)?;
    Ok(())
}

/// Fetch one remote, or all remotes when `remote` is None.
pub fn fetch(repo_path: &str, remote: Option<&str>, prune: bool, creds: Option<Creds>, progress: Progress) -> Result<()> {
    let repo = open(repo_path)?;
    let names: Vec<String> = match remote {
        Some(r) => vec![r.to_string()],
        None => repo.remotes()?.iter().flatten().flatten().map(|s| s.to_string()).collect(),
    };
    if names.is_empty() {
        return err("This repository has no remotes.");
    }
    for n in names {
        fetch_one(&repo, &n, prune, creds.clone(), progress.clone())?;
    }
    Ok(())
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PullOutcome {
    /// up-to-date | fast-forward | merged | rebased | conflicts
    pub result: String,
    pub conflicts: Vec<String>,
}

pub fn pull(repo_path: &str, rebase: bool, creds: Option<Creds>, progress: Progress) -> Result<PullOutcome> {
    let repo = open(repo_path)?;
    let Some(branch) = head_branch(&repo) else {
        return err("Check out a branch before pulling.");
    };
    let refname = format!("refs/heads/{branch}");
    let remote = repo
        .branch_upstream_remote(&refname)
        .ok()
        .and_then(|b| b.as_str().ok().map(|s| s.to_string()))
        .ok_or_else(|| Error::Msg(format!("'{branch}' doesn't track a remote branch. Push it first, or set its upstream.")))?;
    fetch_one(&repo, &remote, false, creds, progress)?;
    let b = repo.find_branch(&branch, BranchType::Local)?;
    let up = b.upstream().map_err(|_| Error::Msg(format!("The upstream of '{branch}' no longer exists on {remote}.")))?;
    let up_name = up.name()?.unwrap_or("").to_string();
    let up_oid = up.get().target().unwrap();
    let head = repo.head()?.peel_to_commit()?;
    let annotated = repo.find_annotated_commit(up_oid)?;
    let (analysis, _) = repo.merge_analysis(&[&annotated])?;
    if analysis.is_up_to_date() {
        return Ok(PullOutcome { result: "up-to-date".into(), conflicts: vec![] });
    }
    if analysis.is_fast_forward() {
        advance_head(&repo, up_oid, &format!("pull: fast-forward to {up_name}")).map_err(|e| match e {
            Error::Git(g) if g.code() == git2::ErrorCode::Conflict => {
                Error::Msg("Your local changes would be overwritten by the pull. Commit or stash them first.".into())
            }
            other => other,
        })?;
        undo::record_head_move(&repo, format!("Pull {up_name}"), Some(head.id()), "mixed");
        return Ok(PullOutcome { result: "fast-forward".into(), conflicts: vec![] });
    }
    if rebase {
        let r = super::rebase::rebase(repo_path, &up_name)?;
        Ok(PullOutcome { result: r.result.into(), conflicts: vec![] })
    } else {
        let m = super::merge::merge(repo_path, &up_name, false)?;
        Ok(PullOutcome { result: m.result.into(), conflicts: m.conflicts })
    }
}

fn push_refspecs(repo: &Repository, remote: &str, specs: &[String], creds: Option<Creds>, progress: Progress) -> Result<()> {
    let mut r = repo.find_remote(remote)?;
    let rejected: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let rej = rejected.clone();
    let mut cb = callbacks(repo.config()?, creds, progress);
    cb.push_update_reference(move |refname, status| {
        if let Some(s) = status {
            rej.lock().unwrap().push(format!("{refname}: {s}"));
        }
        Ok(())
    });
    let mut po = PushOptions::new();
    po.remote_callbacks(cb);
    r.push(specs, Some(&mut po)).map_err(|e| {
        let m = e.message().to_lowercase();
        if e.code() == git2::ErrorCode::NotFastForward || m.contains("non-fast") || m.contains("fastforward") || m.contains("fast-forward") || m.contains("not present locally") {
            Error::Msg(format!("REJECTED:The remote has commits you don't have. Pull first, or force push. ({})", e.message()))
        } else {
            map_auth(e)
        }
    })?;
    let rej = rejected.lock().unwrap();
    if !rej.is_empty() {
        return err(format!("REJECTED:The remote refused the push:\n{}", rej.join("\n")));
    }
    Ok(())
}

static PUSHING: AtomicBool = AtomicBool::new(false);

pub fn push(
    repo_path: &str,
    branch: Option<&str>,
    remote: Option<&str>,
    force: bool,
    set_upstream: bool,
    creds: Option<Creds>,
    progress: Progress,
) -> Result<String> {
    if PUSHING.swap(true, Ordering::SeqCst) {
        return err("A push is already running.");
    }
    let res = (|| {
        let repo = open(repo_path)?;
        let branch = match branch {
            Some(b) => b.to_string(),
            None => head_branch(&repo).ok_or_else(|| Error::Msg("Check out a branch before pushing.".into()))?,
        };
        let refname = format!("refs/heads/{branch}");
        let upstream_remote = repo.branch_upstream_remote(&refname).ok().and_then(|b| b.as_str().ok().map(|s| s.to_string()));
        let remotes: Vec<String> = repo.remotes()?.iter().flatten().flatten().map(|s| s.to_string()).collect();
        let remote = remote
            .map(|s| s.to_string())
            .or(upstream_remote.clone())
            .or_else(|| remotes.iter().find(|r| *r == "origin").cloned())
            .or_else(|| remotes.first().cloned())
            .ok_or_else(|| Error::Msg("This repository has no remotes. Add one first.".into()))?;
        // Push to the upstream branch's name when it lives on this remote.
        let dest = if upstream_remote.as_deref() == Some(remote.as_str()) {
            repo.branch_upstream_name(&refname).ok().and_then(|b| b.as_str().ok().map(|s| s.to_string()))
        } else {
            None
        };
        let dest_ref = dest
            .and_then(|d| d.strip_prefix(&format!("refs/remotes/{remote}/")).map(|s| format!("refs/heads/{s}")))
            .unwrap_or_else(|| refname.clone());
        let spec = format!("{}{refname}:{dest_ref}", if force { "+" } else { "" });
        push_refspecs(&repo, &remote, &[spec], creds, progress)?;
        if set_upstream || upstream_remote.is_none() {
            let mut b = repo.find_branch(&branch, BranchType::Local)?;
            let short_dest = dest_ref.trim_start_matches("refs/heads/");
            let _ = b.set_upstream(Some(&format!("{remote}/{short_dest}")));
        }
        Ok(format!("{remote}/{}", dest_ref.trim_start_matches("refs/heads/")))
    })();
    PUSHING.store(false, Ordering::SeqCst);
    res
}

pub fn push_tag(repo_path: &str, tag: &str, remote: &str, creds: Option<Creds>, progress: Progress) -> Result<()> {
    let repo = open(repo_path)?;
    push_refspecs(&repo, remote, &[format!("refs/tags/{tag}:refs/tags/{tag}")], creds, progress)
}

pub fn delete_remote_branch(repo_path: &str, full_name: &str, creds: Option<Creds>, progress: Progress) -> Result<()> {
    let repo = open(repo_path)?;
    let (remote, name) = full_name
        .split_once('/')
        .ok_or_else(|| Error::Msg(format!("'{full_name}' isn't a remote branch name.")))?;
    push_refspecs(&repo, remote, &[format!(":refs/heads/{name}")], creds, progress)?;
    // libgit2 doesn't always drop the tracking ref after a delete push.
    if let Ok(mut r) = repo.find_reference(&format!("refs/remotes/{full_name}")) {
        let _ = r.delete();
    }
    Ok(())
}
