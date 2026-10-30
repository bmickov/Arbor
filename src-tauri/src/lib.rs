//! Tauri entry point: exposes the git module as commands the UI can invoke.

pub mod git;

use git::diff::DiffSettings;
use git::patch::Selection;
use git::rebase::Step;
use git::remote::{Creds, Progress};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

type R<T> = std::result::Result<T, String>;

/// Run blocking libgit2 work off the main thread so the window stays responsive.
async fn blocking<T, F>(f: F) -> R<T>
where
    T: Send + 'static,
    F: FnOnce() -> git::Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

fn progress(app: &AppHandle) -> Progress {
    let app = app.clone();
    Some(Arc::new(move |s: String| {
        let _ = app.emit("git-progress", s);
    }))
}

// ------------------------------------------------------------------ repos

#[tauri::command]
async fn open_repo(path: String) -> R<git::repo::RepoInfo> {
    blocking(move || git::repo::open_repo(&path)).await
}
#[tauri::command]
async fn init_repo(path: String) -> R<git::repo::RepoInfo> {
    blocking(move || git::repo::init_repo(&path)).await
}
#[tauri::command]
async fn clone_repo(app: AppHandle, url: String, dest: String, creds: Option<Creds>) -> R<git::repo::RepoInfo> {
    let p = progress(&app);
    blocking(move || git::repo::clone_repo(&url, &dest, creds, p)).await
}
#[tauri::command]
fn recent_repos() -> Vec<git::repo::RecentRepo> {
    git::repo::recent()
}
#[tauri::command]
fn forget_repo(path: String) {
    git::repo::forget(&path)
}

// ----------------------------------------------------------------- library

#[tauri::command]
fn library_get() -> git::library::Library {
    git::library::get()
}
#[tauri::command]
fn library_save(library: git::library::Library) -> R<()> {
    git::library::save(&library).map_err(|e| e.to_string())
}
#[tauri::command]
async fn library_add_many(paths: Vec<String>, group: Option<String>) -> R<git::library::Library> {
    blocking(move || git::library::add_many(&paths, group)).await
}
#[tauri::command]
async fn scan_folder(path: String, depth: Option<usize>) -> R<Vec<String>> {
    blocking(move || git::library::scan(&path, depth.unwrap_or(4))).await
}
#[tauri::command]
async fn repo_summaries(paths: Vec<String>) -> R<Vec<git::library::Summary>> {
    blocking(move || Ok(git::library::summaries(&paths))).await
}

// ----------------------------------------------------------- working copy

#[tauri::command]
async fn get_status(repo: String) -> R<git::status::WorkingStatus> {
    blocking(move || git::status::status(&repo)).await
}
#[tauri::command]
async fn diff_unstaged(repo: String, path: String, settings: Option<DiffSettings>) -> R<Option<git::diff::FileDiff>> {
    blocking(move || git::diff::unstaged_file(&repo, &path, settings.unwrap_or_default())).await
}
#[tauri::command]
async fn diff_staged(repo: String, path: String, old_path: Option<String>, settings: Option<DiffSettings>) -> R<Option<git::diff::FileDiff>> {
    blocking(move || git::diff::staged_file(&repo, &path, old_path.as_deref(), settings.unwrap_or_default())).await
}
#[tauri::command]
async fn file_at(repo: String, rev: Option<String>, path: String) -> R<Option<Vec<u8>>> {
    blocking(move || git::diff::file_at(&repo, rev.as_deref(), &path)).await
}
#[tauri::command]
async fn stage_files(repo: String, paths: Vec<String>) -> R<()> {
    blocking(move || git::patch::stage_files(&repo, &paths)).await
}
#[tauri::command]
async fn unstage_files(repo: String, paths: Vec<String>) -> R<()> {
    blocking(move || git::patch::unstage_files(&repo, &paths)).await
}
#[tauri::command]
async fn stage_all(repo: String) -> R<()> {
    blocking(move || git::patch::stage_all(&repo)).await
}
#[tauri::command]
async fn unstage_all(repo: String) -> R<()> {
    blocking(move || git::patch::unstage_all(&repo)).await
}
#[tauri::command]
async fn discard_files(repo: String, paths: Vec<String>) -> R<()> {
    blocking(move || git::patch::discard_files(&repo, &paths)).await
}
#[tauri::command]
async fn stage_lines(repo: String, path: String, selection: Vec<Selection>, settings: Option<DiffSettings>) -> R<()> {
    blocking(move || git::patch::stage_lines(&repo, &path, &selection, settings.unwrap_or_default())).await
}
#[tauri::command]
async fn unstage_lines(repo: String, path: String, old_path: Option<String>, selection: Vec<Selection>, settings: Option<DiffSettings>) -> R<()> {
    blocking(move || git::patch::unstage_lines(&repo, &path, old_path.as_deref(), &selection, settings.unwrap_or_default())).await
}
#[tauri::command]
async fn discard_lines(repo: String, path: String, selection: Vec<Selection>, settings: Option<DiffSettings>) -> R<()> {
    blocking(move || git::patch::discard_lines(&repo, &path, &selection, settings.unwrap_or_default())).await
}
#[tauri::command]
async fn resolve_conflict(repo: String, path: String, side: String) -> R<()> {
    blocking(move || git::patch::resolve_conflict(&repo, &path, &side)).await
}

