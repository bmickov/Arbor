//! Fetch, pull and push, with SSH agent/key and credential-helper auth.

use super::undo;
use super::{advance_head, err, head_branch, open, Error, Result};
use git2::{AutotagOption, BranchType, CertificateCheckStatus, Config, Cred, CredentialType, FetchOptions, FetchPrune, PushOptions, RemoteCallbacks, Repository};
use serde::{Deserialize, Serialize};
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
            if let Some(pct) = (cur * 100).checked_div(total) {
                p2(format!("Uploading objects {pct}%"));
            }
        });
        cb.sideband_progress(move |data| {
            let line = String::from_utf8_lossy(data);
            if let Some(l) = line.split(['\r', '\n']).rfind(|l| !l.trim().is_empty()) {
                p(l.trim().to_string());
            }
            true
        });
    }
    cb
}

/// A remote that's a folder on this computer and isn't there. libgit2 calls
/// that an "unsupported URL protocol", which sends people looking in the
/// wrong place.
pub fn check_local_url(url: &str, what: &str) -> Result<()> {
    let path = url.strip_prefix("file://").unwrap_or(url);
    let b = path.as_bytes();
    let absolute = path.starts_with('/') || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'));
    if absolute && !std::path::Path::new(path).exists() {
        return err(format!("There's no repository at {path}{what}. Check the URL."));
    }
    Ok(())
}

fn fetch_one(repo: &Repository, name: &str, prune: bool, creds: Option<Creds>, progress: Progress) -> Result<()> {
    let mut remote = repo.find_remote(name)?;
    check_local_url(remote.url().unwrap_or(""), &format!(", where {name} points (Repository settings → Remotes)"))?;
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
        Ok(PullOutcome { result: r.result.into(), conflicts: r.conflicts })
    } else {
        let m = super::merge::merge(repo_path, &up_name, false)?;
        Ok(PullOutcome { result: m.result.into(), conflicts: m.conflicts })
    }
}

/// The remote to push through. libgit2 pushes to a remote's fetch URL even
/// when it has a separate push URL (remote.<name>.pushurl), unlike git, so
/// for those we push through an anonymous remote made from the push URL.
/// The bool says whether that happened (remote-tracking branches then have
/// to be updated by hand).
fn push_remote<'r>(repo: &'r Repository, name: &str) -> Result<(git2::Remote<'r>, bool)> {
    let r = repo.find_remote(name)?;
    let url = r.pushurl().ok().flatten().or(r.url().ok()).unwrap_or("").to_string();
    check_local_url(&url, &format!(", where {name} pushes (Repository settings → Remotes)"))?;
    match r.pushurl().ok().flatten().map(str::to_string) {
        Some(push) if Some(push.as_str()) != r.url().ok() => Ok((repo.remote_anonymous(&push)?, true)),
        _ => Ok((r, false)),
    }
}

/// After pushing through an anonymous remote: move `refs/remotes/<remote>/…`
/// the way a normal push would have.
fn update_tracking(repo: &Repository, remote: &str, specs: &[String]) -> Result<()> {
    for spec in specs {
        let spec = spec.trim_start_matches('+');
        let Some((src, dst)) = spec.split_once(':') else { continue };
        let Some(name) = dst.strip_prefix("refs/heads/") else { continue };
        let tracking = format!("refs/remotes/{remote}/{name}");
        if src.is_empty() {
            if let Ok(mut r) = repo.find_reference(&tracking) {
                r.delete()?;
            }
        } else {
            let target = repo.revparse_single(src)?.peel_to_commit()?.id();
            repo.reference(&tracking, target, true, &format!("update by push to {remote}"))?;
        }
    }
    Ok(())
}

