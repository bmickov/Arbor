//! The conflict wizard's backend: everything about one conflicted file, and
//! saving the version the user put together.
//!
//! The UI parses conflict markers itself. It gets the file as it is now in
//! the working tree (which may already be half edited) and a fresh merge of
//! the three index stages in diff3 style, so it can always start over.

use super::{err, head_branch, open, short, Error, Result};
use git2::{IndexConflict, IndexEntry, Repository, RepositoryState};
use serde::Serialize;
use std::path::Path;

/// Files bigger than this aren't offered to the wizard.
const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictFile {
    pub path: String,
    /// The working-tree file, when it exists and is text.
    pub text: Option<String>,
    /// The three stages merged afresh with diff3-style markers (None for binary or deleted sides).
    pub merged: Option<String>,
    /// Which sides have the file: a missing side deleted it.
    pub has_base: bool,
    pub has_ours: bool,
    pub has_theirs: bool,
    pub binary: bool,
    /// What the sides are called: "main" and "feature/login", for example.
    pub ours_label: String,
    pub theirs_label: String,
    /// "merge", "cherry-pick", "revert", "rebase" or "stash".
    pub operation: String,
}

fn find(repo: &Repository, path: &str) -> Result<IndexConflict> {
    repo.index()?
        .conflicts()?
        .filter_map(|c| c.ok())
        .find(|c| {
            let e = c.our.as_ref().or(c.their.as_ref()).or(c.ancestor.as_ref());
            e.map(|e| String::from_utf8_lossy(&e.path) == path).unwrap_or(false)
        })
        .ok_or_else(|| Error::Msg(format!("{path} isn't conflicted any more.")))
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|&b| b == 0)
}

/// The branch (or commit) being merged, cherry-picked or reverted in.
fn incoming(repo: &Repository) -> (String, String) {
    let git = repo.path();
    if let Some(p) = super::rebase::paused(repo) {
        let label = git2::Oid::from_str(&p.step.oid)
            .ok()
            .and_then(|id| repo.find_commit(id).ok())
            .map(|c| format!("{} {}", short(c.id()), super::summary(&c)))
            .unwrap_or_else(|| "the commit being replayed".into());
        return ("rebase".into(), label);
    }
    let (op, file) = match repo.state() {
        RepositoryState::Merge => ("merge", "MERGE_HEAD"),
        RepositoryState::CherryPick | RepositoryState::CherryPickSequence => ("cherry-pick", "CHERRY_PICK_HEAD"),
        RepositoryState::Revert | RepositoryState::RevertSequence => ("revert", "REVERT_HEAD"),
        RepositoryState::Rebase | RepositoryState::RebaseInteractive | RepositoryState::RebaseMerge => ("rebase", "REBASE_HEAD"),
        // Conflicts with no operation in progress come from applying a stash.
        _ => ("stash", ""),
    };
    if op == "stash" {
        return (op.into(), "stash".into());
    }
    let id = std::fs::read_to_string(git.join(file)).ok().and_then(|s| git2::Oid::from_str(s.lines().next()?.trim()).ok());
    let Some(id) = id else { return (op.into(), "theirs".into()) };
    // A branch pointing at it reads better than a commit id.
    let name = repo.branches(None).ok().and_then(|bs| {
        bs.filter_map(|b| b.ok()).find_map(|(b, _)| {
            (b.get().target() == Some(id)).then(|| b.name().ok().flatten().map(|s| s.to_string())).flatten()
        })
    });
    (op.into(), name.unwrap_or_else(|| short(id)))
}

pub fn conflict_file(repo_path: &str, path: &str) -> Result<ConflictFile> {
    let repo = open(repo_path)?;
    let c = find(&repo, path)?;
    let (operation, theirs_label) = incoming(&repo);
    let ours_label = match super::rebase::paused(&repo) {
        Some(p) => format!("{} (rebased so far)", p.onto_label),
        None => head_branch(&repo).unwrap_or_else(|| "HEAD".into()),
    };

    let full = repo.workdir().ok_or_else(|| Error::Msg("This repository has no working tree.".into()))?.join(path);
    let bytes = std::fs::read(&full).ok();
    let mut binary = bytes.as_deref().map(is_binary).unwrap_or(false);
    for e in [&c.ancestor, &c.our, &c.their].into_iter().flatten() {
        if let Ok(b) = repo.find_blob(e.id) {
            binary |= b.is_binary();
        }
    }
    let too_big = bytes.as_ref().map(|b| b.len() > MAX_BYTES).unwrap_or(false);
    let text = match &bytes {
        Some(b) if !binary && !too_big => String::from_utf8(b.clone()).ok(),
        _ => None,
    };

    let merged = match (&c.our, &c.their) {
        (Some(ours), Some(theirs)) if !binary && !too_big => {
            // An add/add conflict has no base: merge against an empty file, as git does.
            let base = match &c.ancestor {
                Some(a) => clone_entry(a),
                None => {
                    let mut e = clone_entry(ours);
                    e.id = repo.blob(b"")?;
                    e
                }
            };
            let mut opts = git2::MergeFileOptions::new();
            opts.style_diff3(true).ancestor_label("base").our_label(&*ours_label).their_label(&*theirs_label);
            repo.merge_file_from_index(&base, ours, theirs, Some(&mut opts))
                .ok()
                .and_then(|r| String::from_utf8(r.content().to_vec()).ok())
        }
        _ => None,
    };

    Ok(ConflictFile {
        path: path.to_string(),
        text,
        merged,
        has_base: c.ancestor.is_some(),
        has_ours: c.our.is_some(),
        has_theirs: c.their.is_some(),
        binary,
        ours_label,
        theirs_label,
        operation,
    })
}

fn clone_entry(e: &IndexEntry) -> IndexEntry {
    IndexEntry {
        ctime: e.ctime,
        mtime: e.mtime,
        dev: e.dev,
        ino: e.ino,
        mode: e.mode,
        uid: e.uid,
        gid: e.gid,
        file_size: e.file_size,
        id: e.id,
        flags: e.flags,
        flags_extended: e.flags_extended,
        path: e.path.clone(),
    }
}

/// Save the resolved file and mark it resolved (stage it).
pub fn resolve_with_content(repo_path: &str, path: &str, content: &str) -> Result<()> {
    let repo = open(repo_path)?;
    find(&repo, path)?;
    if content.lines().any(|l| l.starts_with("<<<<<<< ") || l == "<<<<<<<" || l.starts_with(">>>>>>> ") || l == ">>>>>>>") {
        return err("The file still has conflict markers.");
    }
    let full = repo.workdir().ok_or_else(|| Error::Msg("This repository has no working tree.".into()))?.join(path);
    if let Some(dir) = full.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&full, content)?;
    let mut index = repo.index()?;
    index.add_path(Path::new(path))?;
    index.write()?;
    Ok(())
}