// ----------------------------------------------------------------- commits

#[tauri::command]
async fn commit(repo: String, message: String, amend: bool, skip_hooks: bool) -> R<git::commit::CommitResult> {
    blocking(move || git::commit::commit(&repo, &message, amend, skip_hooks)).await
}
#[tauri::command]
async fn head_message(repo: String) -> R<Option<String>> {
    blocking(move || git::commit::head_message(&repo)).await
}
#[tauri::command]
async fn cherry_pick(repo: String, oid: String) -> R<git::commit::CommitResult> {
    blocking(move || git::commit::cherry_pick(&repo, &oid)).await
}
#[tauri::command]
async fn revert_commit(repo: String, oid: String) -> R<git::commit::CommitResult> {
    blocking(move || git::commit::revert(&repo, &oid)).await
}
#[tauri::command]
async fn reset_to(repo: String, oid: String, mode: String) -> R<()> {
    blocking(move || git::commit::reset(&repo, &oid, &mode)).await
}

// ---------------------------------------------------------------- history

#[tauri::command]
async fn get_log(repo: String, scope: String, limit: usize) -> R<git::log::Log> {
    blocking(move || git::log::log(&repo, &scope, limit)).await
}
#[tauri::command]
async fn commit_info(repo: String, oid: String) -> R<git::log::CommitInfo> {
    blocking(move || git::log::commit_info(&repo, &oid)).await
}
#[tauri::command]
async fn commit_file_diff(repo: String, oid: String, path: String, settings: Option<DiffSettings>) -> R<Option<git::diff::FileDiff>> {
    blocking(move || git::diff::commit_file(&repo, &oid, &path, settings.unwrap_or_default())).await
}
#[tauri::command]
async fn file_history(repo: String, path: String, limit: usize) -> R<Vec<git::log::FileHistoryEntry>> {
    blocking(move || git::log::file_history(&repo, &path, limit)).await
}
#[tauri::command]
async fn blame(repo: String, path: String) -> R<git::blame::Blame> {
    blocking(move || git::blame::blame(&repo, &path)).await
}

// ---------------------------------------------------------- branches & tags

#[tauri::command]
async fn get_refs(repo: String) -> R<git::branches::Refs> {
    blocking(move || git::branches::refs(&repo)).await
}
#[tauri::command]
async fn checkout_branch(repo: String, name: String) -> R<()> {
    blocking(move || git::branches::checkout_branch(&repo, &name)).await
}
#[tauri::command]
async fn checkout_commit(repo: String, oid: String) -> R<()> {
    blocking(move || git::branches::checkout_commit(&repo, &oid)).await
}
#[tauri::command]
async fn checkout_remote(repo: String, full_name: String, local_name: Option<String>) -> R<String> {
    blocking(move || git::branches::checkout_remote(&repo, &full_name, local_name.as_deref())).await
}
#[tauri::command]
async fn create_branch(repo: String, name: String, from: Option<String>, checkout: bool) -> R<()> {
    blocking(move || git::branches::create_branch(&repo, &name, from.as_deref(), checkout)).await
}
#[tauri::command]
async fn delete_branch(repo: String, name: String, force: bool) -> R<()> {
    blocking(move || git::branches::delete_branch(&repo, &name, force)).await
}
#[tauri::command]
async fn rename_branch(repo: String, from: String, to: String) -> R<()> {
    blocking(move || git::branches::rename_branch(&repo, &from, &to)).await
}
#[tauri::command]
async fn set_upstream(repo: String, branch: String, upstream: Option<String>) -> R<()> {
    blocking(move || git::branches::set_upstream(&repo, &branch, upstream.as_deref())).await
}
#[tauri::command]
async fn create_tag(repo: String, name: String, target: String, message: Option<String>) -> R<()> {
    blocking(move || git::branches::create_tag(&repo, &name, &target, message.as_deref())).await
}
#[tauri::command]
async fn delete_tag(repo: String, name: String) -> R<()> {
    blocking(move || git::branches::delete_tag(&repo, &name)).await
}
#[tauri::command]
async fn add_remote(repo: String, name: String, url: String) -> R<()> {
    blocking(move || git::branches::add_remote(&repo, &name, &url)).await
}
#[tauri::command]
async fn remove_remote(repo: String, name: String) -> R<()> {
    blocking(move || git::branches::remove_remote(&repo, &name)).await
}

