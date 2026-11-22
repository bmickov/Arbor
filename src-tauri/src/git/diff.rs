//! Diffs for the working copy, the index and commits.

use super::{oid, open, Result};
use git2::{Delta, Diff, DiffFindOptions, DiffOptions, Patch, Repository};
use serde::{Deserialize, Serialize};

/// Diffs whose text is bigger than this are summarized instead of rendered.
/// It only guards against running the UI out of memory: a million lines of
/// typical code is around 40 MB.
const MAX_TEXT_BYTES: usize = 256 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    /// ' ' context, '+' added, '-' removed
    pub origin: char,
    pub content: String,
    /// False when the line has no trailing newline (end of file).
    pub nl: bool,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    /// The line ended in "\r" (CRLF), which `content` leaves out.
    #[serde(skip, default)]
    pub cr: bool,
    /// The exact bytes, for a line that isn't valid UTF-8 (where `content`
    /// has replacement characters), so patches built from it stay exact.
    #[serde(skip, default)]
    pub raw: Option<Vec<u8>>,
}

impl Line {
    /// The line as it is in the file, without its "\n".
    pub fn bytes(&self) -> Vec<u8> {
        let mut b = self.raw.clone().unwrap_or_else(|| self.content.as_bytes().to_vec());
        if self.cr {
            b.push(b'\r');
        }
        b
    }
}

#[derive(Clone, Debug)]
pub struct Hunk {
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<Line>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: &'static str,
    pub binary: bool,
    pub too_large: bool,
    /// Bigger than the caller's `max_lines`: lines left out until asked for.
    pub held_back: bool,
    pub additions: usize,
    pub deletions: usize,
    pub old_mode: u32,
    pub new_mode: u32,
    pub hunks: Vec<Hunk>,
}

/// Hunks go to the UI in a compact, column-wise form, because a diff can run
/// to millions of lines and one JSON object per line is several times larger
/// and slower to parse:
/// - `origins`: one character per line (' ', '+' or '-')
/// - `text`: the line contents joined with '\n'
/// - `noNl`: indexes of lines without a trailing newline
///
/// Line numbers aren't sent; the UI counts them up from `oldStart`/`newStart`.
impl Serialize for Hunk {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut origins = String::with_capacity(self.lines.len());
        let mut text = String::with_capacity(self.lines.iter().map(|l| l.content.len() + 1).sum());
        let mut no_nl = vec![];
        for (i, l) in self.lines.iter().enumerate() {
            origins.push(l.origin);
            if i > 0 {
                text.push('\n');
            }
            text.push_str(&l.content);
            if !l.nl {
                no_nl.push(i);
            }
        }
        let mut st = ser.serialize_struct("Hunk", 8)?;
        st.serialize_field("header", &self.header)?;
        st.serialize_field("oldStart", &self.old_start)?;
        st.serialize_field("oldLines", &self.old_lines)?;
        st.serialize_field("newStart", &self.new_start)?;
        st.serialize_field("newLines", &self.new_lines)?;
        st.serialize_field("origins", &origins)?;
        st.serialize_field("text", &text)?;
        st.serialize_field("noNl", &no_nl)?;
        st.end()
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiffSettings {
    pub context: Option<u32>,
    pub ignore_whitespace: Option<bool>,
    /// Leave the lines out (`held_back`) when more lines than this changed.
    /// None or 0 always includes them.
    pub max_lines: Option<usize>,
}

fn status_name(d: Delta) -> &'static str {
    match d {
        Delta::Added => "added",
        Delta::Deleted => "deleted",
        Delta::Renamed => "renamed",
        Delta::Copied => "copied",
        Delta::Typechange => "typechange",
        Delta::Untracked => "untracked",
        Delta::Conflicted => "conflicted",
        _ => "modified",
    }
}

pub fn options(s: DiffSettings) -> DiffOptions {
    let mut o = DiffOptions::new();
    o.context_lines(s.context.unwrap_or(3)).indent_heuristic(true);
    if s.ignore_whitespace.unwrap_or(false) {
        o.ignore_whitespace_change(true);
    }
    o
}

