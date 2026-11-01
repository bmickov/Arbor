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
        // Git for Windows defaults to converting line endings, which would
        // change the file contents these tests compare.
        r.git(&["config", "core.autocrlf", "false"]);
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
    // Unix needs the hook to be executable; Windows runs it through Git's sh.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
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
fn rebase_pauses_on_conflicts_then_continues_skips_or_aborts() {
    let setup = || {
        let r = Repo::new();
        r.write("f.txt", "1\n");
        r.commit_all("one");
        r.write("f.txt", "2\n");
        r.commit_all("two");
        r.write("f.txt", "3\n");
        r.write("other.txt", "x\n");
        r.commit_all("three");
        let ids: Vec<String> = r.git(&["log", "--format=%H", "--reverse"]).lines().map(|s| s.to_string()).collect();
        r
            .git(&["rev-parse", "HEAD"]);
        (r, ids)
    };
    let swap = |ids: &[String]| vec![
        Step { oid: ids[2].clone(), action: "pick".into(), message: None },
        Step { oid: ids[1].clone(), action: "pick".into(), message: None },
    ];

    // Abort puts everything back.
    let (r, ids) = setup();
    let before = r.git(&["rev-parse", "HEAD"]);
    let out = git::rebase::interactive(&r.path, Some(&ids[0]), &swap(&ids), "Reorder").unwrap();
    assert_eq!(out.result, "conflicts");
    assert_eq!(out.conflicts, vec!["f.txt".to_string()]);
    let st = git::status::status(&r.path).unwrap();
    assert_eq!(st.state, "rebase");
    let info = st.paused_rebase.unwrap();
    assert_eq!((info.step, info.total, info.branch.as_deref()), (1, 2, Some("main")));
    // Nothing else can start meanwhile, and the branch hasn't moved.
    assert!(git::merge::merge(&r.path, "main", false).is_err());
    assert_eq!(r.git(&["rev-parse", "main"]), before);
    let f = git::conflict::conflict_file(&r.path, "f.txt").unwrap();
    assert_eq!(f.operation, "rebase");
    assert!(f.theirs_label.contains("three"), "{}", f.theirs_label);
    git::merge::abort(&r.path).unwrap();
    assert_eq!(r.git(&["rev-parse", "HEAD"]), before);
    assert_eq!(r.git(&["symbolic-ref", "--short", "HEAD"]).trim(), "main");
    assert!(r.git(&["status", "--porcelain"]).trim().is_empty());
    assert_eq!(git::status::status(&r.path).unwrap().state, "clean");

    // Resolve each conflict and continue to the end.
    let (r, ids) = setup();
    git::rebase::interactive(&r.path, Some(&ids[0]), &swap(&ids), "Reorder").unwrap();
    git::conflict::resolve_with_content(&r.path, "f.txt", "3\n").unwrap();
    let out = git::rebase::continue_rebase(&r.path).unwrap();
    assert_eq!(out.result, "conflicts", "the second commit conflicts too");
    assert_eq!(git::status::status(&r.path).unwrap().paused_rebase.unwrap().step, 2);
    git::conflict::resolve_with_content(&r.path, "f.txt", "2 and 3\n").unwrap();
    let out = git::rebase::continue_rebase(&r.path).unwrap();
    assert_eq!(out.result, "rebased");
    assert_eq!(r.subjects(), vec!["two", "three", "one"]);
    assert_eq!(r.read("f.txt"), "2 and 3\n");
    assert_eq!(r.read("other.txt"), "x\n");
    assert_eq!(r.git(&["symbolic-ref", "--short", "HEAD"]).trim(), "main");
    assert!(r.git(&["status", "--porcelain"]).trim().is_empty());
    assert!(!Path::new(&r.path).join(".git/arbor-rebase.json").exists());
    // One undo returns to before the rebase.
    git::undo::undo(&r.path).unwrap();
    assert_eq!(r.subjects(), vec!["three", "two", "one"]);

    // Skip leaves the conflicting commit out.
    let (r, ids) = setup();
    git::rebase::interactive(&r.path, Some(&ids[0]), &swap(&ids), "Reorder").unwrap();
    let out = git::rebase::skip_rebase(&r.path).unwrap();
    assert_eq!(out.result, "rebased");
    assert_eq!(r.subjects(), vec!["two", "one"]);

    // A plain rebase onto another branch pauses the same way.
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "topic"]);
    r.write("f.txt", "topic\n");
    r.commit_all("topic change");
    r.git(&["checkout", "-q", "main"]);
    r.write("f.txt", "main\n");
    r.commit_all("main change");
    r.git(&["checkout", "-q", "topic"]);
    let out = git::rebase::rebase(&r.path, "main").unwrap();
    assert_eq!(out.result, "conflicts");
    assert!(git::conflict::conflict_file(&r.path, "f.txt").unwrap().ours_label.starts_with("main"));
    git::conflict::resolve_with_content(&r.path, "f.txt", "main + topic\n").unwrap();
    assert_eq!(git::rebase::continue_rebase(&r.path).unwrap().result, "rebased");
    assert_eq!(r.subjects(), vec!["topic change", "main change", "base"]);
    assert_eq!(r.git(&["symbolic-ref", "--short", "HEAD"]).trim(), "topic");
}

#[test]
fn cherry_pick_and_revert_pause_on_conflicts() {
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "side"]);
    r.write("f.txt", "side\n");
    r.git(&["add", "-A"]);
    r.git(&["-c", "user.name=Someone Else", "-c", "user.email=else@example.com", "commit", "-q", "-m", "side change"]);
    let side = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    r.git(&["checkout", "-q", "main"]);
    r.write("f.txt", "main\n");
    r.commit_all("main change");
    let before = r.git(&["rev-parse", "HEAD"]).trim().to_string();

    let out = git::commit::cherry_pick(&r.path, &side).unwrap();
    assert_eq!(out.conflicts, vec!["f.txt".to_string()]);
    assert_eq!(out.oid, before);
    assert_eq!(git::status::status(&r.path).unwrap().state, "cherrypick");
    // Abort (and undo) put it back.
    git::merge::abort(&r.path).unwrap();
    assert_eq!(git::status::status(&r.path).unwrap().state, "clean");
    assert!(r.git(&["status", "--porcelain"]).trim().is_empty());

    // Resolve and commit: one parent, the original author, git's message.
    git::commit::cherry_pick(&r.path, &side).unwrap();
    let st = git::status::status(&r.path).unwrap();
    assert!(st.merge_message.as_deref().unwrap_or("").starts_with("side change"));
    git::conflict::resolve_with_content(&r.path, "f.txt", "main and side\n").unwrap();
    git::commit::commit(&r.path, "side change", false, true).unwrap();
    assert_eq!(r.git(&["log", "-1", "--format=%an|%P"]).trim(), format!("Someone Else|{before}"));
    assert_eq!(git::status::status(&r.path).unwrap().state, "clean");

    // Revert of a commit later changes conflict with pauses too.
    r.write("f.txt", "later\n");
    r.commit_all("later");
    let out = git::commit::revert(&r.path, &before).unwrap();
    assert!(!out.conflicts.is_empty());
    assert_eq!(git::status::status(&r.path).unwrap().state, "revert");
    git::merge::abort(&r.path).unwrap();
    assert_eq!(r.subjects()[0], "later");
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
    let pushed = git::remote::push(&a.path, None, None, false, false, true, None, None).unwrap();
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
    git::remote::push(&a.path, None, None, false, false, false, None, None).unwrap();

    let p = git::remote::pull(&b_path, false, None, None).unwrap();
    assert_eq!(p.result, "fast-forward");
    assert_eq!(std::fs::read_to_string(Path::new(&b_path).join("f.txt")).unwrap(), "2\n");

    // Diverge and get a rejected push.
    std::fs::write(Path::new(&b_path).join("g.txt"), "g\n").unwrap();
    git::patch::stage_all(&b_path).unwrap();
    git::commit::commit(&b_path, "from b", false, false).unwrap();
    a.write("f.txt", "3\n");
    a.commit_all("three");
    git::remote::push(&a.path, None, None, false, false, false, None, None).unwrap();
    let e = git::remote::push(&b_path, None, None, false, false, false, None, None).unwrap_err().to_string();
    assert!(e.starts_with("REJECTED:"), "{e}");
    let p = git::remote::pull(&b_path, true, None, None).unwrap();
    assert_eq!(p.result, "rebased");
    git::remote::push(&b_path, None, None, false, false, false, None, None).unwrap();
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
        run(&["config", "core.autocrlf", "false"]);
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

