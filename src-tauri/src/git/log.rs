//! Commit history with a lane-based graph, commit details and file history.

use super::{oid, open, short, Result};
use git2::{DiffFindOptions, Oid, Repository, Sort};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RefLabel {
    /// head | local | remote | tag
    pub kind: &'static str,
    pub name: String,
}

/// One row of the graph. `segs` are line segments: [fromLane, toLane, part, color]
/// where part 0 = full row height, 1 = top half (into the node row's middle),
/// 2 = bottom half (out of the middle).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GraphRow {
    pub col: u16,
    pub color: u16,
    pub width: u16,
    pub segs: Vec<[u16; 4]>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CommitRow {
    pub oid: String,
    pub short: String,
    pub summary: String,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub parents: Vec<String>,
    pub refs: Vec<RefLabel>,
    pub graph: GraphRow,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Log {
    pub rows: Vec<CommitRow>,
    /// More commits exist beyond the limit.
    pub more: bool,
}

pub fn ref_labels(repo: &Repository) -> Result<HashMap<Oid, Vec<RefLabel>>> {
    let mut map: HashMap<Oid, Vec<RefLabel>> = HashMap::new();
    let head_branch = super::head_branch(repo);
    let detached = repo.head_detached().unwrap_or(false);
    if detached {
        if let Some(h) = super::head_oid(repo) {
            map.entry(h).or_default().push(RefLabel { kind: "head", name: "HEAD".into() });
        }
    }
    for r in repo.references()? {
        let r = r?;
        let Ok(name) = r.name() else { continue };
        let (kind, short_name) = if let Some(n) = name.strip_prefix("refs/heads/") {
            (if Some(n) == head_branch.as_deref() && !detached { "head" } else { "local" }, n)
        } else if let Some(n) = name.strip_prefix("refs/remotes/") {
            if n.ends_with("/HEAD") {
                continue;
            }
            ("remote", n)
        } else if let Some(n) = name.strip_prefix("refs/tags/") {
            ("tag", n)
        } else {
            continue;
        };
        let Ok(c) = r.peel_to_commit() else { continue };
        map.entry(c.id()).or_default().push(RefLabel { kind, name: short_name.to_string() });
    }
    for v in map.values_mut() {
        v.sort_by_key(|l| match l.kind {
            "head" => 0,
            "local" => 1,
            "remote" => 2,
            _ => 3,
        });
    }
    Ok(map)
}

struct Lanes {
    lanes: Vec<Option<(Oid, u16)>>,
    next_color: u16,
}

impl Lanes {
    fn find(&self, id: Oid) -> Option<usize> {
        self.lanes.iter().position(|l| l.map(|(o, _)| o) == Some(id))
    }
    fn free(&mut self) -> usize {
        match self.lanes.iter().position(|l| l.is_none()) {
            Some(i) => i,
            None => {
                self.lanes.push(None);
                self.lanes.len() - 1
            }
        }
    }
    fn color(&mut self) -> u16 {
        let c = self.next_color;
        self.next_color = self.next_color.wrapping_add(1);
        c
    }

    fn row(&mut self, id: Oid, parents: &[Oid]) -> GraphRow {
        let mut segs: Vec<[u16; 4]> = vec![];
        let existing = self.find(id);
        let (col, color) = match existing {
            Some(c) => (c, self.lanes[c].unwrap().1),
            None => {
                let c = self.free();
                let color = self.color();
                (c, color)
            }
        };
        // Other lanes that were also waiting for this commit join it here.
        for j in 0..self.lanes.len() {
            if j != col {
                if let Some((o, c)) = self.lanes[j] {
                    if o == id {
                        segs.push([j as u16, col as u16, 1, c]);
                        self.lanes[j] = None;
                    }
                }
            }
        }
        for j in 0..self.lanes.len() {
            if j != col {
                if let Some((_, c)) = self.lanes[j] {
                    segs.push([j as u16, j as u16, 0, c]);
                }
            }
        }
        if existing.is_some() {
            segs.push([col as u16, col as u16, 1, color]);
        }
        if col < self.lanes.len() {
            self.lanes[col] = None;
        }
        for (i, p) in parents.iter().enumerate() {
            match self.find(*p) {
                Some(k) => {
                    // Joining a lane that already exists: the curve belongs
                    // to this commit's branch, so it keeps this commit's colour.
                    segs.push([col as u16, k as u16, 2, color]);
                }
                None => {
                    let (k, c) = if i == 0 {
                        if col >= self.lanes.len() {
                            self.lanes.resize(col + 1, None);
                        }
                        (col, color)
                    } else {
                        let k = self.free();
                        (k, self.color())
                    };
                    self.lanes[k] = Some((*p, c));
                    segs.push([col as u16, k as u16, 2, c]);
                }
            }
        }
        while matches!(self.lanes.last(), Some(None)) {
            self.lanes.pop();
        }
        let width = segs.iter().map(|s| s[0].max(s[1])).max().unwrap_or(0).max(col as u16) + 1;
        GraphRow { col: col as u16, color, width, segs }
    }
}

/// `scope`: "all" for every branch, "head" for the current branch, or a ref name.
pub fn log(repo_path: &str, scope: &str, limit: usize) -> Result<Log> {
    let repo = open(repo_path)?;
    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
    match scope {
        "all" => {
            let _ = walk.push_head();
            walk.push_glob("refs/heads")?;
            walk.push_glob("refs/remotes")?;
            walk.push_glob("refs/tags")?;
        }
        "head" => {
            if walk.push_head().is_err() {
                return Ok(Log { rows: vec![], more: false });
            }
        }
        other => {
            let id = super::branches::resolve(&repo, other)?;
            walk.push(id)?;
        }
    }
    let labels = ref_labels(&repo)?;
    let mut lanes = Lanes { lanes: vec![], next_color: 0 };
    let mut rows = Vec::new();
    let mut more = false;
    for id in walk {
        let id = id?;
        if rows.len() >= limit {
            more = true;
            break;
        }
        let c = repo.find_commit(id)?;
        let parents: Vec<Oid> = c.parent_ids().collect();
        let graph = lanes.row(id, &parents);
        let author = c.author();
        rows.push(CommitRow {
            oid: id.to_string(),
            short: short(id),
            summary: super::summary(&c),
            author: author.name().unwrap_or("").to_string(),
            email: author.email().unwrap_or("").to_string(),
            time: author.when().seconds(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            refs: labels.get(&id).cloned().unwrap_or_default(),
            graph,
        });
    }
    Ok(Log { rows, more })
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub name: String,
    pub email: String,
    pub time: i64,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub oid: String,
    pub short: String,
    pub summary: String,
    pub body: String,
    pub author: Person,
    pub committer: Person,
    pub parents: Vec<String>,
    pub refs: Vec<RefLabel>,
    pub files: Vec<super::diff::FileDiff>,
}

fn person(s: &git2::Signature) -> Person {
    Person {
        name: s.name().unwrap_or("").to_string(),
        email: s.email().unwrap_or("").to_string(),
        time: s.when().seconds(),
    }
}

pub fn commit_info(repo_path: &str, id: &str) -> Result<CommitInfo> {
    let repo = open(repo_path)?;
    let c = repo.find_commit(oid(id)?)?;
    let msg = c.message().unwrap_or("");
    let (summary, body) = match msg.split_once('\n') {
        Some((s, b)) => (s.to_string(), b.trim().to_string()),
        None => (msg.trim().to_string(), String::new()),
    };
    let labels = ref_labels(&repo)?;
    let author = person(&c.author());
    let committer = person(&c.committer());
    let info = CommitInfo {
        oid: c.id().to_string(),
        short: short(c.id()),
        summary,
        body,
        author,
        committer,
        parents: c.parent_ids().map(|p| p.to_string()).collect(),
        refs: labels.get(&c.id()).cloned().unwrap_or_default(),
        files: super::diff::commit_files(repo_path, id)?,
    };
    Ok(info)
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileHistoryEntry {
    pub commit: CommitRow,
    /// The file's path in this commit (it may have been renamed later).
    pub path: String,
}

/// Commits on HEAD that changed `path`, newest first, following renames.
pub fn file_history(repo_path: &str, path: &str, limit: usize) -> Result<Vec<FileHistoryEntry>> {
    let repo = open(repo_path)?;
    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
    if walk.push_head().is_err() {
        return Ok(vec![]);
    }
    let labels = ref_labels(&repo)?;
    let mut current = path.to_string();
    let mut out = vec![];
    for id in walk {
        let id = id?;
        let c = repo.find_commit(id)?;
        let tree = c.tree()?;
        let Ok(entry) = tree.get_path(Path::new(&current)) else { continue };
        let parent_tree = if c.parent_count() > 0 { Some(c.parent(0)?.tree()?) } else { None };
        let parent_entry = parent_tree.as_ref().and_then(|t| t.get_path(Path::new(&current)).ok());
        let changed = match &parent_entry {
            Some(pe) => pe.id() != entry.id(),
            None => true,
        };
        if !changed {
            continue;
        }
        let author = c.author();
        out.push(FileHistoryEntry {
            commit: CommitRow {
                oid: id.to_string(),
                short: short(id),
                summary: super::summary(&c),
                author: author.name().unwrap_or("").to_string(),
                email: author.email().unwrap_or("").to_string(),
                time: author.when().seconds(),
                parents: c.parent_ids().map(|p| p.to_string()).collect(),
                refs: labels.get(&id).cloned().unwrap_or_default(),
                graph: GraphRow { col: 0, color: 0, width: 1, segs: vec![] },
            },
            path: current.clone(),
        });
        if out.len() >= limit {
            break;
        }
        // Added here: was it renamed from somewhere?
        if parent_entry.is_none() {
            if let Some(pt) = &parent_tree {
                let mut d = repo.diff_tree_to_tree(Some(pt), Some(&tree), None)?;
                let mut f = DiffFindOptions::new();
                f.renames(true);
                d.find_similar(Some(&mut f))?;
                let renamed_from = d.deltas().find_map(|delta| {
                    let np = delta.new_file().path()?.to_string_lossy().to_string();
                    (delta.status() == git2::Delta::Renamed && np == current)
                        .then(|| delta.old_file().path().map(|p| p.to_string_lossy().to_string()))
                        .flatten()
                });
                match renamed_from {
                    Some(old) => current = old,
                    None => break,
                }
            }
        }
    }
    Ok(out)
}

// ---- Searching the whole history ----

#[derive(Deserialize, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LogFilter {
    /// Message, author or commit ID contains this (case-insensitive).
    pub text: Option<String>,
    /// Author name or email contains this (case-insensitive).
    pub author: Option<String>,
    /// Unix seconds, inclusive.
    pub since: Option<i64>,
    pub until: Option<i64>,
    /// Commits that change this file or anything inside this folder.
    pub path: Option<String>,
    /// Commits that add or remove this text (like `git log -S`).
    pub contains: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub rows: Vec<CommitRow>,
    /// How many commits were looked at.
    pub scanned: usize,
    /// Stopped early: more matches may exist.
    pub more: bool,
}

fn nonempty(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// How often `needle` occurs in a blob (0 for binary or missing).
fn occurrences(repo: &Repository, id: Oid, needle: &[u8]) -> usize {
    if id.is_zero() {
        return 0;
    }
    let Ok(b) = repo.find_blob(id) else { return 0 };
    if b.is_binary() {
        return 0;
    }
    let hay = b.content();
    if needle.is_empty() || hay.len() < needle.len() {
        return 0;
    }
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

pub fn search(repo_path: &str, scope: &str, f: &LogFilter, limit: usize) -> Result<SearchResult> {
    let repo = open(repo_path)?;
    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
    match scope {
        "all" => {
            let _ = walk.push_head();
            walk.push_glob("refs/heads")?;
            walk.push_glob("refs/remotes")?;
            walk.push_glob("refs/tags")?;
        }
        "head" => {
            if walk.push_head().is_err() {
                return Ok(SearchResult { rows: vec![], scanned: 0, more: false });
            }
        }
        other => {
            let id = super::branches::resolve(&repo, other)?;
            walk.push(id)?;
        }
    }
    let text = nonempty(&f.text).map(str::to_lowercase);
    let author = nonempty(&f.author).map(str::to_lowercase);
    let path = nonempty(&f.path).map(|p| p.trim_matches('/').to_string());
    let contains = nonempty(&f.contains).map(|s| s.as_bytes().to_vec());
    let labels = ref_labels(&repo)?;
    // Searching inside changes is slow on huge histories; give up after a while.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut rows = vec![];
    let mut scanned = 0;
    let mut more = false;
    for id in walk {
        let id = id?;
        if rows.len() >= limit || std::time::Instant::now() > deadline {
            more = true;
            break;
        }
        scanned += 1;
        let c = repo.find_commit(id)?;
        let a = c.author();
        let when = a.when().seconds();
        if f.since.is_some_and(|s| when < s) || f.until.is_some_and(|u| when > u) {
            continue;
        }
        let name = a.name().unwrap_or("");
        let email = a.email().unwrap_or("");
        if let Some(au) = &author {
            if !name.to_lowercase().contains(au) && !email.to_lowercase().contains(au) {
                continue;
            }
        }
        if let Some(t) = &text {
            let msg = c.message().unwrap_or("").to_lowercase();
            if !msg.contains(t) && !name.to_lowercase().contains(t) && !id.to_string().starts_with(t.as_str()) {
                continue;
            }
        }
        if path.is_some() || contains.is_some() {
            let tree = c.tree()?;
            let parent = c.parent(0).ok().map(|p| p.tree()).transpose()?;
            let mut opts = git2::DiffOptions::new();
            if let Some(p) = &path {
                opts.pathspec(p);
            }
            let diff = repo.diff_tree_to_tree(parent.as_ref(), Some(&tree), Some(&mut opts))?;
            if diff.deltas().len() == 0 {
                continue;
            }
            if let Some(needle) = &contains {
                let hit = diff.deltas().any(|d| occurrences(&repo, d.old_file().id(), needle) != occurrences(&repo, d.new_file().id(), needle));
                if !hit {
                    continue;
                }
            }
        }
        rows.push(CommitRow {
            oid: id.to_string(),
            short: short(id),
            summary: super::summary(&c),
            author: name.to_string(),
            email: email.to_string(),
            time: when,
            parents: c.parent_ids().map(|p| p.to_string()).collect(),
            refs: labels.get(&id).cloned().unwrap_or_default(),
            graph: GraphRow { col: 0, color: 0, width: 0, segs: vec![] },
        });
    }
    Ok(SearchResult { rows, scanned, more })
}