fn find_renames(diff: &mut Diff) -> Result<()> {
    let mut f = DiffFindOptions::new();
    f.renames(true).copies(false);
    diff.find_similar(Some(&mut f))?;
    Ok(())
}

/// Convert delta `idx` of `diff` into a FileDiff. With `with_lines` false only
/// the summary (paths, status, counts) is filled in; the same happens, with
/// `held_back` set, when more than `max_lines` lines changed.
pub fn file_diff(diff: &Diff, idx: usize, with_lines: bool, max_lines: Option<usize>) -> Result<FileDiff> {
    let delta = diff.get_delta(idx).expect("delta index in range");
    let new_path = delta.new_file().path().map(|p| p.to_string_lossy().to_string());
    let old_path = delta.old_file().path().map(|p| p.to_string_lossy().to_string());
    let path = new_path.clone().or(old_path.clone()).unwrap_or_default();
    let mut fd = FileDiff {
        path: path.clone(),
        old_path: old_path.filter(|o| *o != path),
        status: status_name(delta.status()),
        binary: false,
        too_large: false,
        held_back: false,
        additions: 0,
        deletions: 0,
        old_mode: u32::from(delta.old_file().mode()),
        new_mode: u32::from(delta.new_file().mode()),
        hunks: vec![],
    };
    let Some(patch) = Patch::from_diff(diff, idx)? else {
        fd.binary = true;
        return Ok(fd);
    };
    if patch.delta().flags().is_binary() {
        fd.binary = true;
        return Ok(fd);
    }
    let (_, adds, dels) = patch.line_stats()?;
    fd.additions = adds;
    fd.deletions = dels;
    if !with_lines {
        return Ok(fd);
    }
    if max_lines.is_some_and(|m| m > 0 && adds + dels > m) {
        fd.held_back = true;
        return Ok(fd);
    }
    if patch.size(true, false, false) > MAX_TEXT_BYTES {
        fd.too_large = true;
        return Ok(fd);
    }
    for h in 0..patch.num_hunks() {
        let (hunk, n) = patch.hunk(h)?;
        let mut lines: Vec<Line> = Vec::with_capacity(n);
        for l in 0..n {
            let line = patch.line_in_hunk(h, l)?;
            let origin = line.origin();
            match origin {
                ' ' | '+' | '-' => {
                    let mut bytes = line.content();
                    let nl = bytes.last() == Some(&b'\n');
                    if nl {
                        bytes = &bytes[..bytes.len() - 1];
                    }
                    let cr = bytes.last() == Some(&b'\r');
                    if cr {
                        bytes = &bytes[..bytes.len() - 1];
                    }
                    let (content, raw) = match std::str::from_utf8(bytes) {
                        Ok(t) => (t.to_string(), None),
                        Err(_) => (String::from_utf8_lossy(bytes).into_owned(), Some(bytes.to_vec())),
                    };
                    lines.push(Line { origin, content, nl, old_no: line.old_lineno(), new_no: line.new_lineno(), cr, raw });
                }
                // "\ No newline at end of file" markers: already captured by `nl`.
                _ => {}
            }
        }
        fd.hunks.push(Hunk {
            header: String::from_utf8_lossy(hunk.header()).trim_end().to_string(),
            old_start: hunk.old_start(),
            old_lines: hunk.old_lines(),
            new_start: hunk.new_start(),
            new_lines: hunk.new_lines(),
            lines,
        });
    }
    Ok(fd)
}

fn pick(diff: &Diff, path: &str, max_lines: Option<usize>) -> Result<Option<FileDiff>> {
    for i in 0..diff.deltas().len() {
        let d = diff.get_delta(i).unwrap();
        let np = d.new_file().path().map(|p| p.to_string_lossy().to_string());
        let op = d.old_file().path().map(|p| p.to_string_lossy().to_string());
        if np.as_deref() == Some(path) || (np.is_none() && op.as_deref() == Some(path)) {
            return Ok(Some(file_diff(diff, i, true, max_lines)?));
        }
    }
    Ok(None)
}