#[test]
fn huge_diffs_are_returned_in_full() {
    let r = Repo::new();
    r.write("big.txt", &lines(150_000));
    r.commit_all("init");
    // Change every other line: 150k lines out, 75k in, all in one hunk.
    let changed: String = (1..=150_000).map(|i| if i % 2 == 0 { format!("changed {i}\n") } else { format!("line {i}\n") }).collect();
    r.write("big.txt", &changed);

    let fd = git::diff::unstaged_file(&r.path, "big.txt", s()).unwrap().unwrap();
    assert!(!fd.too_large);
    assert_eq!((fd.additions, fd.deletions), (75_000, 75_000));
    let total: usize = fd.hunks.iter().map(|h| h.lines.len()).sum();
    assert_eq!(total, 225_000);

    // The wire format is column-wise and round-trips to the same lines.
    let json = serde_json::to_value(&fd).unwrap();
    let h = &json["hunks"][0];
    let origins = h["origins"].as_str().unwrap();
    let text = h["text"].as_str().unwrap();
    assert_eq!(origins.len(), fd.hunks[0].lines.len());
    let decoded: Vec<&str> = text.split('\n').collect();
    assert_eq!(decoded.len(), origins.len());
    for (i, l) in fd.hunks[0].lines.iter().enumerate().step_by(997) {
        assert_eq!(decoded[i], l.content);
        assert_eq!(origins.as_bytes()[i] as char, l.origin);
    }
    assert!(h.get("lines").is_none(), "per-line objects must not be sent");

    // Staging a line deep inside the huge hunk still works.
    let deep = fd.hunks[0].lines.iter().position(|l| l.content == "changed 149998").unwrap();
    git::patch::stage_lines(&r.path, "big.txt", &[Selection { hunk: 0, lines: vec![deep] }], s()).unwrap();
    let staged = r.staged_content("big.txt");
    assert!(staged.contains("line 149998\nchanged 149998\n"), "only the addition half was staged");
}

#[test]
fn missing_newline_survives_the_wire_format() {
    let r = Repo::new();
    r.write("a.txt", "one\ntwo");
    r.commit_all("init");
    r.write("a.txt", "one\ntwo\nthree");
    let fd = git::diff::unstaged_file(&r.path, "a.txt", s()).unwrap().unwrap();
    let json = serde_json::to_value(&fd).unwrap();
    let h = &json["hunks"][0];
    assert_eq!(h["origins"], " -++");
    assert_eq!(h["text"], "one\ntwo\ntwo\nthree");
    assert_eq!(h["noNl"], serde_json::json!([1, 3]));
}

#[test]
fn large_diffs_are_held_back_until_asked_for() {
    let r = Repo::new();
    r.write("a.txt", "x\n");
    r.commit_all("init");
    r.write("a.txt", &lines(500));
    let capped = DiffSettings { max_lines: Some(100), ..Default::default() };

    let fd = git::diff::unstaged_file(&r.path, "a.txt", capped).unwrap().unwrap();
    assert!(fd.held_back && fd.hunks.is_empty());
    assert_eq!((fd.additions, fd.deletions), (500, 1), "counts are still filled in");

    // Asking without a cap (the "Show diff" button) returns the lines.
    let fd = git::diff::unstaged_file(&r.path, "a.txt", s()).unwrap().unwrap();
    assert!(!fd.held_back && fd.hunks[0].lines.len() == 501);

    // A cap above the change size changes nothing; 0 means no cap.
    let fd = git::diff::unstaged_file(&r.path, "a.txt", DiffSettings { max_lines: Some(1000), ..Default::default() }).unwrap().unwrap();
    assert!(!fd.held_back);
    let fd = git::diff::unstaged_file(&r.path, "a.txt", DiffSettings { max_lines: Some(0), ..Default::default() }).unwrap().unwrap();
    assert!(!fd.held_back);

    // Staging lines works regardless of the cap (it never holds back).
    git::patch::stage_lines(&r.path, "a.txt", &[Selection { hunk: 0, lines: vec![1] }], capped).unwrap();
    assert_eq!(r.staged_content("a.txt"), "x\nline 1\n");
}

#[test]
fn delete_several_branches_at_once_and_undo() {
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    r.git(&["branch", "merged-a"]);
    r.git(&["branch", "merged-b"]);
    r.git(&["checkout", "-q", "-b", "wip"]);
    r.write("f.txt", "wip\n");
    r.commit_all("unmerged work");
    r.git(&["checkout", "-q", "main"]);
    let branches = || r.git(&["branch", "--format=%(refname:short)"]).lines().map(String::from).collect::<Vec<_>>();
    let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();

    // An unmerged branch in the set: nothing is deleted, and it's named.
    let e = git::branches::delete_branches(&r.path, &names(&["merged-a", "wip", "merged-b"]), false).unwrap_err().to_string();
    assert_eq!(e, "NOT_MERGED:wip");
    assert_eq!(branches().len(), 4);

    // The checked-out branch is refused outright, even with force.
    assert!(git::branches::delete_branches(&r.path, &names(&["merged-a", "main"]), true).is_err());
    assert_eq!(branches().len(), 4);

    // Forced, all three go in one step.
    assert_eq!(git::branches::delete_branches(&r.path, &names(&["merged-a", "wip", "merged-b"]), true).unwrap(), 3);
    assert_eq!(branches(), vec!["main"]);

    // One undo brings all three back, at the same commits.
    let label = git::undo::undo(&r.path).unwrap();
    assert_eq!(label, "Delete 3 branches");
    assert_eq!(branches(), vec!["main", "merged-a", "merged-b", "wip"]);
    assert_eq!(r.git(&["log", "-1", "--format=%s", "wip"]).trim(), "unmerged work");
}

#[test]
fn delete_several_stashes_at_once_and_undo() {
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    for m in ["one", "two", "three", "four"] {
        r.write("f.txt", &format!("{m}\n"));
        git::stash::save(&r.path, Some(m), false, false, true).unwrap();
    }
    let messages = || git::stash::list(&r.path).unwrap().into_iter().map(|s| s.message).collect::<Vec<_>>();
    assert_eq!(messages(), vec!["four", "three", "two", "one"]);
    let list = git::stash::list(&r.path).unwrap();
    // Delete "three" and "one" (indexes 1 and 3) by id.
    let ids = vec![list[1].oid.clone(), list[3].oid.clone()];
    assert_eq!(git::stash::drop_many(&r.path, &ids).unwrap(), 2);
    assert_eq!(messages(), vec!["four", "two"]);

    // Deleting a stash that's already gone deletes nothing.
    assert!(git::stash::drop_many(&r.path, &[list[0].oid.clone(), list[1].oid.clone()]).is_err());
    assert_eq!(messages(), vec!["four", "two"]);

    // One undo restores both, keeping their order relative to each other.
    assert_eq!(git::undo::undo(&r.path).unwrap(), "Delete 2 stashes");
    assert_eq!(messages(), vec!["three", "one", "four", "two"]);
    assert_eq!(r.git(&["stash", "list"]).lines().count(), 4, "git sees them too");
}

