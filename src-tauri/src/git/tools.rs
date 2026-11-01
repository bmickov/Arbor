//! Opening a diff or a conflict in an external tool (Meld, VS Code, Beyond
//! Compare, KDiff3, …), or in whatever git's own difftool/mergetool settings
//! name. The two (or three) versions are written to temporary files; the
//! working-tree file is passed as itself, so edits made in the tool stick.

use super::{err, open, Error, Result};
use crate::platform::quiet_command;
use git2::Repository;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;

struct Preset {
    id: &'static str,
    name: &'static str,
    /// Programs to look for, first found wins.
    bins: &'static [&'static str],
    diff: &'static str,
    merge: &'static str,
}

const PRESETS: &[Preset] = &[
    Preset { id: "meld", name: "Meld", bins: &["meld"], diff: "$LOCAL $REMOTE", merge: "$LOCAL $BASE $REMOTE --output=$MERGED" },
    Preset { id: "vscode", name: "Visual Studio Code", bins: &["code", "codium", "code-oss"], diff: "--new-window --wait --diff $LOCAL $REMOTE", merge: "--new-window --wait --merge $LOCAL $REMOTE $BASE $MERGED" },
    Preset { id: "kdiff3", name: "KDiff3", bins: &["kdiff3"], diff: "$LOCAL $REMOTE", merge: "$BASE $LOCAL $REMOTE -o $MERGED" },
    Preset { id: "bc", name: "Beyond Compare", bins: &["bcompare", "bcomp", "BCompare"], diff: "$LOCAL $REMOTE", merge: "$LOCAL $REMOTE $BASE $MERGED" },
    Preset { id: "p4merge", name: "P4Merge", bins: &["p4merge"], diff: "$LOCAL $REMOTE", merge: "$BASE $LOCAL $REMOTE $MERGED" },
    Preset { id: "kaleidoscope", name: "Kaleidoscope", bins: &["ksdiff"], diff: "--partial-changeset --wait $LOCAL $REMOTE", merge: "--merge --wait --output $MERGED --base $BASE -- $LOCAL $REMOTE" },
    Preset { id: "winmerge", name: "WinMerge", bins: &["WinMergeU", "winmergeu"], diff: "/e /u /wl $LOCAL $REMOTE", merge: "/e /u /wl /wr /am /o $MERGED $LOCAL $BASE $REMOTE" },
    Preset { id: "kompare", name: "Kompare", bins: &["kompare"], diff: "$LOCAL $REMOTE", merge: "" },
];

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub merges: bool,
}