// ----------------------------------------------------------- merge/rebase

#[tauri::command]
async fn merge(repo: String, source: String, no_ff: bool) -> R<git::merge::MergeOutcome> {
    blocking(move || git::merge::merge(&repo, &source, no_ff)).await
}
#[tauri::command]
async fn merge_into(repo: String, source: String, target: String, no_ff: bool) -> R<git::merge::MergeOutcome> {
    blocking(move || git::merge::merge_into(&repo, &source, &target, no_ff)).await
}
#[tauri::command]
async fn abort_merge(repo: String) -> R<()> {
    blocking(move || git::merge::abort(&repo)).await
}
#[tauri::command]
async fn rebase(repo: String, onto: String) -> R<git::rebase::RebaseOutcome> {
    blocking(move || git::rebase::rebase(&repo, &onto)).await
}
#[tauri::command]
async fn rebase_branch(repo: String, branch: String, onto: String) -> R<git::rebase::RebaseOutcome> {
    blocking(move || git::rebase::rebase_branch(&repo, &branch, &onto)).await
}
#[tauri::command]
async fn interactive_rebase(repo: String, base: Option<String>, steps: Vec<Step>, label: String) -> R<git::rebase::RebaseOutcome> {
    blocking(move || git::rebase::interactive(&repo, base.as_deref(), &steps, &label)).await
}

// ----------------------------------------------------------------- remotes

#[tauri::command]
async fn fetch(app: AppHandle, repo: String, remote: Option<String>, prune: bool, creds: Option<Creds>) -> R<()> {
    let p = progress(&app);
    blocking(move || git::remote::fetch(&repo, remote.as_deref(), prune, creds, p)).await
}
#[tauri::command]
async fn pull(app: AppHandle, repo: String, rebase: bool, creds: Option<Creds>) -> R<git::remote::PullOutcome> {
    let p = progress(&app);
    blocking(move || git::remote::pull(&repo, rebase, creds, p)).await
}
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn push(app: AppHandle, repo: String, branch: Option<String>, remote: Option<String>, force: bool, set_upstream: bool, creds: Option<Creds>) -> R<String> {
    let p = progress(&app);
    blocking(move || git::remote::push(&repo, branch.as_deref(), remote.as_deref(), force, set_upstream, creds, p)).await
}
#[tauri::command]
async fn push_tag(app: AppHandle, repo: String, tag: String, remote: String, creds: Option<Creds>) -> R<()> {
    let p = progress(&app);
    blocking(move || git::remote::push_tag(&repo, &tag, &remote, creds, p)).await
}
#[tauri::command]
async fn delete_remote_branch(app: AppHandle, repo: String, full_name: String, creds: Option<Creds>) -> R<()> {
    let p = progress(&app);
    blocking(move || git::remote::delete_remote_branch(&repo, &full_name, creds, p)).await
}

// ------------------------------------------------------------------ stash

#[tauri::command]
async fn stash_list(repo: String) -> R<Vec<git::stash::StashEntry>> {
    blocking(move || git::stash::list(&repo)).await
}
#[tauri::command]
async fn stash_save(repo: String, message: Option<String>, include_untracked: bool, keep_index: bool, quiet: Option<bool>) -> R<()> {
    blocking(move || git::stash::save(&repo, message.as_deref(), include_untracked, keep_index, quiet.unwrap_or(false))).await
}
#[tauri::command]
async fn stash_apply(repo: String, index: usize) -> R<()> {
    blocking(move || git::stash::apply(&repo, index)).await
}
#[tauri::command]
async fn stash_pop(repo: String, index: usize, quiet: Option<bool>) -> R<()> {
    blocking(move || git::stash::pop(&repo, index, quiet.unwrap_or(false))).await
}
#[tauri::command]
async fn delete_branches(repo: String, names: Vec<String>, force: bool) -> R<usize> {
    blocking(move || git::branches::delete_branches(&repo, &names, force)).await
}