#[test]
fn unmerged_check_and_single_delete_labels() {
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    r.git(&["branch", "done"]);
    r.git(&["checkout", "-q", "-b", "wip"]);
    r.write("f.txt", "wip\n");
    r.commit_all("unmerged");
    r.git(&["checkout", "-q", "main"]);
    let names = vec!["done".to_string(), "wip".to_string(), "missing".to_string()];
    assert_eq!(git::branches::unmerged(&r.path, &names).unwrap(), vec!["wip"]);

    git::branches::delete_branches(&r.path, &["done".to_string()], false).unwrap();
    assert_eq!(git::undo::undo(&r.path).unwrap(), "Delete branch done");

    r.write("f.txt", "x\n");
    git::stash::save(&r.path, Some("solo"), false, false, true).unwrap();
    let id = git::stash::list(&r.path).unwrap()[0].oid.clone();
    git::stash::drop_many(&r.path, &[id]).unwrap();
    assert_eq!(git::undo::undo(&r.path).unwrap(), "Delete stash \"solo\"");
    assert_eq!(git::stash::list(&r.path).unwrap().len(), 1);
}

#[test]
fn force_push_with_lease_protects_unfetched_commits() {
    let bare = tempfile::tempdir().unwrap();
    let bare_path = bare.path().to_string_lossy().to_string();
    Command::new("git").args(["init", "-q", "--bare", "-b", "main", &bare_path]).output().unwrap();
    let a = Repo::new();
    a.write("f.txt", "1\n");
    a.commit_all("one");
    git::branches::add_remote(&a.path, "origin", &bare_path).unwrap();
    git::remote::push(&a.path, None, None, false, false, true, None, None).unwrap();

    // B clones, then A pushes more work that B hasn't fetched.
    let dest = tempfile::tempdir().unwrap();
    let b_path = dest.path().join("b").to_string_lossy().to_string();
    git::repo::clone_repo(&bare_path, &b_path, None, None).unwrap();
    for (k, v) in [("user.name", "B"), ("user.email", "b@example.com"), ("core.autocrlf", "false")] {
        Command::new("git").args(["config", k, v]).current_dir(&b_path).output().unwrap();
    }
    a.write("f.txt", "2\n");
    a.commit_all("two from a");
    git::remote::push(&a.path, None, None, false, false, false, None, None).unwrap();

    // B rewrites history and force pushes with lease: refused, A's commit is safe.
    std::fs::write(Path::new(&b_path).join("f.txt"), "b\n").unwrap();
    git::patch::stage_all(&b_path).unwrap();
    git::commit::commit(&b_path, "b's version", true, false).unwrap();
    let e = git::remote::push(&b_path, None, None, true, true, false, None, None).unwrap_err().to_string();
    assert!(e.starts_with("LEASE:"), "{e}");
    let tip = |p: &str| Command::new("git").args(["log", "-1", "--format=%s", "main"]).current_dir(p).output().unwrap().stdout;
    assert_eq!(String::from_utf8_lossy(&tip(&bare_path)).trim(), "two from a");

    // After fetching (B has now seen A's commit), the lease holds and the force push goes through.
    git::remote::fetch(&b_path, None, false, None, None).unwrap();
    git::remote::push(&b_path, None, None, true, true, false, None, None).unwrap();
    assert_eq!(String::from_utf8_lossy(&tip(&bare_path)).trim(), "b's version");

    // A plain force push (no lease) overwrites regardless.
    a.write("f.txt", "3\n");
    a.commit_all("three from a");
    git::remote::push(&a.path, None, None, true, false, false, None, None).unwrap();
    assert_eq!(String::from_utf8_lossy(&tip(&bare_path)).trim(), "three from a");
}

#[test]
fn identity_per_repository_and_commit_template() {
    let r = Repo::new();
    // Repo::new sets a local identity.
    let id = git::identity::identity(&r.path).unwrap();
    assert_eq!((id.name.as_deref(), id.email.as_deref(), id.local), (Some("Test"), Some("test@example.com"), true));

    git::identity::set_identity(&r.path, Some(("Work Me", "me@work.example"))).unwrap();
    r.write("a.txt", "a\n");
    git::patch::stage_all(&r.path).unwrap();
    git::commit::commit(&r.path, "as work", false, false).unwrap();
    assert_eq!(r.git(&["log", "-1", "--format=%an <%ae>"]).trim(), "Work Me <me@work.example>");

    // Going back to the global identity removes the repository's own setting.
    git::identity::set_identity(&r.path, None).unwrap();
    assert!(!git::identity::identity(&r.path).unwrap().local);
    git::identity::set_identity(&r.path, None).unwrap(); // removing twice is fine

    assert!(git::identity::commit_template(&r.path).unwrap().is_none());
    r.write(".gitmessage", "feat: \n\nWhy:\n");
    r.git(&["config", "commit.template", ".gitmessage"]);
    assert_eq!(git::identity::commit_template(&r.path).unwrap().as_deref(), Some("feat: \n\nWhy:\n"));
}

#[test]
fn ignore_untrack_and_restore_files() {
    let r = Repo::new();
    r.write("keep.txt", "v1\n");
    r.write("secret.env", "TOKEN=1\n");
    r.commit_all("first");
    r.write("keep.txt", "v2\n");
    r.commit_all("second");

    // Ignore: appended once, undoable.
    assert!(git::files::ignore(&r.path, "*.log").unwrap());
    assert!(!git::files::ignore(&r.path, "*.log").unwrap());
    assert_eq!(r.read(".gitignore"), "*.log\n");
    git::undo::undo(&r.path).unwrap();
    assert!(!Path::new(&r.path).join(".gitignore").exists());

    // Stop tracking: gone from the index, still on disk; undo puts it back.
    git::files::untrack(&r.path, &["secret.env".to_string()]).unwrap();
    assert!(r.read("secret.env").contains("TOKEN"));
    assert!(r.git(&["ls-files"]).lines().all(|l| l != "secret.env"));
    git::undo::undo(&r.path).unwrap();
    assert!(r.git(&["ls-files"]).lines().any(|l| l == "secret.env"));

    // Restore the version from before the last commit, then undo it.
    git::files::restore_file(&r.path, "HEAD^", "keep.txt").unwrap();
    assert_eq!(r.read("keep.txt"), "v1\n");
    git::undo::undo(&r.path).unwrap();
    assert_eq!(r.read("keep.txt"), "v2\n");
    // A file that didn't exist at that commit is removed.
    r.write("new.txt", "n\n");
    r.commit_all("third");
    git::files::restore_file(&r.path, "HEAD^", "new.txt").unwrap();
    assert!(!Path::new(&r.path).join("new.txt").exists());
}

