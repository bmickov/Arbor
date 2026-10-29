//! End-to-end tests of the git backend against throwaway repositories.
//! The `git` CLI is used to set up fixtures and to check results independently.

use arbor_lib::git::{self, diff::DiffSettings, patch::Selection, rebase::Step};
use std::path::Path;
use std::process::Command;

struct Repo {
    _dir: tempfile::TempDir,
    path: String,
}

/// Keep tests out of the real ~/.config/arbor.
fn isolate_config() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let d = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    unsafe { std::env::set_var("ARBOR_CONFIG_DIR", d.path()) };
}

impl Repo {
    fn new() -> Repo {
        isolate_config();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let r = Repo { _dir: dir, path };
        r.git(&["init", "-q", "-b", "main"]);
        r.git(&["config", "user.name", "Test"]);
        r.git(&["config", "user.email", "test@example.com"]);
        r.git(&["config", "commit.gpgsign", "false"]);
        r
    }
    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git").args(args).current_dir(&self.path).output().unwrap();
        assert!(out.status.success(), "git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).to_string()
    }
    fn write(&self, name: &str, content: &str) {
        let p = Path::new(&self.path).join(name);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }
    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(Path::new(&self.path).join(name)).unwrap()
    }
    fn commit_all(&self, msg: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", msg]);
    }
    fn staged_content(&self, name: &str) -> String {
        self.git(&["show", &format!(":{name}")])
    }
    fn subjects(&self) -> Vec<String> {
        self.git(&["log", "--format=%s"]).lines().map(|s| s.to_string()).collect()
    }
}

fn lines(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

fn s() -> DiffSettings {
    DiffSettings::default()
}

/// Indexes of the changed lines in hunk `h` whose content contains `needle`.
fn pick(fd: &git::diff::FileDiff, h: usize, needles: &[&str]) -> Selection {
    let lines = fd.hunks[h]
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.origin != ' ' && needles.iter().any(|n| l.content.contains(n)))
        .map(|(i, _)| i)
        .collect();
    Selection { hunk: h, lines }
}

#[test]
fn stage_single_lines_from_two_hunks() {
    let r = Repo::new();
    r.write("a.txt", &lines(30));
    r.commit_all("init");
    let mut content = lines(30).replace("line 3\n", "line 3 changed\n").replace("line 25\n", "line 25 changed\n");
    content = content.replace("line 10\n", "line 10\nnew after 10\n");
    content.push_str("tail\n");
    r.write("a.txt", &content);

    let fd = git::diff::unstaged_file(&r.path, "a.txt", s()).unwrap().unwrap();
    assert_eq!(fd.hunks.len(), 3, "hunks: {:#?}", fd.hunks.iter().map(|h| &h.header).collect::<Vec<_>>());
    // Stage only "new after 10" (hunk 1) and the "25 changed" replacement (both lines, hunk 2).
    let sel = vec![pick(&fd, 1, &["new after 10"]), pick(&fd, 2, &["line 25"])];
    git::patch::stage_lines(&r.path, "a.txt", &sel, s()).unwrap();

    let staged = r.staged_content("a.txt");
    assert!(staged.contains("new after 10"));
    assert!(staged.contains("line 25 changed"));
    assert!(!staged.contains("line 3 changed"), "line 3 must stay unstaged");
    assert!(!staged.contains("tail"), "tail must stay unstaged");
    // Working tree untouched.
    assert_eq!(r.read("a.txt"), content);
}

#[test]
fn stage_only_the_addition_half_of_a_replacement() {
    let r = Repo::new();
    r.write("a.txt", &lines(10));
    r.commit_all("init");
    r.write("a.txt", &lines(10).replace("line 5\n", "five\n"));
    let fd = git::diff::unstaged_file(&r.path, "a.txt", s()).unwrap().unwrap();
    // Select only "+five": the "-line 5" stays as context, so both exist.
    git::patch::stage_lines(&r.path, "a.txt", &[pick(&fd, 0, &["five"])], s()).unwrap();
    let staged = r.staged_content("a.txt");
    assert!(staged.contains("line 5\nfive\n"), "staged: {staged}");
}

