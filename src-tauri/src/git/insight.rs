//! Looking ahead and looking inside: predicting whether a merge will
//! conflict (without touching anything), searching file contents at any
//! revision, and the repository's own settings (remotes, .gitignore, config).

use super::branches::resolve;
use super::{err, open, Error, Result};
use git2::{ObjectType, TreeWalkMode, TreeWalkResult};
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MergePrediction {
    /// "up-to-date", "fast-forward", "clean" or "conflicts".
    pub result: &'static str,
    pub conflicts: Vec<String>,
    /// Files both sides changed (whether or not they conflict).
    pub both_changed: usize,
}

/// What merging `source` into `target` would do, worked out in memory.
pub fn predict_merge(repo_path: &str, source: &str, target: &str) -> Result<MergePrediction> {
    let repo = open(repo_path)?;
    let s = repo.find_commit(resolve(&repo, source)?)?;
    let t = repo.find_commit(resolve(&repo, target)?)?;
    let base = repo.merge_base(s.id(), t.id())?;
    if base == s.id() {
        return Ok(MergePrediction { result: "up-to-date", conflicts: vec![], both_changed: 0 });
    }
    if base == t.id() {
        return Ok(MergePrediction { result: "fast-forward", conflicts: vec![], both_changed: 0 });
    }
    let idx = repo.merge_commits(&t, &s, None)?;
    let conflicts: Vec<String> = idx
        .conflicts()?
        .filter_map(|c| c.ok())
        .filter_map(|c| c.our.or(c.their).or(c.ancestor))
        .map(|e| String::from_utf8_lossy(&e.path).to_string())
        .collect();
    // How many files both sides touched, for "merges cleanly, but…" hints.
    let bt = repo.find_commit(base)?.tree()?;
    let changed = |c: &git2::Commit| -> Result<std::collections::HashSet<String>> {
        let d = repo.diff_tree_to_tree(Some(&bt), Some(&c.tree()?), None)?;
        Ok(d.deltas().filter_map(|d| d.new_file().path().or(d.old_file().path()).map(|p| p.to_string_lossy().to_string())).collect())
    };
    let both_changed = changed(&s)?.intersection(&changed(&t)?).count();
    Ok(MergePrediction { result: if conflicts.is_empty() { "clean" } else { "conflicts" }, conflicts, both_changed })
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    pub path: String,
    /// 1-based line number.
    pub line: usize,
    pub text: String,
    /// Character ranges of the match in `text`.
    pub ranges: Vec<[usize; 2]>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GrepResult {
    pub matches: Vec<Match>,
    pub files: usize,
    /// Stopped at the limit.
    pub more: bool,
}

/// Lines containing `query` in the files of `rev` (case-insensitive unless
/// `case`; a regular expression when `regex`). Binary and huge files are skipped.
pub fn grep(repo_path: &str, rev: &str, query: &str, case: bool, regex: bool, limit: usize) -> Result<GrepResult> {
    let repo = open(repo_path)?;
    if query.is_empty() {
        return Ok(GrepResult { matches: vec![], files: 0, more: false });
    }
    let pattern = if regex { query.to_string() } else { regex_escape(query) };
    let re = regex_lite::RegexBuilder::new(&pattern)
        .case_insensitive(!case)
        .build()
        .map_err(|e| Error::Msg(format!("That isn't a valid regular expression: {e}")))?;
    let tree = repo.revparse_single(rev)?.peel_to_tree()?;
    let mut matches = vec![];
    let mut files = std::collections::HashSet::new();
    let mut more = false;
    let mut failure: Option<git2::Error> = None;
    tree.walk(TreeWalkMode::PreOrder, |dir, entry| {
        if matches.len() >= limit {
            more = true;
            return TreeWalkResult::Abort;
        }
        if entry.kind() != Some(ObjectType::Blob) {
            return TreeWalkResult::Ok;
        }
        let blob = match repo.find_blob(entry.id()) {
            Ok(b) => b,
            Err(e) => {
                failure = Some(e);
                return TreeWalkResult::Abort;
            }
        };
        if blob.is_binary() || blob.size() > 4 * 1024 * 1024 {
            return TreeWalkResult::Ok;
        }
        let Ok(text) = std::str::from_utf8(blob.content()) else { return TreeWalkResult::Ok };
        let path = format!("{dir}{}", entry.name().unwrap_or(""));
        for (i, line) in text.lines().enumerate() {
            let found: Vec<[usize; 2]> = re
                .find_iter(line)
                .filter(|m| m.end() > m.start())
                .map(|m| [line[..m.start()].chars().count(), line[..m.end()].chars().count()])
                .collect();
            if found.is_empty() {
                continue;
            }
            files.insert(path.clone());
            // Very long lines (minified files) are trimmed around the first match.
            let (shown, ranges) = if line.chars().count() > 400 {
                let start = found[0][0].saturating_sub(60);
                let s: String = line.chars().skip(start).take(300).collect();
                let r = found.iter().filter(|r| r[0] >= start && r[1] <= start + 300).map(|r| [r[0] - start, r[1] - start]).collect();
                (format!("…{s}…"), r)
            } else {
                (line.to_string(), found)
            };
            let ranges = if shown.starts_with('…') { ranges.into_iter().map(|r: [usize; 2]| [r[0] + 1, r[1] + 1]).collect() } else { ranges };
            matches.push(Match { path: path.clone(), line: i + 1, text: shown, ranges });
            if matches.len() >= limit {
                more = true;
                return TreeWalkResult::Abort;
            }
        }
        TreeWalkResult::Ok
    })
    .or_else(|e| if more { Ok(()) } else { Err(e) })?;
    if let Some(e) = failure {
        return Err(e.into());
    }
    Ok(GrepResult { matches, files: files.len(), more })
}

fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

// ---- Repository settings ----

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    pub name: String,
    pub url: String,
    /// Set when pushes go somewhere else (remote.<name>.pushurl).
    pub push_url: Option<String>,
}