#[test]
fn patches_round_trip() {
    let r = Repo::new();
    r.write("a.txt", "one\ntwo\n");
    r.commit_all("base");
    r.write("a.txt", "one\nTWO\n");
    r.write("b.txt", "new file\n");
    r.git(&["add", "-A"]);
    r.git(&["commit", "-q", "-m", "Change two and add b", "-m", "Because reasons."]);
    let head = r.git(&["rev-parse", "HEAD"]).trim().to_string();

    // A commit as an email patch, applied to a copy that's one commit behind.
    let dir = tempfile::tempdir().unwrap();
    let patch = dir.path().join("change.patch").to_string_lossy().to_string();
    git::files::save_commit_patch(&r.path, &head, &patch).unwrap();
    let text = std::fs::read_to_string(&patch).unwrap();
    assert!(text.starts_with("From ") && text.contains("Subject: [PATCH] Change two and add b"), "{text}");
    r.git(&["reset", "-q", "--hard", "HEAD^"]);
    let applied = git::files::apply_patch(&r.path, &patch).unwrap();
    assert_eq!(applied.files, 2);
    assert_eq!(applied.subject.as_deref(), Some("Change two and add b"));
    assert_eq!(applied.body.as_deref(), Some("Because reasons."));
    assert_eq!(r.read("a.txt"), "one\nTWO\n");
    assert_eq!(r.read("b.txt"), "new file\n");

    // Undo puts the files back as they were.
    git::undo::undo(&r.path).unwrap();
    assert_eq!(r.read("a.txt"), "one\ntwo\n");
    assert!(!Path::new(&r.path).join("b.txt").exists());

    // Uncommitted changes (including a new file) as a plain patch.
    r.write("a.txt", "one\n2\n");
    r.write("c.txt", "c\n");
    let p2 = dir.path().join("wip.patch").to_string_lossy().to_string();
    assert_eq!(git::files::save_changes_patch(&r.path, false, &p2).unwrap(), 2);
    assert!(git::files::save_changes_patch(&r.path, true, &p2).is_err(), "nothing staged");
    git::files::save_changes_patch(&r.path, false, &p2).unwrap();
    r.git(&["checkout", "-q", "--", "a.txt"]);
    std::fs::remove_file(Path::new(&r.path).join("c.txt")).unwrap();
    let applied = git::files::apply_patch(&r.path, &p2).unwrap();
    assert!(applied.subject.is_none());
    assert_eq!(r.read("a.txt"), "one\n2\n");
    assert_eq!(r.read("c.txt"), "c\n");

    // A patch that doesn't fit changes nothing.
    r.write("a.txt", "completely different\n");
    std::fs::remove_file(Path::new(&r.path).join("c.txt")).unwrap();
    assert!(git::files::apply_patch(&r.path, &p2).is_err());
    assert_eq!(r.read("a.txt"), "completely different\n");
    assert!(!Path::new(&r.path).join("c.txt").exists());
}

#[test]
fn reflog_finds_commits_of_a_deleted_branch() {
    let r = Repo::new();
    r.write("f.txt", "1\n");
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "experiment"]);
    r.write("f.txt", "2\n");
    r.commit_all("precious work");
    let lost = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    r.git(&["checkout", "-q", "main"]);
    r.git(&["branch", "-D", "experiment"]);

    let log = git::review::reflog(&r.path, "HEAD", 100).unwrap();
    let e = log.iter().find(|e| e.oid == lost).expect("the deleted branch's commit is in HEAD's reflog");
    assert!(e.lost, "nothing else points at it");
    assert_eq!(e.summary.as_deref(), Some("precious work"));
    assert!(!log[0].lost, "the current commit is on main");
    // Recovering it is just making a branch there.
    git::branches::create_branch(&r.path, "rescued", Some(&lost), false).unwrap();
    let log = git::review::reflog(&r.path, "HEAD", 100).unwrap();
    assert!(!log.iter().find(|e| e.oid == lost).unwrap().lost);
}

#[test]
fn branches_review_detects_merged_squashed_and_stale() {
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    // Merged normally.
    r.git(&["checkout", "-q", "-b", "merged"]);
    r.write("a.txt", "a\n");
    r.commit_all("a");
    r.git(&["checkout", "-q", "main"]);
    r.git(&["merge", "-q", "--no-ff", "-m", "merge a", "merged"]);
    // Squash-merged: its commits never reach main, but its changes do.
    r.git(&["checkout", "-q", "-b", "squashed"]);
    r.write("b.txt", "b1\n");
    r.commit_all("b1");
    r.write("b.txt", "b2\n");
    r.commit_all("b2");
    r.git(&["checkout", "-q", "main"]);
    r.git(&["merge", "-q", "--squash", "squashed"]);
    r.git(&["commit", "-q", "-m", "squash b"]);
    // Not merged.
    r.git(&["checkout", "-q", "-b", "wip"]);
    r.write("c.txt", "c\n");
    r.commit_all("c");
    r.git(&["checkout", "-q", "main"]);

    let rev = git::review::branches_review(&r.path, None).unwrap();
    assert_eq!(rev.base, "main");
    let row = |n: &str| rev.rows.iter().find(|x| x.name == n).unwrap();
    assert_eq!(row("merged").merged, "merged");
    assert_eq!(row("squashed").merged, "squashed");
    assert_eq!(row("wip").merged, "unmerged");
    assert_eq!((row("wip").ahead, row("wip").behind), (1, 0));
    assert!(row("main").is_base && row("main").is_head);
}

#[test]
fn compare_two_branches() {
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("f.txt", "feature\n");
    r.commit_all("feature 1");
    r.write("g.txt", "g\n");
    r.commit_all("feature 2");
    r.git(&["checkout", "-q", "main"]);
    r.write("h.txt", "h\n");
    r.commit_all("main moved on");

    let c = git::review::compare(&r.path, "main", "feature").unwrap();
    assert_eq!(c.ahead.iter().map(|x| x.summary.as_str()).collect::<Vec<_>>(), vec!["feature 2", "feature 1"]);
    assert_eq!(c.behind.iter().map(|x| x.summary.as_str()).collect::<Vec<_>>(), vec!["main moved on"]);
    // Files are what feature brings (since the merge base), not main's own change.
    let mut files: Vec<_> = c.files.iter().map(|f| f.path.as_str()).collect();
    files.sort();
    assert_eq!(files, vec!["f.txt", "g.txt"]);
    let d = git::review::compare_file(&r.path, "main", "feature", "f.txt", Default::default()).unwrap().unwrap();
    assert_eq!((d.additions, d.deletions), (1, 1));
    assert!(git::review::compare(&r.path, "main", "main").is_err());
}

#[test]
fn browse_and_export_zip() {
    let r = Repo::new();
    r.write("README.md", "hello\n");
    r.write("src/main.rs", "fn main() {}\n");
    r.commit_all("v1");
    r.write("src/main.rs", "fn main() { println!(\"v2\"); }\n");
    r.commit_all("v2");

    let files = git::tree::tree_at(&r.path, "HEAD^").unwrap();
    let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["README.md", "src/main.rs"]);
    assert_eq!(files[0].size, 6);
    let old = git::tree::file_content(&r.path, "HEAD^", "src/main.rs").unwrap();
    assert_eq!(old.text.as_deref(), Some("fn main() {}\n"));

    let dir = tempfile::tempdir().unwrap();
    let zip_path = dir.path().join("v1.zip").to_string_lossy().to_string();
    assert_eq!(git::tree::export_zip(&r.path, "HEAD^", &zip_path, "project-v1").unwrap(), 2);
    let out = Command::new("unzip").args(["-l", &zip_path]).output();
    if let Ok(out) = out {
        let list = String::from_utf8_lossy(&out.stdout);
        assert!(list.contains("project-v1/src/main.rs") && list.contains("project-v1/README.md"), "{list}");
        let content = Command::new("unzip").args(["-p", &zip_path, "project-v1/src/main.rs"]).output().unwrap().stdout;
        assert_eq!(String::from_utf8_lossy(&content), "fn main() {}\n");
    }
}