/// Raw index→workdir diff for one path (untracked files included).
pub fn raw_unstaged<'a>(repo: &'a Repository, path: &str, s: DiffSettings) -> Result<Diff<'a>> {
    let mut o = options(s);
    o.include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true)
        .disable_pathspec_match(true)
        .pathspec(path);
    Ok(repo.diff_index_to_workdir(None, Some(&mut o))?)
}

/// Raw HEAD→index diff for one path (and its rename source, when given).
pub fn raw_staged<'a>(repo: &'a Repository, path: &str, old_path: Option<&str>, s: DiffSettings) -> Result<Diff<'a>> {
    let mut o = options(s);
    o.disable_pathspec_match(true).pathspec(path);
    if let Some(op) = old_path {
        o.pathspec(op);
    }
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let mut d = repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut o))?;
    if old_path.is_some() {
        find_renames(&mut d)?;
    }
    Ok(d)
}

pub fn unstaged_file(repo_path: &str, path: &str, s: DiffSettings) -> Result<Option<FileDiff>> {
    let repo = open(repo_path)?;
    let d = raw_unstaged(&repo, path, s)?;
    pick(&d, path, s.max_lines)
}

pub fn staged_file(repo_path: &str, path: &str, old_path: Option<&str>, s: DiffSettings) -> Result<Option<FileDiff>> {
    let repo = open(repo_path)?;
    let d = raw_staged(&repo, path, old_path, s)?;
    pick(&d, path, s.max_lines)
}

/// Diffs that make up a commit: against its first parent (or nothing for a
/// root commit). For stashes, untracked files from the third parent are added.
/// A commit's changes against its first parent (a stash also gets its
/// untracked files). `paths` limits them to those files, which is much
/// faster in a big tree; empty means every file.
pub(crate) fn commit_diffs<'a>(repo: &'a Repository, id: &str, s: DiffSettings, paths: &[&str]) -> Result<Vec<Diff<'a>>> {
    let c = repo.find_commit(oid(id)?)?;
    let tree = c.tree()?;
    let parent_tree = if c.parent_count() > 0 { Some(c.parent(0)?.tree()?) } else { None };
    let opts = || {
        let mut o = options(s);
        if !paths.is_empty() {
            o.disable_pathspec_match(true);
            for p in paths {
                o.pathspec(p);
            }
        }
        o
    };
    let mut d = if paths.is_empty() {
        diff_trees(repo, parent_tree.as_ref(), &tree, &mut options(s))?
    } else {
        repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts()))?
    };
    find_renames(&mut d)?;
    let mut out = vec![d];
    let is_stash = c.parent_count() == 3
        && c.message().map(|m| m.starts_with("WIP on") || m.starts_with("On ")).unwrap_or(false);
    if is_stash {
        let untracked = c.parent(2)?.tree()?;
        out.push(repo.diff_tree_to_tree(None, Some(&untracked), Some(&mut opts()))?);
    }
    Ok(out)
}

/// Compare two trees, like `diff_tree_to_tree`, but quickly in a big
/// project: libgit2 reads every file entry of both trees (tens of thousands)
/// even where whole folders are the same. The changed files are found first,
/// skipping identical folders, and only those are compared; the result is
/// the same. `o` must not have a pathspec of its own.
pub(crate) fn diff_trees<'a>(repo: &'a Repository, old: Option<&git2::Tree>, new: &git2::Tree, o: &mut DiffOptions) -> Result<Diff<'a>> {
    if let Some(list) = changed_paths(repo, old, new, 2000)?.filter(|l| !l.is_empty()) {
        o.disable_pathspec_match(true);
        for p in &list {
            o.pathspec(p);
        }
    }
    Ok(repo.diff_tree_to_tree(old, Some(new), Some(o))?)
}

/// The files that differ between two trees, without looking inside
/// folders that are the same on both sides. None when there are more than
/// `limit` (or a name isn't UTF-8): then comparing everything is as quick.
pub(crate) fn changed_paths(repo: &Repository, old: Option<&git2::Tree>, new: &git2::Tree, limit: usize) -> Result<Option<Vec<String>>> {
    let mut out = vec![];
    Ok(walk_changed(repo, old, Some(new), "", &mut out, limit)?.then_some(out))
}

