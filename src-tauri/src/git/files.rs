//! Working-copy file actions (ignore, stop tracking, restore an older
//! version) and patch files (save a commit or uncommitted changes as a
//! .patch, apply one). Everything here can be undone.

use super::undo::{self, Action};
use super::{err, open, Error, Result};
use git2::{ApplyLocation, Diff, DiffFormat, DiffOptions, EmailCreateOptions, Repository};
use serde::Serialize;
use std::path::Path;

fn workdir(repo: &Repository) -> Result<&Path> {
    repo.workdir().ok_or_else(|| Error::Msg("Bare repositories aren't supported.".into()))
}

/// Add `pattern` to the repository's top-level .gitignore, creating it if
/// needed. Returns false when the pattern was already there.
pub fn ignore(repo_path: &str, pattern: &str) -> Result<bool> {
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern.contains('\n') {
        return err("That isn't a valid ignore pattern.");
    }
    let repo = open(repo_path)?;
    let file = workdir(&repo)?.join(".gitignore");
    let current = std::fs::read_to_string(&file).unwrap_or_default();
    if current.lines().any(|l| l.trim() == pattern) {
        return Ok(false);
    }
    let saved = undo::save_files(&repo, &[".gitignore".to_string()])?;
    let mut text = current;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(pattern);
    text.push('\n');
    std::fs::write(&file, text)?;
    undo::record(&repo, format!("Ignore {pattern}"), Action::RestoreFiles { files: saved });
    Ok(true)
}

/// Stop tracking files (like `git rm --cached`): they leave the index but stay on disk.
pub fn untrack(repo_path: &str, paths: &[String]) -> Result<()> {
    let repo = open(repo_path)?;
    let before = undo::index_tree(&repo);
    let mut index = repo.index()?;
    for p in paths {
        index.remove_path(Path::new(p)).map_err(|_| Error::Msg(format!("{p} isn't tracked.")))?;
    }
    index.write()?;
    if let Some(tree) = before {
        let label = if paths.len() == 1 { format!("Stop tracking {}", paths[0]) } else { format!("Stop tracking {} files", paths.len()) };
        undo::record(&repo, label, Action::RestoreIndex { tree });
    }
    Ok(())
}

/// Put `path` in the working copy back to how it was at `rev` (a commit, or
/// e.g. "<commit>^" for the version before it). If the file didn't exist
/// then, it's removed. The result shows up as an ordinary unstaged change.
pub fn restore_file(repo_path: &str, rev: &str, path: &str) -> Result<()> {
    let repo = open(repo_path)?;
    let commit = repo
        .revparse_single(rev)
        .and_then(|o| o.peel_to_commit())
        .map_err(|_| Error::Msg(format!("Couldn't find the commit {rev}.")))?;
    let entry = commit.tree()?.get_path(Path::new(path)).ok();
    let saved = undo::save_files(&repo, &[path.to_string()])?;
    let full = workdir(&repo)?.join(path);
    match entry {
        Some(e) => {
            let blob = repo.find_blob(e.id())?;
            if let Some(dir) = full.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&full, blob.content())?;
        }
        None => {
            if full.is_file() {
                std::fs::remove_file(&full)?;
            }
        }
    }
    let short = &commit.id().to_string()[..7];
    undo::record(&repo, format!("Restore {path} from {short}"), Action::RestoreFiles { files: saved });
    Ok(())
}

/// Save a commit as an email-style patch (what `git format-patch` writes).
pub fn save_commit_patch(repo_path: &str, oid: &str, dest: &str) -> Result<()> {
    let repo = open(repo_path)?;
    let commit = repo.find_commit(super::oid(oid)?)?;
    if commit.parent_count() > 1 {
        return err("Merge commits can't be saved as a patch.");
    }
    let email = git2::Email::from_commit(&commit, &mut EmailCreateOptions::new())?;
    std::fs::write(dest, email.as_slice())?;
    Ok(())
}