#[test]
fn conflict_wizard_reads_sides_and_saves_a_resolution() {
    let r = Repo::new();
    r.write("a.txt", "top\nshared\nbottom\n");
    r.write("gone.txt", "will be deleted\n");
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("a.txt", "top\ntheirs\nbottom\n");
    r.write("gone.txt", "changed on feature\n");
    r.commit_all("feature");
    r.git(&["checkout", "-q", "main"]);
    r.write("a.txt", "top\nmine\nbottom\n");
    std::fs::remove_file(Path::new(&r.path).join("gone.txt")).unwrap();
    r.commit_all("main");
    let m = git::merge::merge(&r.path, "feature", false).unwrap();
    assert_eq!(m.conflicts.len(), 2);

    let f = git::conflict::conflict_file(&r.path, "a.txt").unwrap();
    assert_eq!((f.ours_label.as_str(), f.theirs_label.as_str(), f.operation.as_str()), ("main", "feature", "merge"));
    assert!(f.has_base && f.has_ours && f.has_theirs && !f.binary);
    assert!(f.text.as_deref().unwrap().contains("<<<<<<< "));
    // The fresh merge carries the base in diff3 style.
    let merged = f.merged.unwrap();
    assert!(merged.contains("<<<<<<< main\nmine\n||||||| base\nshared\n=======\ntheirs\n>>>>>>> feature\n"), "{merged}");

    // A side that deleted the file is reported, with nothing to merge line by line.
    let g = git::conflict::conflict_file(&r.path, "gone.txt").unwrap();
    assert!(!g.has_ours && g.has_theirs && g.merged.is_none());

    // Markers left in are refused; a clean result is written and staged.
    assert!(git::conflict::resolve_with_content(&r.path, "a.txt", &merged).is_err());
    git::conflict::resolve_with_content(&r.path, "a.txt", "top\nmine\ntheirs\nbottom\n").unwrap();
    assert_eq!(r.read("a.txt"), "top\nmine\ntheirs\nbottom\n");
    assert_eq!(r.staged_content("a.txt"), "top\nmine\ntheirs\nbottom\n");
    assert!(git::conflict::conflict_file(&r.path, "a.txt").is_err());
    git::patch::resolve_conflict(&r.path, "gone.txt", "ours").unwrap();
    assert!(r.git(&["diff", "--name-only", "--diff-filter=U"]).trim().is_empty());
}

#[test]
fn conflict_wizard_handles_add_add_conflicts() {
    let r = Repo::new();
    r.write("x", "x\n");
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("new.txt", "one\ntheirs\n");
    r.commit_all("feature adds");
    r.git(&["checkout", "-q", "main"]);
    r.write("new.txt", "one\nmine\n");
    r.commit_all("main adds");
    git::merge::merge(&r.path, "feature", false).unwrap();
    let f = git::conflict::conflict_file(&r.path, "new.txt").unwrap();
    assert!(!f.has_base);
    let merged = f.merged.unwrap();
    assert!(merged.contains("<<<<<<< main") && merged.contains(">>>>>>> feature"), "{merged}");
}

fn have(program: &str, arg: &str) -> bool {
    Command::new(program).arg(arg).output().map(|o| o.status.success()).unwrap_or(false)
}

#[test]
fn ssh_signed_commits_and_verification() {
    if !have("ssh-keygen", "-V") && Command::new("ssh-keygen").arg("-?").output().is_err() {
        eprintln!("ssh-keygen isn't installed; skipping");
        return;
    }
    let r = Repo::new();
    let keys = tempfile::tempdir().unwrap();
    let key = keys.path().join("id_ed25519");
    let ok = Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-C", "test", "-f"]).arg(&key).status().unwrap();
    assert!(ok.success());
    let key_s = key.to_string_lossy().replace('\\', "/");

    // Use plain ssh-keygen whatever the machine's global git config says.
    r.git(&["config", "gpg.ssh.program", "ssh-keygen"]);
    r.write("a", "1\n");
    r.commit_all("unsigned");
    git::sign::set_settings(&r.path, true, "ssh", &key_s, false).unwrap();
    let st = git::sign::settings(&r.path).unwrap();
    assert!(st.enabled && st.format == "ssh" && st.scope == "local");

    r.write("a", "2\n");
    r.git(&["add", "a"]);
    let c = git::commit::commit_signed(&r.path, "signed", false, false, None).unwrap();
    assert!(r.git(&["cat-file", "commit", &c.oid]).contains("-----BEGIN SSH SIGNATURE-----"));
    assert_eq!(r.subjects()[0], "signed");
    // The reflog reads like git's.
    assert!(r.git(&["reflog", "-1", "--format=%gs"]).starts_with("commit: signed"));

    // Without an allowed signers file the signature is valid but the signer unknown.
    let v = git::sign::verify(&r.path, &c.oid).unwrap().unwrap();
    assert_eq!((v.format.as_str(), v.status.as_str()), ("ssh", "unverified"), "{v:?}");
    // With one, it's good and names the signer.
    let public = std::fs::read_to_string(keys.path().join("id_ed25519.pub")).unwrap();
    let allowed = keys.path().join("allowed");
    std::fs::write(&allowed, format!("test@example.com {}", public.trim())).unwrap();
    r.git(&["config", "gpg.ssh.allowedSignersFile", &allowed.to_string_lossy()]);
    let v = git::sign::verify(&r.path, &c.oid).unwrap().unwrap();
    assert_eq!(v.status, "good", "{v:?}");
    assert_eq!(v.signer.as_deref(), Some("test@example.com"));
    // git agrees.
    r.git(&["verify-commit", &c.oid]);

    // An unsigned commit has nothing to verify; opting out per commit works.
    let first = r.git(&["rev-parse", "HEAD~1"]);
    assert!(git::sign::verify(&r.path, first.trim()).unwrap().is_none());
    r.write("a", "3\n");
    r.git(&["add", "a"]);
    let u = git::commit::commit_signed(&r.path, "not signed", false, false, Some(false)).unwrap();
    assert!(git::sign::verify(&r.path, &u.oid).unwrap().is_none());
    // Amending signs too, and keeps the author.
    let a = git::commit::commit_signed(&r.path, "amended", true, false, None).unwrap();
    assert_eq!(git::sign::verify(&r.path, &a.oid).unwrap().unwrap().status, "good");
    assert!(r.git(&["reflog", "-1", "--format=%gs"]).starts_with("commit (amend): amended"));
}

#[test]
fn gpg_signed_commits_and_verification() {
    if cfg!(windows) || !have("gpg", "--version") {
        eprintln!("gpg isn't available; skipping");
        return;
    }
    let r = Repo::new();
    let home = tempfile::tempdir().unwrap();
    // A wrapper keeps this test's keyring away from the real one without touching the environment.
    let wrapper = home.path().join("gpg.sh");
    std::fs::write(&wrapper, format!("#!/bin/sh\nexec gpg --homedir '{}' \"$@\"\n", home.path().display())).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let gen = Command::new(&wrapper)
        .args(["--batch", "--passphrase", "", "--quick-gen-key", "Test <test@example.com>", "ed25519", "sign", "never"])
        .output()
        .unwrap();
    assert!(gen.status.success(), "{}", String::from_utf8_lossy(&gen.stderr));
    r.git(&["config", "gpg.program", &wrapper.to_string_lossy()]);
    git::sign::set_settings(&r.path, true, "openpgp", "test@example.com", false).unwrap();

    r.write("a", "1\n");
    r.git(&["add", "a"]);
    let c = git::commit::commit_signed(&r.path, "gpg signed", false, false, None).unwrap();
    assert!(r.git(&["cat-file", "commit", &c.oid]).contains("-----BEGIN PGP SIGNATURE-----"));
    let v = git::sign::verify(&r.path, &c.oid).unwrap().unwrap();
    // A freshly generated key is ultimately trusted.
    assert_eq!((v.format.as_str(), v.status.as_str()), ("openpgp", "good"), "{v:?}");
    assert_eq!(v.signer.as_deref(), Some("Test <test@example.com>"));
    r.git(&["verify-commit", &c.oid]);

    // A tampered commit (same signature, different content) is reported as bad.
    let raw = r.git(&["cat-file", "commit", &c.oid]).replace("gpg signed", "gpg signed!");
    let mut child = Command::new("git").args(["hash-object", "-t", "commit", "-w", "--stdin"]).current_dir(&r.path)
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(raw.as_bytes()).unwrap();
    let forged = String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap();
    let v = git::sign::verify(&r.path, forged.trim()).unwrap().unwrap();
    assert_eq!(v.status, "bad", "{v:?}");

    // Merges and cherry-picks are signed as well.
    r.git(&["checkout", "-q", "-b", "side", "HEAD~0"]);
    r.write("b", "b\n");
    r.git(&["add", "b"]);
    let side = git::commit::commit_signed(&r.path, "side", false, false, Some(false)).unwrap();
    r.git(&["checkout", "-q", "main"]);
    let p = git::commit::cherry_pick(&r.path, &side.oid).unwrap();
    assert_eq!(git::sign::verify(&r.path, &p.oid).unwrap().unwrap().status, "good");
}