fn walk_changed(repo: &Repository, old: Option<&git2::Tree>, new: Option<&git2::Tree>, prefix: &str, out: &mut Vec<String>, limit: usize) -> Result<bool> {
    type Entry = (git2::Oid, i32);
    let mut names: std::collections::BTreeMap<Vec<u8>, (Option<Entry>, Option<Entry>)> = Default::default();
    for e in old.into_iter().flat_map(|t| t.iter()) {
        names.entry(e.name_bytes().to_vec()).or_default().0 = Some((e.id(), e.filemode()));
    }
    for e in new.into_iter().flat_map(|t| t.iter()) {
        names.entry(e.name_bytes().to_vec()).or_default().1 = Some((e.id(), e.filemode()));
    }
    const TREE: i32 = 0o040000;
    for (name, (o, n)) in names {
        if o == n {
            continue;
        }
        let Ok(name) = String::from_utf8(name) else { return Ok(false) };
        let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let tree_of = |e: Option<Entry>| e.filter(|(_, m)| *m == TREE).map(|(id, _)| repo.find_tree(id)).transpose();
        let (ot, nt) = (tree_of(o)?, tree_of(n)?);
        if (ot.is_some() || nt.is_some()) && !walk_changed(repo, ot.as_ref(), nt.as_ref(), &path, out, limit)? {
            return Ok(false);
        }
        // A file on either side (also where a folder replaced a file).
        if o.is_some_and(|(_, m)| m != TREE) || n.is_some_and(|(_, m)| m != TREE) {
            out.push(path);
            if out.len() > limit {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Summaries (no lines) of every file in `diffs`, each path once.
pub(crate) fn summaries(diffs: &[Diff]) -> Result<Vec<FileDiff>> {
    let mut out = vec![];
    let mut seen = std::collections::HashSet::new();
    for d in diffs {
        for i in 0..d.deltas().len() {
            let f = file_diff(d, i, false, None)?;
            if seen.insert(f.path.clone()) {
                out.push(f);
            }
        }
    }
    Ok(out)
}

/// The first of `diffs` to have `path`, with its lines.
pub(crate) fn pick_in(diffs: &[Diff], path: &str, max_lines: Option<usize>) -> Result<Option<FileDiff>> {
    for d in diffs {
        if let Some(f) = pick(d, path, max_lines)? {
            return Ok(Some(f));
        }
    }
    Ok(None)
}

/// Summaries (no lines) of every file changed by a commit.
pub fn commit_files(repo_path: &str, id: &str) -> Result<Vec<FileDiff>> {
    let repo = super::open_for_reading(repo_path)?;
    let diffs = commit_diffs(&repo, id, DiffSettings::default(), &[])?;
    summaries(&diffs)
}

/// One file's changes in a commit. `listed`: `path` and `old_path` come
/// from the commit's file list, so (renames included) only those two files
/// need comparing, not the whole tree. Otherwise (a path from elsewhere,
/// which may have been renamed in this commit) the whole commit is.
pub fn commit_file(repo_path: &str, id: &str, path: &str, s: DiffSettings) -> Result<Option<FileDiff>> {
    commit_file_listed(repo_path, id, path, None, false, s)
}

pub fn commit_file_listed(repo_path: &str, id: &str, path: &str, old_path: Option<&str>, listed: bool, s: DiffSettings) -> Result<Option<FileDiff>> {
    let repo = super::open_for_reading(repo_path)?;
    let mut paths = vec![];
    if listed {
        paths.push(path);
        paths.extend(old_path);
    }
    let diffs = commit_diffs(&repo, id, s, &paths)?;
    pick_in(&diffs, path, s.max_lines)
}

/// Content of a file at a revision (or the working tree when `rev` is None),
/// used for image previews and the full-file view.
pub fn file_at(repo_path: &str, rev: Option<&str>, path: &str) -> Result<Option<Vec<u8>>> {
    let repo = open(repo_path)?;
    match rev {
        None => {
            let Some(root) = repo.workdir() else { return Ok(None) };
            let full = super::in_workdir(root, path)?;
            Ok(std::fs::read(full).ok())
        }
        Some(r) => {
            let obj = repo.revparse_single(r)?;
            let tree = obj.peel_to_tree()?;
            match tree.get_path(std::path::Path::new(path)) {
                Ok(e) => Ok(Some(repo.find_blob(e.id())?.content().to_vec())),
                Err(_) => Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
    }

    fn write(dir: &std::path::Path, name: &str, text: &str) {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// (status, old path, new path) of each change, as libgit2 finds them comparing everything.
    fn whole(repo: &Repository, id: &str) -> Vec<(String, String, String)> {
        let c = repo.find_commit(oid(id).unwrap()).unwrap();
        let pt = c.parent(0).ok().map(|p| p.tree().unwrap());
        let mut d = repo.diff_tree_to_tree(pt.as_ref(), Some(&c.tree().unwrap()), Some(&mut options(DiffSettings::default()))).unwrap();
        find_renames(&mut d).unwrap();
        list(&d)
    }

    fn list(d: &Diff) -> Vec<(String, String, String)> {
        d.deltas()
            .map(|x| {
                let p = |f: git2::DiffFile| f.path().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                (status_name(x.status()).to_string(), p(x.old_file()), p(x.new_file()))
            })
            .collect()
    }

    #[test]
    fn comparing_only_changed_files_finds_the_same_changes() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        git(d, &["init", "-q"]);
        let long: String = (1..60).map(|i| format!("line {i}\n")).collect();
        write(d, "a.txt", &long);
        write(d, "dir/x.txt", "x\n");
        write(d, "dir/sub/y.txt", "y\n");
        write(d, "dir/sub/deep/z.txt", "z\n");
        for i in 0..40 {
            write(d, &format!("keep/k{i}/f.txt"), &format!("{i}\n"));
        }
        write(d, "run.sh", "echo hi\n");
        write(d, "t", "a file that becomes a folder\n");
        write(d, "same name.txt", "spaces and [brackets]*\n");
        git(d, &["add", "-A"]);
        git(d, &["commit", "-qm", "base"]);

        std::fs::remove_file(d.join("a.txt")).unwrap();
        write(d, "moved/a.txt", &long.replace("line 30\n", "line thirty\n"));
        write(d, "dir/x.txt", "x changed\n");
        git(d, &["rm", "-rq", "dir/sub"]);
        git(d, &["update-index", "--chmod=+x", "run.sh"]);
        git(d, &["rm", "-q", "t"]);
        write(d, "t/inside.txt", "now a folder\n");
        write(d, "new/n1.txt", "n1\n");
        write(d, "new/deeper/n2.txt", "n2\n");
        write(d, "same name.txt", "changed\n");
        git(d, &["add", "-A"]);
        git(d, &["commit", "-qm", "changes"]);

        let repo = Repository::open(d).unwrap();
        let head = repo.head().unwrap().target().unwrap().to_string();
        let mut want = whole(&repo, &head);
        let diffs = commit_diffs(&repo, &head, DiffSettings::default(), &[]).unwrap();
        let mut got = list(&diffs[0]);
        want.sort();
        got.sort();
        assert_eq!(got, want);
        assert!(got.iter().any(|(s, o, n)| s == "renamed" && o == "a.txt" && n == "moved/a.txt"));
        assert!(!got.iter().any(|(_, _, n)| n.starts_with("keep/")));

        // The first commit (no parent) too.
        let first = repo.find_commit(oid(&head).unwrap()).unwrap().parent_id(0).unwrap().to_string();
        let mut want = whole(&repo, &first);
        let mut got = list(&commit_diffs(&repo, &first, DiffSettings::default(), &[]).unwrap()[0]);
        want.sort();
        got.sort();
        assert_eq!(got, want);
        assert_eq!(got.len(), 47);

        // Past the limit, it gives up (and everything is compared instead).
        let c = repo.find_commit(oid(&first).unwrap()).unwrap();
        assert!(changed_paths(&repo, None, &c.tree().unwrap(), 10).unwrap().is_none());
    }
}
