//! Commit history with a lane-based graph, commit details and file history.

use super::{oid, open, short, Result};
use git2::{DiffFindOptions, Oid, Repository, Sort};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
                // The first parent is already awaited by a lane to the right:
                // carry on straight down here and let that lane bend in when
                // the parent comes (git's "|/"), rather than bending this
                // commit's line out to the right.
                Some(k) if i == 0 && k > col => {
                    if col >= self.lanes.len() {
                        self.lanes.resize(col + 1, None);
                    }
                    self.lanes[col] = Some((*p, color));
                    segs.push([col as u16, col as u16, 2, color]);
                }
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

/// A lazy walk of the commits reachable from some tips, newest commit date
/// first. libgit2's own revwalk reads the entire history before returning
/// anything as soon as any sort order is asked for (even date order), which
/// costs a quarter of a second per 40,000 commits.
pub struct DateWalk<'r> {
    repo: &'r Repository,
    heap: std::collections::BinaryHeap<(i64, std::cmp::Reverse<u64>, Oid)>,
    seen: std::collections::HashSet<Oid>,
    seq: u64,
}

impl<'r> DateWalk<'r> {
    pub fn new(repo: &'r Repository) -> Self {
        DateWalk { repo, heap: Default::default(), seen: Default::default(), seq: 0 }
    }

    /// Start from a commit (or anything that peels to one; others are ignored).
    pub fn push(&mut self, id: Oid) {
        let Ok(c) = self.repo.find_object(id, None).and_then(|o| o.peel_to_commit()) else { return };
        if self.seen.insert(c.id()) {
            self.seq += 1;
            self.heap.push((c.time().seconds(), std::cmp::Reverse(self.seq), c.id()));
        }
    }

    /// Start from HEAD and every branch, remote branch and tag.
    pub fn push_all(&mut self) -> Result<()> {
        if let Ok(h) = self.repo.head() {
            if let Some(t) = h.target() {
                self.push(t);
            }
        }
        for r in self.repo.references()?.flatten() {
            let name = r.name().unwrap_or("");
            if name.starts_with("refs/heads/") || name.starts_with("refs/remotes/") || name.starts_with("refs/tags/") {
                if let Some(t) = r.resolve().ok().and_then(|r| r.target()) {
                    self.push(t);
                }
            }
        }
        Ok(())
    }
}

impl<'r> Iterator for DateWalk<'r> {
    type Item = Result<git2::Commit<'r>>;
    fn next(&mut self) -> Option<Self::Item> {
        let (_, _, id) = self.heap.pop()?;
        let c = match self.repo.find_commit(id) {
            Ok(c) => c,
            Err(e) => return Some(Err(e.into())),
        };
        for p in c.parent_ids() {
            if self.seen.insert(p) {
                if let Ok(pc) = self.repo.find_commit(p) {
                    self.seq += 1;
                    self.heap.push((pc.time().seconds(), std::cmp::Reverse(self.seq), p));
                }
            }
        }
        Some(Ok(c))
    }
}