#[test]
fn worktrees_add_list_and_remove() {
    let r = Repo::new();
    r.write("a", "1\n");
    r.commit_all("first");
    r.git(&["branch", "existing"]);
    let parent = tempfile::tempdir().unwrap();
    let wt1 = parent.path().join("feature wt");
    let wt1_s = wt1.to_string_lossy().to_string();

    // A new branch from HEAD in a new folder.
    let made = git::worktree::add(&r.path, &wt1_s, "feature", true, None).unwrap();
    assert!(Path::new(&made).join("a").exists());
    assert_eq!(std::fs::read_to_string(Path::new(&made).join("a")).unwrap(), "1\n");
    // An existing branch, into an empty folder that already exists.
    let wt2 = parent.path().join("existing");
    std::fs::create_dir(&wt2).unwrap();
    git::worktree::add(&r.path, &wt2.to_string_lossy(), "existing", false, None).unwrap();
    // A branch can only be checked out once, and a busy folder is refused.
    assert!(git::worktree::add(&r.path, &parent.path().join("again").to_string_lossy(), "feature", false, None).is_err());
    assert!(git::worktree::add(&r.path, &wt1_s, "other", true, None).is_err());

    let list = git::worktree::list(&r.path).unwrap();
    assert_eq!(list.len(), 3);
    assert!(list[0].is_main && list[0].is_current && list[0].branch.as_deref() == Some("main"));
    let f = list.iter().find(|w| w.branch.as_deref() == Some("feature")).unwrap();
    assert!(!f.is_main && !f.is_current && f.name.is_some() && !f.missing);
    // git sees them too.
    assert!(r.git(&["worktree", "list"]).contains("[feature]"));

    // Listing from inside a linked worktree shows the same set, with it current.
    let inside = git::worktree::list(&made).unwrap();
    assert_eq!(inside.len(), 3);
    assert!(inside.iter().any(|w| w.is_current && w.branch.as_deref() == Some("feature")));
    assert!(inside[0].is_main && !inside[0].is_current);

    // Uncommitted changes block removal unless forced; the branch survives.
    std::fs::write(Path::new(&made).join("a"), "changed\n").unwrap();
    let name = f.name.clone().unwrap();
    let e = git::worktree::remove(&r.path, &name, false).unwrap_err().to_string();
    assert!(e.starts_with("DIRTY:"), "{e}");
    git::worktree::remove(&r.path, &name, true).unwrap();
    assert!(!wt1.exists());
    assert_eq!(git::worktree::list(&r.path).unwrap().len(), 2);
    r.git(&["rev-parse", "--verify", "feature"]);

    // A worktree whose folder was deleted by hand is pruned.
    std::fs::remove_dir_all(&wt2).unwrap();
    assert!(git::worktree::list(&r.path).unwrap().iter().any(|w| w.missing));
    assert_eq!(git::worktree::prune(&r.path).unwrap(), 1);
    assert_eq!(git::worktree::list(&r.path).unwrap().len(), 1);
}

#[test]
fn submodules_list_and_update() {
    let lib = Repo::new();
    lib.write("lib.txt", "v1\n");
    lib.commit_all("lib v1");
    let r = Repo::new();
    r.write("app", "app\n");
    r.commit_all("app");
    r.git(&["-c", "protocol.file.allow=always", "submodule", "add", "-q", &lib.path, "vendor/lib"]);
    r.commit_all("add lib");

    let subs = git::worktree::submodules(&r.path).unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!((subs[0].path.as_str(), subs[0].state.as_str()), ("vendor/lib", "ok"));
    assert_eq!(subs[0].url.as_deref(), Some(lib.path.as_str()));

    // A fresh clone has it uninitialized; updating clones and checks it out.
    let dest = tempfile::tempdir().unwrap();
    let clone = dest.path().join("c");
    Command::new("git").args(["clone", "-q"]).arg(&r.path).arg(&clone).status().unwrap();
    let c = clone.to_string_lossy().to_string();
    assert_eq!(git::worktree::submodules(&c).unwrap()[0].state, "uninitialized");
    assert_eq!(git::worktree::update_submodules(&c, &[], None, None).unwrap(), 1);
    assert_eq!(std::fs::read_to_string(clone.join("vendor/lib/lib.txt")).unwrap(), "v1\n");
    assert_eq!(git::worktree::submodules(&c).unwrap()[0].state, "ok");

    // New commits in the submodule's folder show as moved; update puts it back.
    let sub = clone.join("vendor/lib");
    Command::new("git").args(["-c", "user.name=T", "-c", "user.email=t@e", "-c", "commit.gpgsign=false", "commit", "-q", "--allow-empty", "-m", "local"]).current_dir(&sub).status().unwrap();
    assert_eq!(git::worktree::submodules(&c).unwrap()[0].state, "moved");
    git::worktree::update_submodules(&c, &["vendor/lib".to_string()], None, None).unwrap();
    assert_eq!(git::worktree::submodules(&c).unwrap()[0].state, "ok");
    // Changed files inside block updating it.
    std::fs::write(sub.join("lib.txt"), "edited\n").unwrap();
    assert_eq!(git::worktree::submodules(&c).unwrap()[0].state, "dirty");
    assert!(git::worktree::update_submodules(&c, &["vendor/lib".to_string()], None, None).is_err());
}

#[test]
fn partial_stash_and_history_filters() {
    let r = Repo::new();
    r.write("a.txt", "a\n");
    r.write("src/b.txt", "b\n");
    r.commit_all("first");
    // Only the chosen file is stashed, under the given name.
    r.write("a.txt", "a2\n");
    r.write("src/b.txt", "b2\n");
    r.write("new.txt", "untracked\n");
    r.write("staged.txt", "staged\n");
    r.git(&["add", "staged.txt"]);
    git::stash::save_paths(&r.path, Some("just a"), false, false, false, &["a.txt".to_string()]).unwrap();
    assert_eq!(r.read("a.txt"), "a\n");
    assert_eq!(r.read("src/b.txt"), "b2\n");
    // Other changes stay exactly as they were: untracked, and staged.
    assert_eq!(r.read("new.txt"), "untracked\n");
    assert_eq!(r.staged_content("staged.txt"), "staged\n");
    let list = git::stash::list(&r.path).unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].message.contains("just a"), "{}", list[0].message);
    assert!(r.git(&["log", "-1", "--format=%s", "stash@{0}"]).contains("On main: just a"));
    assert_eq!(r.git(&["stash", "list"]).lines().count(), 1);
    git::stash::pop(&r.path, 0, false).unwrap();
    assert_eq!(r.read("a.txt"), "a2\n");
    // Applying while something else is staged works, and that stays staged.
    assert_eq!(r.staged_content("staged.txt"), "staged\n");
    assert_eq!(r.read("src/b.txt"), "b2\n");
    r.git(&["reset", "-q", "--hard"]);
    r.git(&["clean", "-fdq"]);

    // History to search.
    r.write("src/b.txt", "needle here\n");
    r.commit_all("add the needle");
    r.write("a.txt", "unrelated\n");
    r.git(&["add", "-A"]);
    r.git(&["-c", "user.name=Ana", "-c", "user.email=ana@example.org", "commit", "-q", "-m", "ana's change", "--date=2020-01-02T00:00:00"]);
    r.write("src/b.txt", "gone\n");
    r.commit_all("remove the needle");
    let subjects = |f: git::log::LogFilter| -> Vec<String> {
        git::log::search(&r.path, "head", &f, 100).unwrap().rows.into_iter().map(|c| c.summary).collect()
    };
    use git::log::LogFilter;
    assert_eq!(subjects(LogFilter { author: Some("ANA@".into()), ..Default::default() }), vec!["ana's change"]);
    assert_eq!(subjects(LogFilter { path: Some("src".into()), ..Default::default() }), vec!["remove the needle", "add the needle", "first"]);
    assert_eq!(subjects(LogFilter { contains: Some("needle".into()), ..Default::default() }), vec!["remove the needle", "add the needle"]);
    assert_eq!(subjects(LogFilter { contains: Some("needle".into()), text: Some("remove".into()), ..Default::default() }), vec!["remove the needle"]);
    // Author dates: ana's commit is from 2020.
    let y2021 = 1609459200;
    assert_eq!(subjects(LogFilter { until: Some(y2021), ..Default::default() }), vec!["ana's change"]);
    assert!(!subjects(LogFilter { since: Some(y2021), ..Default::default() }).contains(&"ana's change".to_string()));
}