#[test]
fn unstage_and_discard_lines() {
    let r = Repo::new();
    r.write("a.txt", &lines(20));
    r.commit_all("init");
    let changed = lines(20).replace("line 2\n", "two\n").replace("line 18\n", "eighteen\n");
    r.write("a.txt", &changed);
    r.git(&["add", "a.txt"]);

    let fd = git::diff::staged_file(&r.path, "a.txt", None, s()).unwrap().unwrap();
    assert_eq!(fd.hunks.len(), 2);
    git::patch::unstage_lines(&r.path, "a.txt", None, &[pick(&fd, 1, &["18", "eighteen"])], s()).unwrap();
    let staged = r.staged_content("a.txt");
    assert!(staged.contains("two") && staged.contains("line 18") && !staged.contains("eighteen"));

    // Now discard the unstaged "eighteen" change from the working tree.
    let fd = git::diff::unstaged_file(&r.path, "a.txt", s()).unwrap().unwrap();
    assert_eq!(fd.hunks.len(), 1);
    // Discard just the + line, keeping the - line: result restores "line 18".
    git::patch::discard_lines(&r.path, "a.txt", &[pick(&fd, 0, &["eighteen", "line 18"])], s()).unwrap();
    assert_eq!(r.read("a.txt"), lines(20).replace("line 2\n", "two\n"));

    // Undo the discard brings "eighteen" back.
    let label = git::undo::undo(&r.path).unwrap();
    assert!(label.starts_with("Discard"), "{label}");
    assert!(r.read("a.txt").contains("eighteen"));
}

#[test]
fn partial_stage_of_untracked_file() {
    let r = Repo::new();
    r.write("base.txt", "x\n");
    r.commit_all("init");
    r.write("new.txt", "keep\nskip\nkeep2\n");
    let fd = git::diff::unstaged_file(&r.path, "new.txt", s()).unwrap().unwrap();
    git::patch::stage_lines(&r.path, "new.txt", &[pick(&fd, 0, &["keep"])], s()).unwrap();
    assert_eq!(r.staged_content("new.txt"), "keep\nkeep2\n");
}

#[test]
fn no_newline_at_end_of_file() {
    let r = Repo::new();
    r.write("a.txt", "one\ntwo");
    r.commit_all("init");
    r.write("a.txt", "one\ntwo\nthree");
    let fd = git::diff::unstaged_file(&r.path, "a.txt", s()).unwrap().unwrap();
    let all: Vec<usize> = (0..fd.hunks[0].lines.len()).collect();
    git::patch::stage_lines(&r.path, "a.txt", &[Selection { hunk: 0, lines: all }], s()).unwrap();
    assert_eq!(r.staged_content("a.txt"), "one\ntwo\nthree");
}

#[test]
fn commit_and_undo_commit_keeps_changes_staged() {
    let r = Repo::new();
    r.write("a.txt", "a\n");
    r.commit_all("init");
    r.write("a.txt", "b\n");
    git::patch::stage_files(&r.path, &["a.txt".into()]).unwrap();
    git::commit::commit(&r.path, "second", false, false).unwrap();
    assert_eq!(r.subjects(), vec!["second", "init"]);
    let label = git::undo::undo(&r.path).unwrap();
    assert_eq!(label, "Commit second");
    assert_eq!(r.subjects(), vec!["init"]);
    assert_eq!(r.staged_content("a.txt"), "b\n");
    // The stage before it is undoable too.
    git::undo::undo(&r.path).unwrap();
    assert_eq!(r.staged_content("a.txt"), "a\n");
}

#[test]
fn first_commit_can_be_undone() {
    let r = Repo::new();
    r.write("a.txt", "a\n");
    git::patch::stage_all(&r.path).unwrap();
    git::commit::commit(&r.path, "root", false, false).unwrap();
    git::undo::undo(&r.path).unwrap();
    let st = git::status::status(&r.path).unwrap();
    assert!(st.head.is_none());
    assert_eq!(st.staged.len(), 1);
}

#[test]
fn amend_and_undo() {
    let r = Repo::new();
    r.write("a.txt", "a\n");
    r.commit_all("init");
    r.write("b.txt", "b\n");
    r.commit_all("tpyo");
    git::commit::commit(&r.path, "typo fixed", true, false).unwrap();
    assert_eq!(r.subjects(), vec!["typo fixed", "init"]);
    git::undo::undo(&r.path).unwrap();
    assert_eq!(r.subjects(), vec!["tpyo", "init"]);
}