/// The first `limit` commits of a date-ordered walk (a few more when several
/// share the last one's time), put in topological order: children before
/// parents, newest first among those ready. The bool says whether there were
/// more.
///
/// libgit2's TOPOLOGICAL sort walks the entire history before returning the
/// first commit, so a page of 2,000 commits cost as much as all of them (and
/// seconds on a big repository). A date-ordered walk is lazy; sorting just
/// the page fixes the rare commit whose date is older than its parent's.
fn topo_page<'r>(walk: DateWalk<'r>, limit: usize) -> Result<(Vec<git2::Commit<'r>>, bool)> {
    let limit = limit.max(1);
    let mut page: Vec<git2::Commit<'r>> = Vec::with_capacity(limit.min(1 << 16) + 1);
    let mut more = false;
    for c in walk {
        let c = c?;
        // Past the limit, keep going only through commits with the same time
        // as the last one: same-second commits have no date order, and
        // stopping among them could leave a child out while its parent is in.
        if page.len() >= limit && page.last().is_some_and(|l| l.time().seconds() != c.time().seconds()) {
            more = true;
            break;
        }
        page.push(c);
    }
    // Kahn's algorithm within the page, newest commit first among the ready.
    let index: HashMap<Oid, usize> = page.iter().enumerate().map(|(i, c)| (c.id(), i)).collect();
    let mut children = vec![0usize; page.len()];
    for c in &page {
        for p in c.parent_ids() {
            if let Some(&j) = index.get(&p) {
                children[j] += 1;
            }
        }
    }
    let key = |i: usize| (page[i].time().seconds(), std::cmp::Reverse(i));
    let mut ready: std::collections::BinaryHeap<(i64, std::cmp::Reverse<usize>)> = (0..page.len()).filter(|&i| children[i] == 0).map(key).collect();
    let mut order = Vec::with_capacity(page.len());
    while let Some((_, std::cmp::Reverse(i))) = ready.pop() {
        order.push(i);
        for p in page[i].parent_ids() {
            if let Some(&j) = index.get(&p) {
                children[j] -= 1;
                if children[j] == 0 {
                    ready.push(key(j));
                }
            }
        }
    }
    let mut slots: Vec<Option<git2::Commit<'r>>> = page.into_iter().map(Some).collect();
    let sorted: Vec<git2::Commit<'r>> = order.into_iter().filter_map(|i| slots[i].take()).collect();
    Ok((sorted, more))
}

/// `scope`: "all" for every branch, "head" for the current branch, or a ref name.
/// The commits of `scope` ("all", "head" or a branch), newest first; None
/// when there are none (an unborn HEAD).
fn scoped_walk<'r>(repo: &'r Repository, scope: &str) -> Result<Option<DateWalk<'r>>> {
    let mut walk = DateWalk::new(repo);
    match scope {
        "all" => walk.push_all()?,
        "head" => match repo.head().ok().and_then(|h| h.target()) {
            Some(t) => walk.push(t),
            None => return Ok(None),
        },
        other => walk.push(super::branches::resolve(repo, other)?),
    }
    Ok(Some(walk))
}

/// How many commits the log of `scope` must load to include `id` (None:
/// it isn't in that history, or not within `max` commits).
pub fn log_depth(repo_path: &str, scope: &str, id: &str, max: usize) -> Result<Option<usize>> {
    let repo = open(repo_path)?;
    let target = oid(id)?;
    let Some(walk) = scoped_walk(&repo, scope)? else { return Ok(None) };
    for (i, c) in walk.take(max).enumerate() {
        if c?.id() == target {
            return Ok(Some(i + 1));
        }
    }
    Ok(None)
}