#[test]
fn gitflow_start_and_finish() {
    let r = Repo::new();
    r.write("a", "1\n");
    r.commit_all("init");
    let c = git::gitflow::suggest(&r.path).unwrap();
    assert_eq!((c.main.as_str(), c.develop.as_str()), ("main", "develop"));
    let c = git::gitflow::FlowConfig { version_tag: "v".into(), ..c };
    git::gitflow::init(&r.path, &c).unwrap();
    assert_eq!(r.git(&["config", "gitflow.branch.develop"]).trim(), "develop");
    r.git(&["rev-parse", "--verify", "develop"]);

    // Feature: from develop, merged back with a merge commit, then deleted.
    assert_eq!(git::gitflow::start(&r.path, "feature", "login").unwrap(), "feature/login");
    r.write("login", "x\n");
    r.commit_all("login screen");
    let f = git::gitflow::finish(&r.path, "feature/login", None, false).unwrap();
    assert_eq!(f.result, "finished");
    assert_eq!(r.git(&["symbolic-ref", "--short", "HEAD"]).trim(), "develop");
    assert!(r.git(&["branch", "--list", "feature/login"]).trim().is_empty());
    assert_eq!(r.git(&["log", "-1", "--format=%P", "develop"]).split_whitespace().count(), 2, "no fast-forward");

    // Release: merged into main and tagged, then main into develop.
    git::gitflow::start(&r.path, "release", "1.0").unwrap();
    assert!(git::gitflow::start(&r.path, "release", "1.1").is_err(), "one release at a time");
    r.write("version", "1.0\n");
    r.commit_all("bump version");
    let f = git::gitflow::finish(&r.path, "release/1.0", Some("First release"), false).unwrap();
    assert_eq!((f.result, f.tag.as_deref()), ("finished", Some("v1.0")));
    assert_eq!(r.git(&["rev-parse", "v1.0^{commit}"]), r.git(&["rev-parse", "main"]));
    assert!(r.git(&["cat-file", "-p", "v1.0"]).contains("First release"));
    r.git(&["merge-base", "--is-ancestor", "main", "develop"]);

    // Hotfix from main, finished into both; a conflict stops it and finishing again carries on.
    git::gitflow::start(&r.path, "hotfix", "1.0.1").unwrap();
    r.write("version", "1.0.1\n");
    r.commit_all("hotfix");
    r.git(&["checkout", "-q", "develop"]);
    r.write("version", "2.0-dev\n");
    r.commit_all("start 2.0");
    let f = git::gitflow::finish(&r.path, "hotfix/1.0.1", None, false).unwrap();
    assert_eq!(f.result, "conflicts");
    assert_eq!(f.merging_into.as_deref(), Some("develop"));
    assert_eq!(f.tag.as_deref(), Some("v1.0.1"));
    git::conflict::resolve_with_content(&r.path, "version", "2.0-dev\n").unwrap();
    git::commit::commit(&r.path, "Merge main into develop", false, true).unwrap();
    let f = git::gitflow::finish(&r.path, "hotfix/1.0.1", None, false).unwrap();
    assert_eq!(f.result, "finished");
    assert!(r.git(&["branch", "--list", "hotfix/1.0.1"]).trim().is_empty());
    assert_eq!(r.git(&["show", "main:version"]), "1.0.1\n");
}

#[test]
fn stacked_branches_restack() {
    let r = Repo::new();
    r.write("a", "1\n");
    r.commit_all("init");
    r.git(&["checkout", "-q", "-b", "part1"]);
    r.write("p1", "1\n");
    r.commit_all("part 1");
    r.git(&["checkout", "-q", "-b", "part2"]);
    r.write("p2", "2\n");
    r.commit_all("part 2");
    git::stack::set_parent(&r.path, "part1", Some("main")).unwrap();
    git::stack::set_parent(&r.path, "part2", Some("part1")).unwrap();
    assert!(git::stack::set_parent(&r.path, "part1", Some("part2")).is_err(), "no loops");
    assert_eq!(git::stack::descendants(&r.path, "main").unwrap(), vec!["part1", "part2"]);

    // part1 gets amended (rewritten) and main moves on.
    r.git(&["checkout", "-q", "main"]);
    r.write("m", "m\n");
    r.commit_all("main moves");
    r.git(&["checkout", "-q", "part1"]);
    r.write("p1", "1 amended\n");
    r.git(&["commit", "-q", "-a", "--amend", "-m", "part 1 (amended)"]);
    let list = git::stack::list(&r.path).unwrap();
    let p1 = list.iter().find(|s| s.branch == "part1").unwrap();
    assert_eq!((p1.behind, p1.own), (1, 1));

    assert_eq!(git::stack::restack(&r.path, "part1").unwrap().result, "rebased");
    assert_eq!(r.git(&["log", "--format=%s", "part1"]).lines().collect::<Vec<_>>(), vec!["part 1 (amended)", "main moves", "init"]);
    // part2's own commit only: the old "part 1" isn't replayed on top of the amended one.
    assert_eq!(git::stack::restack(&r.path, "part2").unwrap().result, "rebased");
    assert_eq!(r.git(&["log", "--format=%s", "part2"]).lines().collect::<Vec<_>>(), vec!["part 2", "part 1 (amended)", "main moves", "init"]);
    assert!(git::stack::list(&r.path).unwrap().iter().all(|s| s.behind == 0));
    assert_eq!(git::stack::restack(&r.path, "part2").unwrap().result, "up-to-date");
    git::stack::set_parent(&r.path, "part2", None).unwrap();
    assert_eq!(git::stack::list(&r.path).unwrap().len(), 1);
}

#[test]
fn bisect_finds_the_first_bad_commit() {
    let r = Repo::new();
    for i in 1..=16 {
        r.write("n", &format!("{i}\n"));
        r.commit_all(&format!("c{i}"));
    }
    let ids: Vec<String> = r.git(&["log", "--format=%H", "--reverse"]).lines().map(str::to_string).collect();
    let id = |n: usize| ids[n - 1].clone();
    let first_bad = 11;
    let good = id(1);
    let mut st = git::bisect::start(&r.path, None, &good).unwrap();
    assert_eq!(st.original, "main");
    let mut tests = 0;
    while st.found.is_none() {
        tests += 1;
        assert!(tests <= 6, "too many steps");
        let v: usize = r.read("n").trim().parse().unwrap();
        st = git::bisect::mark(&r.path, if v >= first_bad { "bad" } else { "good" }, None).unwrap();
    }
    assert_eq!(st.found.as_deref(), Some(id(first_bad).as_str()));
    assert!(tests <= 4, "{tests} tests for 15 commits");
    // git sees the same session.
    assert!(r.git(&["rev-parse", "refs/bisect/bad"]).trim() == id(first_bad));
    git::bisect::reset(&r.path).unwrap();
    assert_eq!(r.git(&["symbolic-ref", "--short", "HEAD"]).trim(), "main");
    assert!(git::bisect::state(&r.path).unwrap().is_none());
    assert!(r.git(&["for-each-ref", "refs/bisect"]).trim().is_empty());
}