#[test]
fn hooks_run_and_can_block() {
    let r = Repo::new();
    r.write("a.txt", "a\n");
    r.commit_all("init");
    let hook = Path::new(&r.path).join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho 'lint failed' >&2\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    r.write("a.txt", "b\n");
    git::patch::stage_all(&r.path).unwrap();
    let e = git::commit::commit(&r.path, "x", false, false).unwrap_err().to_string();
    assert!(e.contains("lint failed"), "{e}");
    // Skipping hooks works.
    git::commit::commit(&r.path, "x", false, true).unwrap();
}

#[test]
fn interactive_rebase_reorder_squash_reword_drop() {
    let r = Repo::new();
    r.write("base.txt", "base\n");
    r.commit_all("base");
    for n in ["a", "b", "c", "d"] {
        r.write(&format!("{n}.txt"), &format!("{n}\n"));
        r.commit_all(n);
    }
    let ids: Vec<String> = r.git(&["log", "--format=%H", "--reverse"]).lines().map(|s| s.to_string()).collect();
    let (base, a, b, c, d) = (&ids[0], &ids[1], &ids[2], &ids[3], &ids[4]);
    let step = |o: &str, act: &str, m: Option<&str>| Step { oid: o.into(), action: act.into(), message: m.map(|s| s.into()) };
    let steps = vec![
        step(c, "pick", None),
        step(a, "reword", Some("A renamed")),
        step(b, "squash", None),
        step(d, "drop", None),
    ];
    git::rebase::interactive(&r.path, Some(base), &steps, "Rewrite history").unwrap();
    assert_eq!(r.subjects(), vec!["A renamed", "c", "base"]);
    let tip_msg = r.git(&["log", "-1", "--format=%B"]);
    assert!(tip_msg.contains("A renamed") && tip_msg.contains("\nb"), "{tip_msg}");
    assert!(!Path::new(&r.path).join("d.txt").exists(), "dropped commit's file is gone");
    assert!(Path::new(&r.path).join("b.txt").exists());
    git::undo::undo(&r.path).unwrap();
    assert_eq!(r.subjects(), vec!["d", "c", "b", "a", "base"]);
    assert!(Path::new(&r.path).join("d.txt").exists());
}

#[test]
fn interactive_rebase_conflict_changes_nothing() {
    let r = Repo::new();
    r.write("f.txt", "1\n");
    r.commit_all("one");
    r.write("f.txt", "2\n");
    r.commit_all("two");
    r.write("f.txt", "3\n");
    r.commit_all("three");
    let ids: Vec<String> = r.git(&["log", "--format=%H", "--reverse"]).lines().map(|s| s.to_string()).collect();
    let before = r.git(&["rev-parse", "HEAD"]);
    let steps = vec![
        Step { oid: ids[2].clone(), action: "pick".into(), message: None },
        Step { oid: ids[1].clone(), action: "pick".into(), message: None },
    ];
    let e = git::rebase::interactive(&r.path, Some(&ids[0]), &steps, "Reorder").unwrap_err().to_string();
    assert!(e.contains("conflicts"), "{e}");
    assert_eq!(r.git(&["rev-parse", "HEAD"]), before);
}

#[test]
fn merge_fast_forward_normal_and_conflict() {
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("g.txt", "g\n");
    r.commit_all("feature work");
    r.git(&["checkout", "-q", "main"]);
    let m = git::merge::merge(&r.path, "feature", false).unwrap();
    assert_eq!(m.result, "fast-forward");
    git::undo::undo(&r.path).unwrap();
    assert_eq!(r.subjects(), vec!["base"]);

    r.write("h.txt", "h\n");
    r.commit_all("main work");
    let m = git::merge::merge(&r.path, "feature", false).unwrap();
    assert_eq!(m.result, "merged");
    assert_eq!(r.git(&["rev-list", "--parents", "-1", "HEAD"]).split_whitespace().count(), 3);
    assert_eq!(r.subjects()[0], "Merge branch 'feature'");

    // Conflict, then abort through undo.
    r.git(&["checkout", "-q", "-b", "c1"]);
    r.write("f.txt", "ours\n");
    r.commit_all("ours");
    r.git(&["checkout", "-q", "-b", "c2", "HEAD~1"]);
    r.write("f.txt", "theirs\n");
    r.commit_all("theirs");
    let m = git::merge::merge(&r.path, "c1", false).unwrap();
    assert_eq!(m.result, "conflicts");
    assert_eq!(m.conflicts, vec!["f.txt"]);
    let st = git::status::status(&r.path).unwrap();
    assert_eq!(st.state, "merge");
    assert_eq!(st.conflicted.len(), 1);
    git::patch::resolve_conflict(&r.path, "f.txt", "theirs").unwrap();
    assert_eq!(r.read("f.txt"), "ours\n");
    git::undo::undo(&r.path).unwrap();
    let st = git::status::status(&r.path).unwrap();
    assert_eq!(st.state, "clean");
    assert_eq!(r.read("f.txt"), "theirs\n");
}

