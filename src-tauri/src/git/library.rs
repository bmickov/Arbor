//! The repository library: every repository Arbor knows about, organised
//! into groups, plus folder scanning and quick status summaries for the
//! repository manager.

use super::Result;
use git2::{BranchType, Repository, StatusOptions};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub collapsed: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub path: String,
    pub name: String,
    /// Group id; None means ungrouped.
    #[serde(default)]
    pub group: Option<String>,
    /// Last time it was opened in Arbor (unix seconds).
    #[serde(default)]
    pub opened: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub groups: Vec<Group>,
    pub repos: Vec<Bookmark>,
}

// Serialises reads and writes of the library file within this process.
static LOCK: Mutex<()> = Mutex::new(());

fn file() -> Option<PathBuf> {
    super::config_dir().map(|d| d.join("library.json"))
}

fn read() -> Library {
    let Some(f) = file() else { return Library::default() };
    if let Some(lib) = std::fs::read_to_string(&f).ok().and_then(|s| serde_json::from_str(&s).ok()) {
        return lib;
    }
    // First run with a library: start from the recent-repositories list.
    let repos = super::repo::recent()
        .into_iter()
        .map(|r| Bookmark { path: r.path, name: r.name, group: None, opened: r.opened })
        .collect();
    Library { groups: vec![], repos }
}

fn write(lib: &Library) -> Result<()> {
    if let Some(f) = file() {
        if let Some(dir) = f.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = f.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(lib)?)?;
        std::fs::rename(tmp, f)?;
    }
    Ok(())
}

pub fn get() -> Library {
    let _g = LOCK.lock().unwrap();
    read()
}

pub fn save(lib: &Library) -> Result<()> {
    let _g = LOCK.lock().unwrap();
    write(lib)
}

/// Add a repository (or mark an existing one as just opened).
pub fn touch(path: &str, name: &str) {
    let _g = LOCK.lock().unwrap();
    let mut lib = read();
    let now = chrono::Utc::now().timestamp();
    match lib.repos.iter_mut().find(|r| r.path == path) {
        Some(r) => r.opened = now,
        None => lib.repos.push(Bookmark { path: path.into(), name: name.into(), group: None, opened: now }),
    }
    let _ = write(&lib);
}

/// Add many repositories at once (from a folder scan), optionally into a group.
pub fn add_many(paths: &[String], group: Option<String>) -> Result<Library> {
    let _g = LOCK.lock().unwrap();
    let mut lib = read();
    for p in paths {
        if lib.repos.iter().any(|r| r.path == *p) {
            continue;
        }
        let name = Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.clone());
        lib.repos.push(Bookmark { path: p.clone(), name, group: group.clone(), opened: 0 });
    }
    write(&lib)?;
    Ok(lib)
}

const SKIP: &[&str] = &["node_modules", "target", "vendor", "dist", "build", "out", "venv", "__pycache__", "Pods", "DerivedData"];

/// Find Git repositories under `root`, up to `max_depth` folders deep.
/// Doesn't descend into repositories it finds, hidden folders or build output.
pub fn scan(root: &str, max_depth: usize) -> Result<Vec<String>> {
    let mut out = vec![];
    let mut stack = vec![(PathBuf::from(root), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if out.len() >= 1000 {
            break;
        }
        if dir.join(".git").exists() {
            if let Ok(r) = Repository::open(&dir) {
                if let Some(wd) = r.workdir() {
                    out.push(wd.to_string_lossy().trim_end_matches('/').to_string());
                }
            }
            continue;
        }
        if depth >= max_depth {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || SKIP.contains(&name.as_str()) {
                continue;
            }
            stack.push((e.path(), depth + 1));
        }
    }
    out.sort_by_key(|p| p.to_lowercase());
    Ok(out)
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub path: String,
    /// False when the folder is gone or no longer a repository.
    pub ok: bool,
    pub error: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    /// Files with staged or unstaged changes, including new files.
    pub changes: usize,
    pub conflicts: usize,
    /// clean | merge | rebase | ...
    pub state: String,
    pub last_commit: Option<i64>,
    pub last_summary: Option<String>,
    pub stashes: usize,
}

fn summarize(path: &str) -> Summary {
    let mut s = Summary { path: path.into(), ..Default::default() };
    let mut repo = match Repository::open(path) {
        Ok(r) => r,
        Err(e) => {
            s.error = Some(if Path::new(path).exists() { e.message().to_string() } else { "Folder not found".into() });
            return s;
        }
    };
    s.ok = true;
    s.detached = repo.head_detached().unwrap_or(false);
    s.branch = super::head_branch(&repo);
    s.state = format!("{:?}", repo.state()).to_lowercase();
    if let Ok(c) = repo.head().and_then(|h| h.peel_to_commit()) {
        s.last_commit = Some(c.time().seconds());
        s.last_summary = Some(super::summary(&c));
    }
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).include_ignored(false).exclude_submodules(true);
    if let Ok(st) = repo.statuses(Some(&mut opts)) {
        s.changes = st.len();
        s.conflicts = st.iter().filter(|e| e.status().contains(git2::Status::CONFLICTED)).count();
    }
    if let Some(name) = &s.branch {
        if let Ok(b) = repo.find_branch(name, BranchType::Local) {
            if let Ok(up) = b.upstream() {
                s.upstream = up.name().ok().flatten().map(|n| n.to_string());
                if let (Some(l), Some(u)) = (b.get().target(), up.get().target()) {
                    let (a, bh) = repo.graph_ahead_behind(l, u).unwrap_or((0, 0));
                    s.ahead = a;
                    s.behind = bh;
                }
            }
        }
    }
    let mut n = 0;
    let _ = repo.stash_foreach(|_, _, _| {
        n += 1;
        true
    });
    s.stashes = n;
    s
}

/// Status summaries for many repositories, computed in parallel.
pub fn summaries(paths: &[String]) -> Vec<Summary> {
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    let next = Mutex::new(0usize);
    let results: Mutex<Vec<Option<Summary>>> = Mutex::new(vec![None; paths.len()]);
    std::thread::scope(|scope| {
        for _ in 0..workers.min(paths.len().max(1)) {
            scope.spawn(|| loop {
                let i = {
                    let mut n = next.lock().unwrap();
                    let i = *n;
                    *n += 1;
                    i
                };
                if i >= paths.len() {
                    break;
                }
                let s = summarize(&paths[i]);
                results.lock().unwrap()[i] = Some(s);
            });
        }
    });
    results.into_inner().unwrap().into_iter().flatten().collect()
}