#[tauri::command]
async fn unmerged_branches(repo: String, names: Vec<String>) -> R<Vec<String>> {
    blocking(move || git::branches::unmerged(&repo, &names)).await
}

#[tauri::command]
async fn stash_drop_many(repo: String, oids: Vec<String>) -> R<usize> {
    blocking(move || git::stash::drop_many(&repo, &oids)).await
}

#[tauri::command]
async fn stash_drop(repo: String, index: usize) -> R<()> {
    blocking(move || git::stash::drop(&repo, index)).await
}
#[tauri::command]
async fn stash_branch(repo: String, index: usize, name: String) -> R<()> {
    blocking(move || git::stash::branch(&repo, index, &name)).await
}

// ------------------------------------------------------------------- undo

#[tauri::command]
async fn undo(repo: String) -> R<String> {
    blocking(move || git::undo::undo(&repo)).await
}
#[tauri::command]
async fn undo_list(repo: String) -> R<Vec<git::undo::Entry>> {
    blocking(move || git::undo::list(&repo)).await
}

// ------------------------------------------------------------------ system

/// Print a message from the web view (console errors, uncaught exceptions)
/// to stderr, so problems in the UI show up when Arbor runs from a terminal.
#[tauri::command]
fn log_ui(level: String, message: String) {
    eprintln!("[arbor ui {level}] {message}");
}

/// A folder passed on the command line (`arbor ~/code/project`), if any.
#[tauri::command]
fn startup_path() -> Option<String> {
    let arg = std::env::args().skip(1).find(|a| !a.starts_with('-'))?;
    let p = std::path::Path::new(&arg);
    let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().ok()?.join(p) };
    abs.is_dir().then(|| abs.to_string_lossy().to_string())
}

/// Open a file or folder with the desktop's default application.
#[tauri::command]
fn open_external(path: String) -> R<()> {
    std::process::Command::new("xdg-open").arg(&path).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

/// Open a terminal in `path`, trying common terminal emulators.
#[tauri::command]
fn open_terminal(path: String) -> R<()> {
    let candidates: [(&str, &[&str]); 7] = [
        ("x-terminal-emulator", &[]),
        ("gnome-terminal", &["--working-directory"]),
        ("konsole", &["--workdir"]),
        ("kgx", &["--working-directory"]),
        ("xfce4-terminal", &["--working-directory"]),
        ("alacritty", &["--working-directory"]),
        ("kitty", &["--directory"]),
    ];
    for (bin, flag) in candidates {
        let mut cmd = std::process::Command::new(bin);
        cmd.current_dir(&path);
        if let Some(f) = flag.first() {
            cmd.arg(f).arg(&path);
        }
        if cmd.spawn().is_ok() {
            return Ok(());
        }
    }
    Err("No terminal emulator was found.".into())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // Window size and position are saved on exit; the UI decides whether
        // to restore them (the "pick up where you left off" preference).
        .plugin(tauri_plugin_window_state::Builder::default().skip_initial_state("main").build())
        .setup(|app| {
            // The window starts hidden so restoring its size doesn't flicker.
            // The UI shows it once ready; this is a fallback if it never does.
            if let Some(w) = app.get_webview_window("main") {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(4));
                    if !w.is_visible().unwrap_or(true) {
                        let _ = w.show();
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_repo, init_repo, clone_repo, recent_repos, forget_repo,
            library_get, library_save, library_add_many, scan_folder, repo_summaries,
            get_status, diff_unstaged, diff_staged, file_at,
            stage_files, unstage_files, stage_all, unstage_all, discard_files,
            stage_lines, unstage_lines, discard_lines, resolve_conflict,
            commit, head_message, cherry_pick, revert_commit, reset_to,
            get_log, commit_info, commit_file_diff, file_history, blame,
            get_refs, checkout_branch, checkout_commit, checkout_remote, create_branch,
            delete_branch, delete_branches, unmerged_branches, rename_branch, set_upstream, create_tag, delete_tag,
            add_remote, remove_remote,
            merge, merge_into, abort_merge, rebase, rebase_branch, interactive_rebase,
            fetch, pull, push, push_tag, delete_remote_branch,
            stash_list, stash_save, stash_apply, stash_pop, stash_drop, stash_drop_many, stash_branch,
            undo, undo_list,
            open_external, open_terminal, startup_path, log_ui,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Arbor");
}