pub fn log(repo_path: &str, scope: &str, limit: usize) -> Result<Log> {
    let repo = open(repo_path)?;
    let Some(walk) = scoped_walk(&repo, scope)? else { return Ok(Log { rows: vec![], more: false }) };
    let labels = ref_labels(&repo)?;
    let mut lanes = Lanes { lanes: vec![], next_color: 0 };
    let (page, more) = topo_page(walk, limit)?;
    let mut rows = Vec::with_capacity(page.len());
    for c in page {
        let id = c.id();
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
    pub files: Vec<super::diff::FileDiff>,
}

fn person(s: &git2::Signature) -> Person {
    Person {
        name: s.name().unwrap_or("").to_string(),
        email: s.email().unwrap_or("").to_string(),
        time: s.when().seconds(),
    }
}

fn info_of(c: &git2::Commit, files: Vec<super::diff::FileDiff>) -> CommitInfo {
    let msg = c.message().unwrap_or("");
    let (summary, body) = match msg.split_once('\n') {
        Some((s, b)) => (s.to_string(), b.trim().to_string()),
        None => (msg.trim().to_string(), String::new()),
    };
    CommitInfo {
        oid: c.id().to_string(),
        short: short(c.id()),
        summary,
        body,
        author: person(&c.author()),
        committer: person(&c.committer()),
        parents: c.parent_ids().map(|p| p.to_string()).collect(),
        files,
    }
}

pub fn commit_info(repo_path: &str, id: &str) -> Result<CommitInfo> {
    let repo = super::open_for_reading(repo_path)?;
    let c = repo.find_commit(oid(id)?)?;
    let diffs = super::diff::commit_diffs(&repo, id, Default::default(), &[])?;
    let files = super::diff::summaries(&diffs)?;
    Ok(info_of(&c, files))
}

/// A commit's message alone (no diff), for editing it.
#[derive(Serialize, Debug)]
pub struct CommitMessage {
    pub summary: String,
    pub body: String,
}

pub fn commit_message(repo_path: &str, id: &str) -> Result<CommitMessage> {
    let repo = open(repo_path)?;
    let c = repo.find_commit(oid(id)?)?;
    let i = info_of(&c, vec![]);
    Ok(CommitMessage { summary: i.summary, body: i.body })
}

/// What the commit view shows first: the commit, its files, and the diff
/// of one of them (`want` if the commit changed it, else the first), all
/// from one comparison of the trees.
#[derive(Serialize, Debug)]
pub struct CommitView {
    pub info: CommitInfo,
    pub diff: Option<super::diff::FileDiff>,
}

pub fn commit_view(repo_path: &str, id: &str, want: Option<&str>, s: super::diff::DiffSettings) -> Result<CommitView> {
    let repo = super::open_for_reading(repo_path)?;
    let c = repo.find_commit(oid(id)?)?;
    let diffs = super::diff::commit_diffs(&repo, id, Default::default(), &[])?;
    let files = super::diff::summaries(&diffs)?;
    let shown = want.and_then(|w| files.iter().find(|f| f.path == w)).or(files.first());
    let diff = match shown {
        None => None,
        // The file list is compared with the default settings; other
        // settings (context, whitespace) need the file compared again.
        Some(f) if s.context.is_none() && !s.ignore_whitespace.unwrap_or(false) => super::diff::pick_in(&diffs, &f.path, s.max_lines)?,
        Some(f) => super::diff::commit_file_listed(repo_path, id, &f.path, f.old_path.as_deref(), true, s)?,
    };
    Ok(CommitView { info: info_of(&c, files), diff })
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
    let repo = super::open_for_reading(repo_path)?;
    let mut walk = repo.revwalk()?;
    // Topological: commits made in the same second (a rebase makes many)
    // have no date order. Finding every change to a file walks the whole
    // history anyway, so the full walk this needs costs little extra.
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
    if walk.push_head().is_err() {
        return Ok(vec![]);
    }
    let labels = ref_labels(&repo)?;
    let ids: Vec<Oid> = walk.collect::<std::result::Result<_, _>>()?;
    let mut current = path.to_string();
    let mut trail = PathTrail::new(&repo, &current);
    let mut out = vec![];
    let (mut batch, mut batch_from): (Vec<Lookup>, usize) = (vec![], 0);
    for (k, &id) in ids.iter().enumerate() {
        // The path's ids in each commit and its parents, worked out a batch
        // at a time on several threads (reading trees is most of the work).
        // After a rename the older path is looked up here, as it comes.
        let look = if current == path {
            if !(batch_from..batch_from + batch.len()).contains(&k) {
                batch_from = k;
                batch = lookups(repo_path, &ids[k..ids.len().min(k + LOOKUP_BATCH)], path, false)?;
            }
            std::mem::take(&mut batch[k - batch_from])
        } else {
            lookup(&repo, &mut trail, id, false)?
        };
        let Some(entry) = look.entry else { continue };
        // Unchanged from the first parent; or a merge that took the file as
        // it was on another parent (as git log shows it: the change is on
        // that side).
        if look.parents.contains(&Some(entry)) {
            continue;
        }
        let c = repo.find_commit(id)?;
        let parent_entry = look.parents.first().copied().flatten();
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
            if let Ok(p) = c.parent(0) {
                let mut d = super::diff::diff_trees(&repo, Some(&p.tree()?), &c.tree()?, &mut git2::DiffOptions::new())?;
                let mut f = DiffFindOptions::new();
                f.renames(true);
                d.find_similar(Some(&mut f))?;
                let renamed_from = d.deltas().find_map(|delta| {
                    let np = delta.new_file().path()?.to_string_lossy().to_string();
                    (delta.status() == git2::Delta::Renamed && np == current)
                        .then(|| delta.old_file().path().map(|p| p.to_string_lossy().to_string()))
                        .flatten()
                });
                // Not renamed: keep going all the same. The file may have
                // been deleted and added again, and commits on a branch
                // merged later come after this in the walk.
                if let Some(old) = renamed_from {
                    current = old;
                    trail = PathTrail::new(&repo, &current);
                }
            }
        }
    }
    Ok(out)
}