fn find(bins: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) { vec![".exe".into(), ".cmd".into(), String::new()] } else { vec![String::new()] };
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    if cfg!(target_os = "macos") {
        dirs.push("/usr/local/bin".into());
        dirs.push("/opt/homebrew/bin".into());
    }
    for b in bins {
        for d in &dirs {
            for e in &exts {
                let p = d.join(format!("{b}{e}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// The tools Arbor knows, and which are installed.
pub fn list() -> Vec<ToolInfo> {
    PRESETS
        .iter()
        .map(|p| ToolInfo { id: p.id.into(), name: p.name.into(), installed: find(p.bins).is_some(), merges: !p.merge.is_empty() })
        .collect()
}

/// Split a command line into words, keeping "quoted parts" together.
fn words(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    for ch in s.chars() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (None, '"') | (None, '\'') => {
                quote = Some(ch);
                has = true;
            }
            (None, c) if c.is_whitespace() => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (_, c) => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

/// The program and arguments for a tool: a preset id, or a custom command
/// line using $LOCAL, $REMOTE, $BASE and $MERGED.
fn command(tool: &str, custom: &str, merge: bool, vars: &[(&str, &Path)]) -> Result<std::process::Command> {
    let (program, template): (PathBuf, String) = if tool == "custom" {
        let w = words(custom);
        let Some(first) = w.first() else { return err("Enter the command for your tool in Preferences → Diffs & Git.") };
        (PathBuf::from(first), w[1..].join("\u{1f}"))
    } else {
        let p = PRESETS.iter().find(|p| p.id == tool).ok_or_else(|| Error::Msg(format!("Unknown tool '{tool}'.")))?;
        let t = if merge { p.merge } else { p.diff };
        if t.is_empty() {
            return err(format!("{} can't resolve conflicts. Pick another merge tool in Preferences → Diffs & Git.", p.name));
        }
        let bin = find(p.bins).ok_or_else(|| Error::Msg(format!("{} isn't installed (looked for {}).", p.name, p.bins.join(", "))))?;
        (bin, words(t).join("\u{1f}"))
    };
    let mut cmd = quiet_command(program);
    for arg in template.split('\u{1f}').filter(|a| !a.is_empty()) {
        let mut a = arg.to_string();
        for (k, v) in vars {
            a = a.replace(k, &v.to_string_lossy());
        }
        cmd.arg(a);
    }
    Ok(cmd)
}

fn blob_at(repo: &Repository, spec: &str, path: &str) -> Option<Vec<u8>> {
    let tree = repo.revparse_single(spec).ok()?.peel_to_tree().ok()?;
    let e = tree.get_path(Path::new(path)).ok()?;
    Some(repo.find_blob(e.id()).ok()?.content().to_vec())
}

fn index_blob(repo: &Repository, path: &str, stage: i32) -> Option<Vec<u8>> {
    let idx = repo.index().ok()?;
    let e = idx.get_path(Path::new(path), stage)?;
    Some(repo.find_blob(e.id).ok()?.content().to_vec())
}

/// A temporary copy named like the original ("server.rs" → "server.LOCAL.rs").
fn temp_copy(dir: &Path, path: &str, label: &str, bytes: &[u8]) -> Result<PathBuf> {
    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
    let file = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}.{label}.{ext}"),
        _ => format!("{name}.{label}"),
    };
    let p = dir.join(file);
    std::fs::write(&p, bytes)?;
    Ok(p)
}

fn temp_dir() -> Result<PathBuf> {
    let d = std::env::temp_dir().join(format!("arbor-tool-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

/// Open the change to `path` in a diff tool. `area`: "unstaged" (index ↔
/// working tree), "staged" (HEAD ↔ index), or a commit id (parent ↔ commit).
/// Returns right away; the tool runs on its own.
pub fn diff(repo_path: &str, path: &str, area: &str, tool: &str, custom: &str) -> Result<()> {
    let repo = open(repo_path)?;
    let root = repo.workdir().map(Path::to_path_buf).unwrap_or_default();
    if tool == "git" {
        let mut c = quiet_command("git");
        c.current_dir(&root).args(["difftool", "--no-prompt"]);
        match area {
            "unstaged" => {}
            "staged" => {
                c.arg("--cached");
            }
            commit => {
                c.arg(format!("{commit}^!"));
            }
        }
        c.arg("--").arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        c.spawn().map_err(|e| Error::Msg(format!("Couldn't run git difftool: {e}")))?;
        return Ok(());
    }
    let dir = temp_dir()?;
    let (local, mut remote) = match area {
        "unstaged" => {
            let old = index_blob(&repo, path, 0).or_else(|| blob_at(&repo, "HEAD", path)).unwrap_or_default();
            (temp_copy(&dir, path, "INDEX", &old)?, root.join(path))
        }
        "staged" => {
            let old = blob_at(&repo, "HEAD", path).unwrap_or_default();
            let new = index_blob(&repo, path, 0).unwrap_or_default();
            (temp_copy(&dir, path, "HEAD", &old)?, temp_copy(&dir, path, "STAGED", &new)?)
        }
        commit => {
            let c = repo.revparse_single(commit)?.peel_to_commit()?;
            let short = super::short(c.id());
            let new = blob_at(&repo, commit, path).unwrap_or_default();
            let old = c.parent(0).ok().and_then(|p| blob_at(&repo, &p.id().to_string(), path)).unwrap_or_default();
            (temp_copy(&dir, path, &format!("{short}~"), &old)?, temp_copy(&dir, path, &short, &new)?)
        }
    };
    if !remote.exists() {
        // Deleted in the working tree: compare against an empty file.
        remote = temp_copy(&dir, path, "DELETED", b"")?;
    }
    let mut cmd = command(tool, custom, false, &[("$LOCAL", &local), ("$REMOTE", &remote)])?;
    cmd.current_dir(&root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().map_err(|e| Error::Msg(format!("Couldn't start the diff tool: {e}")))?;
    Ok(())
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MergeResult {
    /// The file still has conflict markers (or the tool was closed without saving).
    pub markers_left: bool,
}

/// Resolve a conflicted file in a merge tool and wait for it to close.
pub fn merge(repo_path: &str, path: &str, tool: &str, custom: &str) -> Result<MergeResult> {
    let repo = open(repo_path)?;
    let root = repo.workdir().map(Path::to_path_buf).unwrap_or_default();
    let merged = root.join(path);
    if tool == "git" {
        let st = quiet_command("git")
            .current_dir(&root)
            .args(["mergetool", "--no-prompt", "--"])
            .arg(path)
            .stdin(Stdio::null())
            .status()
            .map_err(|e| Error::Msg(format!("Couldn't run git mergetool: {e}")))?;
        let _ = st;
    } else {
        let dir = temp_dir()?;
        let base = temp_copy(&dir, path, "BASE", &index_blob(&repo, path, 1).unwrap_or_default())?;
        let local = temp_copy(&dir, path, "LOCAL", &index_blob(&repo, path, 2).unwrap_or_default())?;
        let remote = temp_copy(&dir, path, "REMOTE", &index_blob(&repo, path, 3).unwrap_or_default())?;
        let mut cmd = command(tool, custom, true, &[("$BASE", &base), ("$LOCAL", &local), ("$REMOTE", &remote), ("$MERGED", &merged)])?;
        cmd.current_dir(&root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        cmd.status().map_err(|e| Error::Msg(format!("Couldn't start the merge tool: {e}")))?;
        let _ = std::fs::remove_dir_all(&dir);
    }
    let text = std::fs::read_to_string(&merged).unwrap_or_default();
    let markers_left = text.lines().any(|l| l.starts_with("<<<<<<<") || l.starts_with(">>>>>>>"));
    Ok(MergeResult { markers_left })
}

#[cfg(test)]
mod tests {
    use super::words;
    #[test]
    fn splits_command_lines() {
        assert_eq!(words(r#"code --wait "a b" $LOCAL"#), vec!["code", "--wait", "a b", "$LOCAL"]);
        assert_eq!(words("  x  "), vec!["x"]);
        assert_eq!(words(r#"'' y"#), vec!["", "y"]);
    }
}
