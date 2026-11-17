//! Files in the repository: the tracked file list (for Quick Actions), and
//! browsing or exporting the files as they were at any commit.

use super::{open, Error, Result};
use git2::{ObjectType, TreeWalkMode, TreeWalkResult};
use serde::Serialize;
use std::io::Write;

/// Every tracked file (paths from the index), sorted.
pub fn tracked_files(repo_path: &str) -> Result<Vec<String>> {
    let repo = open(repo_path)?;
    let index = repo.index()?;
    let mut out: Vec<String> = index.iter().map(|e| String::from_utf8_lossy(&e.path).to_string()).collect();
    out.dedup(); // conflicted files appear once per stage
    Ok(out)
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TreeEntry {
    pub path: String,
    /// file | link | submodule
    pub kind: &'static str,
    pub size: u64,
    pub executable: bool,
}

fn commit_at<'r>(repo: &'r git2::Repository, rev: &str) -> Result<git2::Commit<'r>> {
    repo.revparse_single(rev)
        .and_then(|o| o.peel_to_commit())
        .map_err(|_| Error::Msg(format!("Couldn't find {rev}.")))
}

/// Every file in the repository at `rev` (a commit, branch or tag), in tree order.
pub fn tree_at(repo_path: &str, rev: &str) -> Result<Vec<TreeEntry>> {
    let repo = open(repo_path)?;
    if super::unborn(&repo) {
        return Ok(vec![]);
    }
    let tree = commit_at(&repo, rev)?.tree()?;
    let odb = repo.odb()?;
    let mut out = vec![];
    tree.walk(TreeWalkMode::PreOrder, |root, e| {
        let name = e.name().unwrap_or("");
        let path = format!("{root}{name}");
        match e.kind() {
            Some(ObjectType::Blob) => {
                let mode = e.filemode();
                let size = odb.read_header(e.id()).map(|(s, _)| s as u64).unwrap_or(0);
                out.push(TreeEntry {
                    path,
                    kind: if mode == 0o120000 { "link" } else { "file" },
                    size,
                    executable: mode == 0o100755,
                });
            }
            Some(ObjectType::Commit) => out.push(TreeEntry { path, kind: "submodule", size: 0, executable: false }),
            _ => {}
        }
        TreeWalkResult::Ok
    })?;
    Ok(out)
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    /// The text, when the file is text and not too big to show.
    pub text: Option<String>,
    pub binary: bool,
    pub size: u64,
}

/// Files bigger than this are listed but not shown.
const MAX_VIEW_BYTES: usize = 64 * 1024 * 1024;

/// One file's content at `rev`, for the file browser.
pub fn file_content(repo_path: &str, rev: &str, path: &str) -> Result<FileContent> {
    let repo = open(repo_path)?;
    let tree = commit_at(&repo, rev)?.tree()?;
    let entry = tree.get_path(std::path::Path::new(path)).map_err(|_| Error::Msg(format!("{path} isn't in {rev}.")))?;
    let blob = repo.find_blob(entry.id())?;
    let size = blob.size() as u64;
    if blob.is_binary() {
        return Ok(FileContent { text: None, binary: true, size });
    }
    if blob.size() > MAX_VIEW_BYTES {
        return Ok(FileContent { text: None, binary: false, size });
    }
    Ok(FileContent { text: Some(String::from_utf8_lossy(blob.content()).into_owned()), binary: false, size })
}

/// Save the files at `rev` as a ZIP archive, inside a top folder named
/// `folder` (like `git archive --prefix`). Returns how many files it holds.
/// Submodules aren't included (their contents live in other repositories).
pub fn export_zip(repo_path: &str, rev: &str, dest: &str, folder: &str) -> Result<usize> {
    let repo = open(repo_path)?;
    let commit = commit_at(&repo, rev)?;
    let tree = commit.tree()?;
    let file = std::fs::File::create(dest)?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let when = zip_time(commit.time().seconds());
    let base = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true)
        .last_modified_time(when);
    let prefix = if folder.is_empty() { String::new() } else { format!("{}/", folder.trim_end_matches('/')) };
    let mut count = 0;
    let mut failure: Option<Error> = None;
    tree.walk(TreeWalkMode::PreOrder, |root, e| {
        let name = format!("{prefix}{root}{}", e.name().unwrap_or(""));
        let r: Result<()> = (|| {
            match e.kind() {
                Some(ObjectType::Tree) => {
                    zip.add_directory(format!("{name}/"), base.unix_permissions(0o755)).map_err(zip_err)?;
                }
                Some(ObjectType::Blob) => {
                    let blob = repo.find_blob(e.id())?;
                    if e.filemode() == 0o120000 {
                        let target = String::from_utf8_lossy(blob.content()).to_string();
                        zip.add_symlink(name, target, base).map_err(zip_err)?;
                    } else {
                        let mode = if e.filemode() == 0o100755 { 0o755 } else { 0o644 };
                        zip.start_file(name, base.unix_permissions(mode)).map_err(zip_err)?;
                        zip.write_all(blob.content())?;
                    }
                    count += 1;
                }
                _ => {}
            }
            Ok(())
        })();
        match r {
            Ok(()) => TreeWalkResult::Ok,
            Err(e) => {
                failure = Some(e);
                TreeWalkResult::Abort
            }
        }
    })
    .ok();
    if let Some(e) = failure {
        let _ = std::fs::remove_file(dest);
        return Err(e);
    }
    zip.finish().map_err(zip_err)?.flush()?;
    Ok(count)
}

fn zip_err(e: zip::result::ZipError) -> Error {
    Error::Msg(format!("Couldn't write the ZIP file: {e}"))
}

/// A commit time as a ZIP timestamp (which can't go before 1980).
fn zip_time(secs: i64) -> zip::DateTime {
    let dt = chrono::DateTime::from_timestamp(secs.max(315_532_800), 0).unwrap_or_default().naive_utc();
    use chrono::{Datelike, Timelike};
    zip::DateTime::from_date_and_time(dt.year() as u16, dt.month() as u8, dt.day() as u8, dt.hour() as u8, dt.minute() as u8, dt.second() as u8)
        .unwrap_or_default()
}