/// Save uncommitted changes (the staged ones, or everything not yet staged,
/// new files included) as a plain patch. Returns how many files it covers.
pub fn save_changes_patch(repo_path: &str, staged: bool, dest: &str) -> Result<usize> {
    let repo = open(repo_path)?;
    let mut o = DiffOptions::new();
    let diff = if staged {
        let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
        repo.diff_tree_to_index(head.as_ref(), None, Some(&mut o))?
    } else {
        o.include_untracked(true).recurse_untracked_dirs(true).show_untracked_content(true).show_binary(true);
        repo.diff_index_to_workdir(None, Some(&mut o))?
    };
    let n = diff.deltas().len();
    if n == 0 {
        return err(if staged { "There are no staged changes to save." } else { "There are no unstaged changes to save." });
    }
    let mut out = Vec::new();
    diff.print(DiffFormat::Patch, |_, _, line| {
        match line.origin() {
            '+' | '-' | ' ' => out.push(line.origin() as u8),
            _ => {}
        }
        out.extend_from_slice(line.content());
        true
    })?;
    std::fs::write(dest, out)?;
    Ok(n)
}

#[derive(Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub files: usize,
    /// From an email patch (`git format-patch`): its subject and message, to commit with.
    pub subject: Option<String>,
    pub body: Option<String>,
}

/// The subject and message of an email-style patch, if it is one.
fn email_message(text: &str) -> (Option<String>, Option<String>) {
    let text = text.replace("\r\n", "\n");
    if !text.starts_with("From ") {
        return (None, None);
    }
    let Some((headers, rest)) = text.split_once("\n\n") else { return (None, None) };
    // Headers can be folded over several lines (continuations start with whitespace).
    let mut subject: Option<String> = None;
    let mut in_subject = false;
    for line in headers.lines() {
        if let Some(s) = line.strip_prefix("Subject:") {
            subject = Some(s.trim().to_string());
            in_subject = true;
        } else if in_subject && line.starts_with([' ', '\t']) {
            if let Some(s) = subject.as_mut() {
                s.push(' ');
                s.push_str(line.trim());
            }
        } else {
            in_subject = false;
        }
    }
    // Drop the "[PATCH 1/3]" prefix.
    let subject = subject.map(|s| {
        let s = s.trim();
        if s.starts_with('[') {
            s.split_once(']').map(|(_, r)| r.trim().to_string()).unwrap_or_else(|| s.to_string())
        } else {
            s.to_string()
        }
    });
    let body: String = rest.lines().take_while(|l| *l != "---").collect::<Vec<_>>().join("\n");
    let body = body.trim();
    (subject.filter(|s| !s.is_empty()), (!body.is_empty()).then(|| body.to_string()))
}

/// Apply a patch file to the working copy (like `git apply`). Nothing changes
/// if any part of it doesn't apply.
pub fn apply_patch(repo_path: &str, file: &str) -> Result<Applied> {
    let repo = open(repo_path)?;
    let bytes = std::fs::read(file)?;
    let diff = Diff::from_buffer(&bytes).map_err(|e| Error::Msg(format!("That file isn't a patch Arbor can read: {}", e.message())))?;
    let mut paths: Vec<String> = vec![];
    for d in diff.deltas() {
        for f in [d.old_file(), d.new_file()] {
            if let Some(p) = f.path() {
                let p = p.to_string_lossy().to_string();
                if !paths.contains(&p) {
                    paths.push(p);
                }
            }
        }
    }
    if paths.is_empty() {
        return err("That patch doesn't change any files.");
    }
    let saved = undo::save_files(&repo, &paths)?;
    repo.apply(&diff, ApplyLocation::WorkDir, None).map_err(|e| {
        Error::Msg(format!(
            "The patch doesn't apply to your files as they are now, so nothing was changed. ({})",
            e.message()
        ))
    })?;
    let name = Path::new(file).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    undo::record(&repo, format!("Apply patch {name}"), Action::RestoreFiles { files: saved });
    let (subject, body) = email_message(&String::from_utf8_lossy(&bytes));
    Ok(Applied { files: diff.deltas().len(), subject, body })
}