/// A path's id in a commit (None: not there) and in each of its parents.
#[derive(Default, Clone)]
struct Lookup {
    entry: Option<Oid>,
    parents: Vec<Option<Oid>>,
}

const LOOKUP_BATCH: usize = 8192;

/// `always`: look in the parents even where the path isn't in the commit
/// (to see deletions); else they're skipped there.
fn lookup(repo: &Repository, trail: &mut PathTrail, id: Oid, always: bool) -> Result<Lookup> {
    let c = repo.find_commit(id)?;
    let entry = trail.id_in(c.tree_id())?;
    let mut parents = vec![];
    if always || entry.is_some() {
        for p in c.parent_ids() {
            parents.push(trail.id_in(repo.find_commit(p)?.tree_id())?);
            // Same as the first parent: the others don't matter.
            if parents[0] == entry {
                break;
            }
        }
    }
    Ok(Lookup { entry, parents })
}

/// `lookup` for many commits, split over a few threads (each with its own
/// repository handle) when there are enough to be worth it.
fn lookups(repo_path: &str, ids: &[Oid], path: &str, always: bool) -> Result<Vec<Lookup>> {
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let threads = if ids.len() < 2000 { 1 } else { cpus.clamp(1, 8) };
    let run = |part: &[Oid]| -> Result<Vec<Lookup>> {
        let repo = super::open_for_reading(repo_path)?;
        let mut trail = PathTrail::new(&repo, path);
        part.iter().map(|&id| lookup(&repo, &mut trail, id, always)).collect()
    };
    if threads == 1 {
        return run(ids);
    }
    std::thread::scope(|s| {
        let parts: Vec<_> = ids.chunks(ids.len().div_ceil(threads)).map(|part| s.spawn(move || run(part))).collect();
        let mut out = Vec::with_capacity(ids.len());
        for h in parts {
            out.extend(h.join().map_err(|_| super::Error::Msg("Reading the history failed.".into()))??);
        }
        Ok(out)
    })
}

/// Looks one path up in many trees, as when going through history. Most
/// folders along the path are the same from one commit to the next (same
/// id), so what each folder holds for the path is remembered and each is
/// read once, rather than twice per commit.
struct PathTrail<'r> {
    repo: &'r Repository,
    parts: Vec<String>,
    /// (folder id, depth) → the next entry along the path: its id and
    /// whether it's a folder.
    memo: HashMap<(Oid, usize), Option<(Oid, bool)>>,
}

impl<'r> PathTrail<'r> {
    fn new(repo: &'r Repository, path: &str) -> Self {
        PathTrail { repo, parts: path.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect(), memo: HashMap::new() }
    }