#[test]
fn lfs_files_are_staged_as_pointers_and_checked_out() {
    if !git::lfs::installed() {
        eprintln!("git-lfs isn't installed; skipping");
        return;
    }
    let r = Repo::new();
    r.write("readme", "x\n");
    r.commit_all("init");
    git::lfs::track(&r.path, "*.bin", false).unwrap();
    assert!(r.read(".gitattributes").contains("*.bin filter=lfs"));
    let info = git::lfs::info(&r.path).unwrap();
    assert!(info.used && info.installed);
    assert_eq!(info.patterns, vec!["*.bin"]);

    let big: String = "0123456789abcdef".repeat(4096);
    r.write("assets/big.bin", &big);
    git::patch::stage_files(&r.path, &[".gitattributes".to_string(), "assets/big.bin".to_string()]).unwrap();
    let staged = r.staged_content("assets/big.bin");
    assert!(staged.starts_with("version https://git-lfs.github.com/spec/v1"), "staged a pointer, got {} bytes", staged.len());
    r.git(&["commit", "-q", "-m", "add big"]);

    // A checkout by libgit2 leaves the pointer in the working tree; lfs checkout fixes it.
    r.write("assets/big.bin", &staged);
    git::lfs::checkout(&r.path).unwrap();
    assert_eq!(r.read("assets/big.bin"), big);
    assert_eq!(git::lfs::info(&r.path).unwrap().files, 1);
    // Stale file times make libgit2 compare raw bytes with the pointer; status still says clean.
    let f = std::fs::File::options().write(true).open(Path::new(&r.path).join("assets/big.bin")).unwrap();
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600)).unwrap();
    drop(f);
    let un: Vec<String> = git::status::status(&r.path).unwrap().unstaged.iter().map(|f| f.path.clone()).collect();
    assert!(un.is_empty(), "downloaded LFS file shows as unchanged: {un:?} / git says {:?}", r.git(&["status", "--porcelain"]));

    // Stage all goes through LFS too.
    r.write("other.bin", "binary-ish\n");
    git::patch::stage_all(&r.path).unwrap();
    assert!(r.staged_content("other.bin").starts_with("version https://git-lfs"));
    assert_eq!(r.staged_content("readme"), "x\n");
}

#[test]
fn external_tools_get_the_right_files() {
    if cfg!(windows) {
        return; // The stand-in tools below are Unix commands.
    }
    let r = Repo::new();
    r.write("f.txt", "base\n");
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "side"]);
    r.write("f.txt", "theirs\n");
    r.commit_all("side");
    r.git(&["checkout", "-q", "main"]);
    r.write("f.txt", "mine\n");
    r.commit_all("mine");
    git::merge::merge(&r.path, "side", false).unwrap();
    // A "merge tool" that takes their version.
    let out = git::tools::merge(&r.path, "f.txt", "custom", "cp $REMOTE $MERGED").unwrap();
    assert!(!out.markers_left);
    assert_eq!(r.read("f.txt"), "theirs\n");
    // One that leaves the file alone: markers are reported.
    git::merge::abort(&r.path).unwrap();
    git::merge::merge(&r.path, "side", false).unwrap();
    assert!(git::tools::merge(&r.path, "f.txt", "custom", "true").unwrap().markers_left);
    git::merge::abort(&r.path).unwrap();

    // A "diff tool" that copies the old side somewhere we can look.
    r.write("f.txt", "edited\n");
    let dest = tempfile::tempdir().unwrap();
    let copy = dest.path().join("old.txt");
    git::tools::diff(&r.path, "f.txt", "unstaged", "custom", &format!("cp $LOCAL '{}'", copy.display())).unwrap();
    for _ in 0..50 {
        if copy.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "mine\n");
    assert!(git::tools::diff(&r.path, "f.txt", "unstaged", "custom", "").is_err());
    assert!(git::tools::list().iter().any(|t| t.id == "meld"));
}

#[test]
fn merge_prediction_grep_and_repo_settings() {
    let r = Repo::new();
    r.write("a.txt", "one\ntwo\n");
    r.write("b.txt", "bee\n");
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "side"]);
    r.write("a.txt", "one\nTWO\n");
    r.write("b.txt", "bee side\n");
    r.commit_all("side");
    r.git(&["checkout", "-q", "main"]);
    // Clean at first (a.txt untouched on main), then conflicting.
    r.write("c.txt", "c\n");
    r.commit_all("main adds c");
    let p = git::insight::predict_merge(&r.path, "side", "main").unwrap();
    assert_eq!((p.result, p.conflicts.len()), ("clean", 0));
    r.write("a.txt", "one\n2\n");
    r.write("b.txt", "bee side\n");
    r.commit_all("main edits a");
    let p = git::insight::predict_merge(&r.path, "side", "main").unwrap();
    assert_eq!(p.result, "conflicts");
    assert_eq!(p.conflicts, vec!["a.txt"]);
    assert_eq!(p.both_changed, 2);
    // Nothing was touched.
    assert!(r.git(&["status", "--porcelain"]).trim().is_empty());
    assert_eq!(git::insight::predict_merge(&r.path, "main", "main").unwrap().result, "up-to-date");

    // Search contents at a revision.
    let g = git::insight::grep(&r.path, "side", "two", false, false, 100).unwrap();
    assert_eq!(g.matches.len(), 1);
    assert_eq!((g.matches[0].path.as_str(), g.matches[0].line, g.matches[0].ranges[0]), ("a.txt", 2, [0, 3]));
    assert!(git::insight::grep(&r.path, "side", "two", true, false, 100).unwrap().matches.is_empty(), "case-sensitive");
    assert_eq!(git::insight::grep(&r.path, "main", r"^\w+$", false, true, 100).unwrap().files, 2, "a.txt and c.txt; \"bee side\" has a space");
    assert!(git::insight::grep(&r.path, "main", "(", false, true, 100).is_err());
    assert_eq!(git::insight::grep(&r.path, "main", "e", false, false, 2).unwrap().matches.len(), 2);

    // Remotes, .gitignore and config.
    r.git(&["remote", "add", "origin", "https://example.com/a.git"]);
    git::insight::edit_remote(&r.path, "origin", "upstream", "https://example.com/b.git", Some("git@example.com:b.git")).unwrap();
    let rs = git::insight::remotes(&r.path).unwrap();
    assert_eq!((rs[0].name.as_str(), rs[0].url.as_str(), rs[0].push_url.as_deref()), ("upstream", "https://example.com/b.git", Some("git@example.com:b.git")));
    git::insight::write_ignore(&r.path, "target/\n*.log").unwrap();
    assert_eq!(git::insight::read_ignore(&r.path).unwrap(), "target/\n*.log\n");
    git::insight::set_config(&r.path, "pull.rebase", Some("true")).unwrap();
    assert!(git::insight::config(&r.path).unwrap().iter().any(|e| e.key == "pull.rebase" && e.value == "true" && e.level == "local"));
    git::insight::set_config(&r.path, "pull.rebase", None).unwrap();
    assert!(git::insight::set_config(&r.path, "nodot", Some("x")).is_err());
}
