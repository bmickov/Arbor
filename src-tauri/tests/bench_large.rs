//! Timings on a large repository: `ARBOR_BENCH_REPO=/path cargo test --release --test bench_large -- --ignored --nocapture`
use arbor_lib::git;
use std::time::Instant;

fn time<T, E: std::fmt::Display>(what: &str, f: impl Fn() -> Result<T, E>) {
    let mut best = f64::MAX;
    for _ in 0..3 {
        let t = Instant::now();
        if let Err(e) = f() { println!("BENCH {what:<40} ERROR {e}"); return; }
        best = best.min(t.elapsed().as_secs_f64() * 1000.0);
    }
    println!("BENCH {what:<40} {best:>9.1} ms");
}

#[test]
#[ignore]
fn bench() {
    if std::env::var("ARBOR_BENCH_NOINIT").is_err() {
        git::init();
    }
    let p = std::env::var("ARBOR_BENCH_REPO").expect("ARBOR_BENCH_REPO");
    let p = p.as_str();
    let ds = || git::diff::DiffSettings { context: None, ignore_whitespace: None, max_lines: Some(3000) };
    let head = git::status::status(p).unwrap().head.unwrap();
    let file = git::tree::tracked_files(p).unwrap()[12345].clone();
    time("status (refresh)", || git::status::status(p));
    time("refs (refresh)", || git::branches::refs(p));
    time("stash list (refresh)", || git::stash::list(p));
    time("undo list (refresh)", || git::undo::list(p));
    time("log all 2000", || git::log::log(p, "all", 2000));
    time("log HEAD 2000", || git::log::log(p, "HEAD", 2000));
    time("log all 5000", || git::log::log(p, "all", 5000));
    time("commit_info", || git::log::commit_info(p, &head));
    time("commit_files", || git::diff::commit_files(p, &head));
    time("file_history 300", || git::log::file_history(p, &file, 300));
    // Same commits as git finds (several threads look paths up).
    for f in [&file, &git::tree::tracked_files(p).unwrap()[3]] {
        let ours: Vec<String> = git::log::file_history(p, f, 100_000).unwrap().into_iter().map(|e| e.commit.oid).collect();
        let out = std::process::Command::new("git").args(["-C", p, "log", "--format=%H", "--", f]).output().unwrap();
        let theirs: Vec<String> = String::from_utf8(out.stdout).unwrap().lines().map(str::to_string).collect();
        let (mut a, mut b) = (ours.clone(), theirs.clone());
        a.sort();
        b.sort();
        assert_eq!(a, b, "{f}");
        println!("file history of {f}: {} commits, same as git", ours.len());
    }
    time("blame", || git::blame::blame(p, &file));
    time("tree_at HEAD", || git::tree::tree_at(p, "HEAD"));
    time("tracked_files", || git::tree::tracked_files(p));
    time("grep HEAD 'omega 3'", || git::insight::grep(p, "HEAD", "omega 3", false, false, 500));
    time("branches_review", || git::review::branches_review(p, None));
    time("reflog HEAD 500", || git::review::reflog(p, "HEAD", 500));
    time("search text", || git::log::search(p, "all", &serde_json::from_str(r#"{"text":"ENR-500"}"#).unwrap(), 2000));
    time("search author", || git::log::search(p, "all", &serde_json::from_str(r#"{"author":"bojan"}"#).unwrap(), 2000));
    time("search path", || git::log::search(p, "all", &serde_json::from_str(&format!(r#"{{"path":"{file}"}}"#)).unwrap(), 2000));
    time("compare main..feature/topic-001", || git::review::compare(p, "main", "feature/topic-001"));
    time("predict_merge", || git::insight::predict_merge(p, "feature/topic-001", "main"));
    time("stack list", || git::stack::list(p));
    time("worktree list", || git::worktree::list(p));
    time("commit_file", || git::diff::commit_file(p, &head, &git::diff::commit_files(p, &head).unwrap()[0].path, ds()));
    let first = git::diff::commit_files(p, &head).unwrap()[0].clone();
    time("commit_file listed", || git::diff::commit_file_listed(p, &head, &first.path, first.old_path.as_deref(), true, ds()));
    time("commit_view", || git::log::commit_view(p, &head, None, ds()));
    time("commit_message", || git::log::commit_message(p, &head));
    time("revwalk all topo (floor)", || {
        let repo = git::open(p).unwrap();
        let mut w = repo.revwalk().unwrap();
        w.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME).unwrap();
        w.push_glob("*").unwrap();
        let n = w.count();
        Ok::<_, git2::Error>(n)
    });
    time("walk + commit + parent", || {
        let repo = git::open(p).unwrap();
        let mut w = repo.revwalk().unwrap();
        w.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME).unwrap();
        w.push_head().unwrap();
        let mut n = 0;
        for id in w {
            let c = repo.find_commit(id.unwrap()).unwrap();
            if let Ok(pc) = c.parent(0) { n += pc.tree_id().as_bytes()[0] as usize; }
        }
        Ok::<_, git2::Error>(n)
    });
    time("walk + root + src trees", || {
        let repo = git::open(p).unwrap();
        let mut w = repo.revwalk().unwrap();
        w.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME).unwrap();
        w.push_head().unwrap();
        let mut n = 0;
        for id in w {
            let c = repo.find_commit(id.unwrap()).unwrap();
            let t = c.tree().unwrap();
            let src = repo.find_tree(t.get_name("src").unwrap().id()).unwrap();
            n += src.len();
        }
        Ok::<_, git2::Error>(n)
    });
    time("revwalk + find_commit + tree", || {
        let repo = git::open(p).unwrap();
        let mut w = repo.revwalk().unwrap();
        w.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME).unwrap();
        w.push_glob("*").unwrap();
        let mut n = 0;
        for id in w {
            let c = repo.find_commit(id.unwrap()).unwrap();
            n += c.tree().unwrap().len();
        }
        Ok::<_, git2::Error>(n)
    });
}

#[test]
#[ignore]
fn bench_dirty() {
    let p = std::env::var("ARBOR_BENCH_REPO").expect("ARBOR_BENCH_REPO");
    let p = p.as_str();
    let ds = || git::diff::DiffSettings { context: None, ignore_whitespace: None, max_lines: Some(3000) };
    time("status (dirty)", || git::status::status(p));
    let st = git::status::status(p).unwrap();
    let f = st.unstaged[0].path.clone();
    time("unstaged_file", || git::diff::unstaged_file(p, &f, ds()));
    time("stash list (dirty)", || git::stash::list(p));
    let head = git::status::status(p).unwrap().head.unwrap();
    time("commit_info (head)", || git::log::commit_info(p, &head));
    let st = git::status::status(p).unwrap();
    println!("staged {} unstaged {} json {} KB", st.staged.len(), st.unstaged.len(), serde_json::to_string(&st).unwrap().len() / 1024);
    if std::env::var("ARBOR_BENCH_STAGE").is_ok() {
        time("stage_all", || git::patch::stage_all(p));
        time("status (all staged)", || git::status::status(p));
        let st = git::status::status(p).unwrap();
        println!("staged {} json {} KB", st.staged.len(), serde_json::to_string(&st).unwrap().len() / 1024);
        time("unstage_all", || git::patch::unstage_all(p));
        let all: Vec<String> = st.staged.iter().take(2000).map(|e| e.path.clone()).collect();
        time("stage 2000 files", || git::patch::stage_files(p, &all));
        time("unstage 2000 files", || git::patch::unstage_files(p, &all));
    }
}