pub fn remotes(repo_path: &str) -> Result<Vec<RemoteInfo>> {
    let repo = open(repo_path)?;
    let mut out = vec![];
    for name in repo.remotes()?.iter().flatten().flatten() {
        let r = repo.find_remote(name)?;
        out.push(RemoteInfo {
            name: name.to_string(),
            url: r.url().ok().unwrap_or("").to_string(),
            push_url: r.pushurl().ok().flatten().map(str::to_string),
        });
    }
    Ok(out)
}

/// Rename a remote and/or change its URLs.
pub fn edit_remote(repo_path: &str, name: &str, new_name: &str, url: &str, push_url: Option<&str>) -> Result<()> {
    let repo = open(repo_path)?;
    let new_name = new_name.trim();
    if new_name.is_empty() || url.trim().is_empty() {
        return err("A remote needs a name and a URL.");
    }
    let mut name = name.to_string();
    if new_name != name {
        let problems = repo.remote_rename(&name, new_name)?;
        if let Some(p) = problems.iter().flatten().flatten().next() {
            return err(format!("Renamed, but this refspec needs fixing by hand: {p}"));
        }
        name = new_name.to_string();
    }
    repo.remote_set_url(&name, url.trim())?;
    match push_url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => repo.remote_set_pushurl(&name, Some(u))?,
        None => repo.remote_set_pushurl(&name, None)?,
    }
    Ok(())
}

pub fn read_ignore(repo_path: &str) -> Result<String> {
    let repo = open(repo_path)?;
    let root = repo.workdir().ok_or_else(|| Error::Msg("This repository has no working tree.".into()))?;
    Ok(std::fs::read_to_string(root.join(".gitignore")).unwrap_or_default())
}

pub fn write_ignore(repo_path: &str, text: &str) -> Result<()> {
    let repo = open(repo_path)?;
    let root = repo.workdir().ok_or_else(|| Error::Msg("This repository has no working tree.".into()))?;
    let mut t = text.to_string();
    if !t.is_empty() && !t.ends_with('\n') {
        t.push('\n');
    }
    std::fs::write(root.join(".gitignore"), t)?;
    Ok(())
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
    /// "local" (this repository), "global" or "system".
    pub level: String,
}

/// Every setting that applies to this repository, with where it comes from.
pub fn config(repo_path: &str) -> Result<Vec<ConfigEntry>> {
    let repo = open(repo_path)?;
    let cfg = repo.config()?;
    let mut out = vec![];
    let mut entries = cfg.entries(None)?;
    while let Some(e) = entries.next() {
        let e = e?;
        let level = match e.level() {
            git2::ConfigLevel::Local | git2::ConfigLevel::Worktree => "local",
            git2::ConfigLevel::Global | git2::ConfigLevel::XDG => "global",
            git2::ConfigLevel::System | git2::ConfigLevel::ProgramData => "system",
            _ => "other",
        };
        out.push(ConfigEntry {
            key: e.name().unwrap_or("").to_string(),
            value: e.value().unwrap_or("").to_string(),
            level: level.into(),
        });
    }
    Ok(out)
}

/// Set (or with `value` None, remove) a setting in this repository's config.
pub fn set_config(repo_path: &str, key: &str, value: Option<&str>) -> Result<()> {
    let repo = open(repo_path)?;
    let key = key.trim();
    if !key.contains('.') {
        return err("Config keys look like section.name, for example pull.rebase.");
    }
    let mut cfg = repo.config()?.open_level(git2::ConfigLevel::Local)?;
    match value {
        Some(v) => cfg.set_str(key, v)?,
        None => cfg.remove(key)?,
    }
    Ok(())
}