#[test]
fn rebase_onto_branch() {
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("g.txt", "g\n");
    r.commit_all("feature 1");
    r.write("g.txt", "g2\n");
    r.commit_all("feature 2");
    r.git(&["checkout", "-q", "main"]);
    r.write("h.txt", "h\n");
    r.commit_all("main 1");
    let out = git::rebase::rebase_branch(&r.path, "feature", "main").unwrap();
    assert_eq!(out.result, "rebased");
    assert_eq!(r.subjects(), vec!["feature 2", "feature 1", "main 1", "base"]);
    assert_eq!(r.git(&["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "feature");
}

#[test]
fn stash_save_pop_drop_and_undo() {
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    r.write("f.txt", "wip\n");
    r.write("new.txt", "untracked\n");
    git::stash::save(&r.path, Some("my work"), true, false, false).unwrap();
    assert_eq!(r.read("f.txt"), "base\n");
    assert!(!Path::new(&r.path).join("new.txt").exists());
    let list = git::stash::list(&r.path).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].message, "my work");
    assert!(list[0].has_untracked);
    let files = git::diff::commit_files(&r.path, &list[0].oid).unwrap();
    assert!(files.iter().any(|f| f.path == "new.txt"), "untracked file listed in stash");

    // Undo "stash" brings the changes back.
    git::undo::undo(&r.path).unwrap();
    assert_eq!(r.read("f.txt"), "wip\n");
    assert!(git::stash::list(&r.path).unwrap().is_empty());

    git::stash::save(&r.path, Some("again"), true, false, false).unwrap();
    git::stash::drop(&r.path, 0).unwrap();
    assert!(git::stash::list(&r.path).unwrap().is_empty());
    git::undo::undo(&r.path).unwrap();
    let list = git::stash::list(&r.path).unwrap();
    assert_eq!(list.len(), 1, "dropped stash restored");
    assert_eq!(r.git(&["stash", "list"]).lines().count(), 1, "git sees it too");

    git::stash::pop(&r.path, 0, false).unwrap();
    assert_eq!(r.read("f.txt"), "wip\n");
    git::undo::undo(&r.path).unwrap();
    assert_eq!(r.read("f.txt"), "base\n");
    assert!(!Path::new(&r.path).join("new.txt").exists());
    assert_eq!(git::stash::list(&r.path).unwrap().len(), 1);
}

#[test]
fn branches_create_delete_rename_undo() {
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    git::branches::create_branch(&r.path, "topic", None, true).unwrap();
    assert_eq!(r.git(&["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "topic");
    r.write("t.txt", "t\n");
    r.commit_all("topic work");
    git::branches::checkout_branch(&r.path, "main").unwrap();
    let e = git::branches::delete_branch(&r.path, "topic", false).unwrap_err().to_string();
    assert!(e.starts_with("NOT_MERGED:"), "{e}");
    git::branches::delete_branch(&r.path, "topic", true).unwrap();
    git::undo::undo(&r.path).unwrap();
    assert!(r.git(&["branch"]).contains("topic"));
    git::branches::rename_branch(&r.path, "topic", "renamed").unwrap();
    git::undo::undo(&r.path).unwrap();
    assert!(r.git(&["branch"]).contains("topic"));
    let refs = git::branches::refs(&r.path).unwrap();
    assert_eq!(refs.local.len(), 2);
    assert!(git::branches::create_branch(&r.path, "bad name", None, false).is_err());
}

#[test]
fn discard_untracked_and_undo() {
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    r.write("junk.txt", "precious\n");
    r.write("f.txt", "edited\n");
    git::patch::discard_files(&r.path, &["junk.txt".into(), "f.txt".into()]).unwrap();
    assert!(!Path::new(&r.path).join("junk.txt").exists());
    assert_eq!(r.read("f.txt"), "base\n");
    git::undo::undo(&r.path).unwrap();
    assert_eq!(r.read("junk.txt"), "precious\n");
    assert_eq!(r.read("f.txt"), "edited\n");
}

#[test]
fn cherry_pick_and_revert() {
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "other"]);
    r.write("o.txt", "o\n");
    r.commit_all("other change");
    let id = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    r.git(&["checkout", "-q", "main"]);
    git::commit::cherry_pick(&r.path, &id).unwrap();
    assert_eq!(r.subjects()[0], "other change");
    assert!(Path::new(&r.path).join("o.txt").exists());
    let head = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    git::commit::revert(&r.path, &head).unwrap();
    assert!(!Path::new(&r.path).join("o.txt").exists());
    assert!(r.subjects()[0].starts_with("Revert"));
}

#[test]
fn graph_lanes_for_a_merge() {
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("g.txt", "g\n");
    r.commit_all("feature");
    r.git(&["checkout", "-q", "main"]);
    r.write("h.txt", "h\n");
    r.commit_all("main");
    r.git(&["merge", "-q", "--no-edit", "feature"]);
    let log = git::log::log(&r.path, "all", 100).unwrap();
    assert_eq!(log.rows.len(), 4);
    let merge = &log.rows[0];
    assert_eq!(merge.parents.len(), 2);
    assert!(merge.refs.iter().any(|l| l.kind == "head" && l.name == "main"));
    // The merge opens a second lane, which closes again at the base commit.
    assert_eq!(merge.graph.segs.iter().filter(|s| s[2] == 2).count(), 2);
    assert!(log.rows.iter().any(|row| row.graph.col == 1));
    let base = log.rows.last().unwrap();
    assert_eq!(base.graph.col, 0);
    // Somewhere above the base, lane 1 bends back into lane 0.
    assert!(log.rows.iter().any(|row| row.graph.segs.iter().any(|s| s[0] == 1 && s[1] == 0)));
}

#[test]
fn file_history_follows_rename() {
    let r = Repo::new();
    r.write("old.txt", &lines(30));
    r.commit_all("create");
    r.write("old.txt", &lines(31));
    r.commit_all("edit");
    r.git(&["mv", "old.txt", "new.txt"]);
    r.commit_all("rename");
    let h = git::log::file_history(&r.path, "new.txt", 50).unwrap();
    let subjects: Vec<_> = h.iter().map(|e| e.commit.summary.as_str()).collect();
    assert_eq!(subjects, vec!["rename", "edit", "create"]);
    assert_eq!(h[2].path, "old.txt");
    let b = git::blame::blame(&r.path, "new.txt").unwrap();
    assert_eq!(b.lines.len(), 31);
    assert_eq!(b.hunks.iter().map(|h| h.count).sum::<usize>(), 31);
}

#[test]
fn push_and_pull_over_local_remote() {
    let remote = Repo::new();
    remote.git(&["config", "receive.denyCurrentBranch", "ignore"]);
    let bare = tempfile::tempdir().unwrap();
    let bare_path = bare.path().to_string_lossy().to_string();
    Command::new("git").args(["init", "-q", "--bare", "-b", "main", &bare_path]).output().unwrap();

    let a = Repo::new();
    a.write("f.txt", "1\n");
    a.commit_all("one");
    git::branches::add_remote(&a.path, "origin", &bare_path).unwrap();
    let pushed = git::remote::push(&a.path, None, None, false, true, None, None).unwrap();
    assert_eq!(pushed, "origin/main");
    let st = git::status::status(&a.path).unwrap();
    assert_eq!(st.upstream.as_deref(), Some("origin/main"));

    let dest = tempfile::tempdir().unwrap();
    let b_path = dest.path().join("b").to_string_lossy().to_string();
    git::repo::clone_repo(&bare_path, &b_path, None, None).unwrap();
    Command::new("git").args(["config", "user.name", "B"]).current_dir(&b_path).output().unwrap();
    Command::new("git").args(["config", "user.email", "b@example.com"]).current_dir(&b_path).output().unwrap();

    a.write("f.txt", "2\n");
    a.commit_all("two");
    git::remote::push(&a.path, None, None, false, false, None, None).unwrap();

    let p = git::remote::pull(&b_path, false, None, None).unwrap();
    assert_eq!(p.result, "fast-forward");
    assert_eq!(std::fs::read_to_string(Path::new(&b_path).join("f.txt")).unwrap(), "2\n");

    // Diverge and get a rejected push.
    std::fs::write(Path::new(&b_path).join("g.txt"), "g\n").unwrap();
    git::patch::stage_all(&b_path).unwrap();
    git::commit::commit(&b_path, "from b", false, false).unwrap();
    a.write("f.txt", "3\n");
    a.commit_all("three");
    git::remote::push(&a.path, None, None, false, false, None, None).unwrap();
    let e = git::remote::push(&b_path, None, None, false, false, None, None).unwrap_err().to_string();
    assert!(e.starts_with("REJECTED:"), "{e}");
    let p = git::remote::pull(&b_path, true, None, None).unwrap();
    assert_eq!(p.result, "rebased");
    git::remote::push(&b_path, None, None, false, false, None, None).unwrap();
}

#[test]
fn library_scan_and_summaries() {
    isolate_config();
    let root = tempfile::tempdir().unwrap();
    let base = root.path();
    for (dir, dirty) in [("a", false), ("nested/b", true), ("nested/deeper/c", false)] {
        let p = base.join(dir);
        std::fs::create_dir_all(&p).unwrap();
        let run = |args: &[&str]| assert!(Command::new("git").args(args).current_dir(&p).output().unwrap().status.success());
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.name", "T"]);
        run(&["config", "user.email", "t@example.com"]);
        std::fs::write(p.join("f.txt"), "x\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-q", "-m", "first"]);
        if dirty {
            std::fs::write(p.join("g.txt"), "new\n").unwrap();
            std::fs::write(p.join("f.txt"), "y\n").unwrap();
        }
    }
    // Repos inside build output and hidden folders are skipped.
    std::fs::create_dir_all(base.join("node_modules/pkg/.git")).unwrap();
    let found = git::library::scan(&base.to_string_lossy(), 4).unwrap();
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(found.iter().all(|p| !p.contains("node_modules")));

    let mut paths = found.clone();
    paths.push(base.join("missing").to_string_lossy().to_string());
    let sums = git::library::summaries(&paths);
    assert_eq!(sums.len(), 4);
    let b = sums.iter().find(|s| s.path.ends_with("nested/b")).unwrap();
    assert!(b.ok);
    assert_eq!(b.changes, 2);
    assert_eq!(b.branch.as_deref(), Some("main"));
    assert_eq!(b.last_summary.as_deref(), Some("first"));
    let missing = sums.iter().find(|s| s.path.ends_with("missing")).unwrap();
    assert!(!missing.ok);
    assert_eq!(missing.error.as_deref(), Some("Folder not found"));
}

#[test]
fn conflicting_stash_pop_keeps_the_stash() {
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    r.write("f.txt", "stashed version\n");
    git::stash::save(&r.path, Some("mine"), false, false, true).unwrap();
    r.write("f.txt", "committed version\n");
    r.commit_all("other");
    let e = git::stash::pop(&r.path, 0, true).unwrap_err().to_string();
    assert!(e.contains("kept"), "{e}");
    assert_eq!(git::stash::list(&r.path).unwrap().len(), 1, "stash must survive a conflicted pop");
    // A clean pop still drops it.
    r.git(&["checkout", "-q", "-f", "HEAD"]);
    r.write("g.txt", "x\n");
    r.git(&["add", "g.txt"]);
    r.commit_all("g");
    git::stash::drop(&r.path, 0).unwrap();
    r.write("g.txt", "y\n");
    git::stash::save(&r.path, None, false, false, true).unwrap();
    git::stash::pop(&r.path, 0, true).unwrap();
    assert!(git::stash::list(&r.path).unwrap().is_empty());
    assert_eq!(r.read("g.txt"), "y\n");
}