fn push_refspecs(repo: &Repository, remote: &str, specs: &[String], creds: Option<Creds>, progress: Progress) -> Result<()> {
    let (mut r, anonymous) = push_remote(repo, remote)?;
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
    let pushed = r.push(specs, Some(&mut po));
    // libgit2 can't push to a repository with a working tree on this
    // computer; git can (when that repository allows it).
    if let Err(e) = &pushed {
        if e.message().contains("non-bare") {
            return push_with_git(repo, remote, specs);
        }
    }
    pushed.map_err(|e| {
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
    if anonymous {
        update_tracking(repo, remote, specs)?;
    }
    Ok(())
}

/// Push with the git program (for what libgit2 can't do), which also
/// updates the remote-tracking branches.
fn push_with_git(repo: &Repository, remote: &str, specs: &[String]) -> Result<()> {
    let dir = repo.workdir().unwrap_or(repo.path());
    let out = crate::platform::quiet_command("git")
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .arg("push")
        // A remote can't be mistaken for an option, whatever it's called.
        .arg("--")
        .arg(remote)
        .args(specs)
        .output()
        .map_err(|_| Error::Msg("Arbor can't push to a repository that has a working tree (a non-bare repository) on this computer, and git isn't installed to do it instead.".into()))?;
    if out.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&out.stderr);
    let text = text.lines().map(|l| l.trim_start_matches("remote: ").trim_start_matches("error: ").trim()).filter(|l| !l.is_empty() && !l.starts_with("To ") && !l.starts_with("hint:")).collect::<Vec<_>>().join("\n");
    if text.contains("non-fast-forward") || text.contains("fetch first") {
        return err(format!("REJECTED:The remote has commits you don't have. Pull first, or force push. ({text})"));
    }
    if text.contains("checked out") || text.contains("denyCurrentBranch") {
        return err(format!("That branch is checked out in the remote repository, which refuses pushes to it. Push to another branch, or make the remote a bare repository.\n({text})"));
    }
    err(format!("The push failed:\n{text}"))
}

/// Repositories with a push under way: one push at a time per repository
/// (two tabs can still push to different repositories at once).
static PUSHING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Holds a repository's place in PUSHING until dropped, even on an early return.
struct PushGuard(String);
impl PushGuard {
    fn take(repo_path: &str) -> Option<PushGuard> {
        let key = std::fs::canonicalize(repo_path).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| repo_path.to_string());
        let mut busy = PUSHING.lock().unwrap_or_else(|e| e.into_inner());
        if busy.contains(&key) {
            return None;
        }
        busy.push(key.clone());
        Some(PushGuard(key))
    }
}
impl Drop for PushGuard {
    fn drop(&mut self) {
        PUSHING.lock().unwrap_or_else(|e| e.into_inner()).retain(|k| k != &self.0);
    }
}

/// Where `refname` points on the remote right now (None: it doesn't exist there).
fn remote_tip(repo: &Repository, remote: &str, refname: &str, creds: Option<Creds>, progress: Progress) -> Result<Option<git2::Oid>> {
    let (mut r, _) = push_remote(repo, remote)?;
    let cb = callbacks(repo.config()?, creds, progress);
    let conn = r.connect_auth(git2::Direction::Push, Some(cb), None).map_err(map_auth)?;
    let tip = conn.list()?.iter().find(|h| h.name() == refname).map(|h| h.oid());
    Ok(tip)
}

#[allow(clippy::too_many_arguments)]
pub fn push(
    repo_path: &str,
    branch: Option<&str>,
    remote: Option<&str>,
    force: bool,
    lease: bool,
    set_upstream: bool,
    creds: Option<Creds>,
    progress: Progress,
) -> Result<String> {
    let Some(_guard) = PushGuard::take(repo_path) else {
        return err("A push is already running in this repository.");
    };
    (|| {
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
        // Force with lease: only overwrite the remote branch if it's still where
        // we last saw it (our remote-tracking branch). If someone pushed since
        // our last fetch, their commits would be lost, so refuse instead.
        if force && lease {
            let short = dest_ref.trim_start_matches("refs/heads/");
            let expected = repo.find_reference(&format!("refs/remotes/{remote}/{short}")).ok().and_then(|r| r.target());
            let actual = remote_tip(&repo, &remote, &dest_ref, creds.clone(), progress.clone())?;
            if actual.is_some() && actual != expected {
                return err(format!(
                    "LEASE:{remote}/{short} has commits you haven't fetched: someone else pushed since you last fetched. \
                     Force pushing now would delete their work."
                ));
            }
        }
        let spec = format!("{}{refname}:{dest_ref}", if force { "+" } else { "" });
        push_refspecs(&repo, &remote, &[spec], creds, progress)?;
        if set_upstream || upstream_remote.is_none() {
            let mut b = repo.find_branch(&branch, BranchType::Local)?;
            let short_dest = dest_ref.trim_start_matches("refs/heads/");
            let _ = b.set_upstream(Some(&format!("{remote}/{short_dest}")));
        }
        Ok(format!("{remote}/{}", dest_ref.trim_start_matches("refs/heads/")))
    })()
}

pub fn push_tag(repo_path: &str, tag: &str, remote: &str, creds: Option<Creds>, progress: Progress) -> Result<()> {
    let repo = open(repo_path)?;
    if repo.find_reference(&format!("refs/tags/{tag}")).is_err() {
        return err(format!("There's no tag called '{tag}'."));
    }
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