    /// The id of the path's file or folder in the tree `root` (None: not there).
    fn id_in(&mut self, root: Oid) -> Result<Option<Oid>> {
        let mut id = root;
        for i in 0..self.parts.len() {
            let next = match self.memo.get(&(id, i)) {
                Some(v) => *v,
                None => {
                    let tree = self.repo.find_tree(id)?;
                    let v = tree.get_name(&self.parts[i]).map(|e| (e.id(), e.kind() == Some(git2::ObjectType::Tree)));
                    self.memo.insert((id, i), v);
                    v
                }
            };
            match next {
                None => return Ok(None),
                Some((child, _)) if i + 1 == self.parts.len() => return Ok(Some(child)),
                Some((child, true)) => id = child,
                Some((_, false)) => return Ok(None),
            }
        }
        Ok(Some(id))
    }
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
    let repo = super::open_for_reading(repo_path)?;
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
    // (Not where paths ignore case, as on macOS and Windows: "Src" finds "src" there.)
    let ignore_case = repo.config().and_then(|c| c.get_bool("core.ignorecase")).unwrap_or(false);
    let plain = path.as_deref().filter(|p| !ignore_case && !p.contains(['*', '?', '[']));
    let mut trail = plain.map(|p| PathTrail::new(&repo, p));
    // Only a path (or changed text) to look for: look it up in every commit,
    // a batch at a time on several threads, as for a file's history.
    let ahead = plain.filter(|_| text.is_none() && author.is_none());
    let ids: Vec<Oid> = walk.collect::<std::result::Result<_, _>>()?;
    let (mut batch, mut batch_from): (Vec<Lookup>, usize) = (vec![], 0);
    // Searching inside changes is slow on huge histories; give up after a while.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut rows = vec![];
    let mut scanned = 0;
    let mut more = false;
    for (k, &id) in ids.iter().enumerate() {
        if rows.len() >= limit || std::time::Instant::now() > deadline {
            more = true;
            break;
        }
        scanned += 1;
        let changed = match ahead {
            Some(p) => {
                if !(batch_from..batch_from + batch.len()).contains(&k) {
                    batch_from = k;
                    batch = lookups(repo_path, &ids[k..ids.len().min(k + LOOKUP_BATCH)], p, true)?;
                }
                let l = &batch[k - batch_from];
                if l.entry == l.parents.first().copied().flatten() {
                    continue;
                }
                true
            }
            None => false,
        };
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
            // A plain path (no wildcards; a file or a folder) is settled by
            // comparing its id with the parent's: equal means unchanged,
            // different means changed. That's all a search by path needs.
            if changed && contains.is_none() {
                push_row(&mut rows, &c, &labels);
                continue;
            }
            if let Some(t) = trail.as_mut().filter(|_| !changed) {
                let mine = t.id_in(c.tree_id())?;
                let theirs = match c.parent(0) {
                    Ok(p) => t.id_in(p.tree_id())?,
                    Err(_) => None,
                };
                if mine == theirs {
                    continue;
                }
                if contains.is_none() {
                    push_row(&mut rows, &c, &labels);
                    continue;
                }
            }
            let tree = c.tree()?;
            let parent = c.parent(0).ok().map(|p| p.tree()).transpose()?;
            let mut opts = git2::DiffOptions::new();
            if let Some(p) = &path {
                opts.pathspec(p);
            }
            let diff = if path.is_some() {
                repo.diff_tree_to_tree(parent.as_ref(), Some(&tree), Some(&mut opts))?
            } else {
                super::diff::diff_trees(&repo, parent.as_ref(), &tree, &mut opts)?
            };
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
        push_row(&mut rows, &c, &labels);
    }
    Ok(SearchResult { rows, scanned, more })
}

fn push_row(rows: &mut Vec<CommitRow>, c: &git2::Commit, labels: &HashMap<Oid, Vec<RefLabel>>) {
    let a = c.author();
    rows.push(CommitRow {
        oid: c.id().to_string(),
        short: short(c.id()),
        summary: super::summary(c),
        author: a.name().unwrap_or("").to_string(),
        email: a.email().unwrap_or("").to_string(),
        time: a.when().seconds(),
        parents: c.parent_ids().map(|p| p.to_string()).collect(),
        refs: labels.get(&c.id()).cloned().unwrap_or_default(),
        graph: GraphRow { col: 0, color: 0, width: 0, segs: vec![] },
    });
}
