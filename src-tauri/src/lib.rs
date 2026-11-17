//! Tauri entry point: exposes the git module as commands the UI can invoke.

pub mod git;
pub mod platform;
pub mod desktop;

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

#[tauri::command]
async fn conflict_file(repo: String, path: String) -> R<git::conflict::ConflictFile> {
    blocking(move || git::conflict::conflict_file(&repo, &path)).await
}

#[tauri::command]
async fn resolve_with_content(repo: String, path: String, content: String) -> R<()> {
    blocking(move || git::conflict::resolve_with_content(&repo, &path, &content)).await
}

// ----------------------------------------------------------------- commits

#[tauri::command]
async fn commit(repo: String, message: String, amend: bool, skip_hooks: bool, sign: Option<bool>) -> R<git::commit::CommitResult> {
    blocking(move || git::commit::commit_signed(&repo, &message, amend, skip_hooks, sign)).await
}

#[tauri::command]
async fn worktrees(repo: String) -> R<Vec<git::worktree::WorktreeInfo>> {
    blocking(move || git::worktree::list(&repo)).await
}

#[tauri::command]
async fn add_worktree(repo: String, path: String, branch: String, create: bool, start: Option<String>) -> R<String> {
    blocking(move || git::worktree::add(&repo, &path, &branch, create, start.as_deref())).await
}

#[tauri::command]
async fn remove_worktree(repo: String, name: String, force: bool) -> R<()> {
    blocking(move || git::worktree::remove(&repo, &name, force)).await
}

#[tauri::command]
async fn prune_worktrees(repo: String) -> R<usize> {
    blocking(move || git::worktree::prune(&repo)).await
}

#[tauri::command]
async fn submodules(repo: String) -> R<Vec<git::worktree::SubmoduleInfo>> {
    blocking(move || git::worktree::submodules(&repo)).await
}

#[tauri::command]
async fn update_submodules(app: AppHandle, repo: String, paths: Vec<String>, creds: Option<Creds>) -> R<usize> {
    let p = progress(&app);
    blocking(move || git::worktree::update_submodules(&repo, &paths, creds, p)).await
}

#[tauri::command]
async fn signing_settings(repo: String) -> R<git::sign::SigningSettings> {
    blocking(move || git::sign::settings(&repo)).await
}

#[tauri::command]
async fn set_signing(repo: String, enabled: bool, format: String, key: String, global: bool) -> R<()> {
    blocking(move || git::sign::set_settings(&repo, enabled, &format, &key, global)).await
}

#[tauri::command]
async fn verify_commit(repo: String, id: String) -> R<Option<git::sign::Verification>> {
    blocking(move || git::sign::verify(&repo, &id)).await
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
async fn search_log(repo: String, scope: String, filter: git::log::LogFilter, limit: usize) -> R<git::log::SearchResult> {
    blocking(move || git::log::search(&repo, &scope, &filter, limit)).await
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

#[tauri::command]
async fn get_identity(repo: String) -> R<git::identity::Identity> {
    blocking(move || git::identity::identity(&repo)).await
}
#[tauri::command]
async fn set_identity(repo: String, name: Option<String>, email: Option<String>) -> R<()> {
    blocking(move || {
        let who = match (&name, &email) {
            (Some(n), Some(e)) => Some((n.as_str(), e.as_str())),
            _ => None,
        };
        git::identity::set_identity(&repo, who)
    })
    .await
}
#[tauri::command]
async fn commit_template(repo: String) -> R<Option<String>> {
    blocking(move || git::identity::commit_template(&repo)).await
}

#[tauri::command]
async fn ignore_pattern(repo: String, pattern: String) -> R<bool> {
    blocking(move || git::files::ignore(&repo, &pattern)).await
}
#[tauri::command]
async fn untrack_files(repo: String, paths: Vec<String>) -> R<()> {
    blocking(move || git::files::untrack(&repo, &paths)).await
}
#[tauri::command]
async fn restore_file(repo: String, rev: String, path: String) -> R<()> {
    blocking(move || git::files::restore_file(&repo, &rev, &path)).await
}
#[tauri::command]
async fn save_commit_patch(repo: String, oid: String, dest: String) -> R<()> {
    blocking(move || git::files::save_commit_patch(&repo, &oid, &dest)).await
}
#[tauri::command]
async fn save_changes_patch(repo: String, staged: bool, dest: String) -> R<usize> {
    blocking(move || git::files::save_changes_patch(&repo, staged, &dest)).await
}
#[tauri::command]
async fn apply_patch(repo: String, file: String) -> R<git::files::Applied> {
    blocking(move || git::files::apply_patch(&repo, &file)).await
}

#[tauri::command]
async fn reflog(repo: String, refname: String, limit: usize) -> R<Vec<git::review::ReflogEntry>> {
    blocking(move || git::review::reflog(&repo, &refname, limit)).await
}
#[tauri::command]
async fn branches_review(repo: String, base: Option<String>) -> R<git::review::Review> {
    blocking(move || git::review::branches_review(&repo, base.as_deref())).await
}
#[tauri::command]
async fn compare_branches(repo: String, base: String, target: String) -> R<git::review::Comparison> {
    blocking(move || git::review::compare(&repo, &base, &target)).await
}
#[tauri::command]
async fn compare_file_diff(repo: String, base: String, target: String, path: String, settings: Option<DiffSettings>) -> R<Option<git::diff::FileDiff>> {
    blocking(move || git::review::compare_file(&repo, &base, &target, &path, settings.unwrap_or_default())).await
}
#[tauri::command]
async fn tree_at(repo: String, rev: String) -> R<Vec<git::tree::TreeEntry>> {
    blocking(move || git::tree::tree_at(&repo, &rev)).await
}
#[tauri::command]
async fn file_content(repo: String, rev: String, path: String) -> R<git::tree::FileContent> {
    blocking(move || git::tree::file_content(&repo, &rev, &path)).await
}
#[tauri::command]
async fn export_zip(repo: String, rev: String, dest: String, folder: String) -> R<usize> {
    blocking(move || git::tree::export_zip(&repo, &rev, &dest, &folder)).await
}

#[tauri::command]
async fn tracked_files(repo: String) -> R<Vec<String>> {
    blocking(move || git::tree::tracked_files(&repo)).await
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
// ------------------------------------------------------------------ git-flow, stacks, bisect
#[tauri::command]
async fn flow_config(repo: String) -> R<Option<git::gitflow::FlowConfig>> {
    blocking(move || git::gitflow::config(&repo)).await
}
#[tauri::command]
async fn flow_suggest(repo: String) -> R<git::gitflow::FlowConfig> {
    blocking(move || git::gitflow::suggest(&repo)).await
}
#[tauri::command]
async fn flow_init(repo: String, config: git::gitflow::FlowConfig) -> R<()> {
    blocking(move || git::gitflow::init(&repo, &config)).await
}
#[tauri::command]
async fn flow_start(repo: String, kind: String, name: String) -> R<String> {
    blocking(move || git::gitflow::start(&repo, &kind, &name)).await
}
#[tauri::command]
async fn flow_finish(repo: String, branch: String, tag_message: Option<String>, keep_branch: bool) -> R<git::gitflow::FinishOutcome> {
    blocking(move || git::gitflow::finish(&repo, &branch, tag_message.as_deref(), keep_branch)).await
}
#[tauri::command]
async fn stack_list(repo: String) -> R<Vec<git::stack::Stacked>> {
    blocking(move || git::stack::list(&repo)).await
}
#[tauri::command]
async fn stack_set_parent(repo: String, branch: String, parent: Option<String>) -> R<()> {
    blocking(move || git::stack::set_parent(&repo, &branch, parent.as_deref())).await
}
#[tauri::command]
async fn stack_restack(repo: String, branch: String) -> R<git::rebase::RebaseOutcome> {
    blocking(move || git::stack::restack(&repo, &branch)).await
}
#[tauri::command]
async fn stack_descendants(repo: String, branch: String) -> R<Vec<String>> {
    blocking(move || git::stack::descendants(&repo, &branch)).await
}
#[tauri::command]
async fn bisect_start(repo: String, bad: Option<String>, good: String) -> R<git::bisect::BisectState> {
    blocking(move || git::bisect::start(&repo, bad.as_deref(), &good)).await
}
#[tauri::command]
async fn bisect_mark(repo: String, verdict: String, id: Option<String>) -> R<git::bisect::BisectState> {
    blocking(move || git::bisect::mark(&repo, &verdict, id.as_deref())).await
}
#[tauri::command]
async fn bisect_reset(repo: String) -> R<()> {
    blocking(move || git::bisect::reset(&repo)).await
}

// ------------------------------------------------------------------ prediction, search, repo settings
#[tauri::command]
async fn predict_merge(repo: String, source: String, target: String) -> R<git::insight::MergePrediction> {
    blocking(move || git::insight::predict_merge(&repo, &source, &target)).await
}
#[tauri::command]
async fn grep_tree(repo: String, rev: String, query: String, case: bool, regex: bool, limit: usize) -> R<git::insight::GrepResult> {
    blocking(move || git::insight::grep(&repo, &rev, &query, case, regex, limit)).await
}
#[tauri::command]
async fn remotes_info(repo: String) -> R<Vec<git::insight::RemoteInfo>> {
    blocking(move || git::insight::remotes(&repo)).await
}
#[tauri::command]
async fn edit_remote(repo: String, name: String, new_name: String, url: String, push_url: Option<String>) -> R<()> {
    blocking(move || git::insight::edit_remote(&repo, &name, &new_name, &url, push_url.as_deref())).await
}
#[tauri::command]
async fn read_ignore(repo: String) -> R<String> {
    blocking(move || git::insight::read_ignore(&repo)).await
}
#[tauri::command]
async fn write_ignore(repo: String, text: String) -> R<()> {
    blocking(move || git::insight::write_ignore(&repo, &text)).await
}
#[tauri::command]
async fn repo_config(repo: String) -> R<Vec<git::insight::ConfigEntry>> {
    blocking(move || git::insight::config(&repo)).await
}
#[tauri::command]
async fn set_repo_config(repo: String, key: String, value: Option<String>) -> R<()> {
    blocking(move || git::insight::set_config(&repo, &key, value.as_deref())).await
}

// ------------------------------------------------------------------ LFS and external tools
#[tauri::command]
async fn lfs_info(repo: String) -> R<git::lfs::LfsInfo> {
    blocking(move || git::lfs::info(&repo)).await
}
#[tauri::command]
async fn lfs_checkout(repo: String) -> R<()> {
    blocking(move || git::lfs::checkout(&repo)).await
}
#[tauri::command]
async fn lfs_pull(repo: String) -> R<()> {
    blocking(move || git::lfs::pull(&repo)).await
}
#[tauri::command]
async fn lfs_track(repo: String, pattern: String, untrack: bool) -> R<()> {
    blocking(move || git::lfs::track(&repo, &pattern, untrack)).await
}
#[tauri::command]
async fn lfs_locks(repo: String) -> R<Vec<git::lfs::Lock>> {
    blocking(move || git::lfs::locks(&repo)).await
}
#[tauri::command]
async fn lfs_lock(repo: String, path: String, unlock: bool, force: bool) -> R<()> {
    blocking(move || git::lfs::lock(&repo, &path, unlock, force)).await
}
#[tauri::command]
async fn tools_list() -> R<Vec<git::tools::ToolInfo>> {
    blocking(|| Ok(git::tools::list())).await
}
#[tauri::command]
async fn diff_tool(repo: String, path: String, area: String, tool: String, custom: String) -> R<()> {
    blocking(move || git::tools::diff(&repo, &path, &area, &tool, &custom)).await
}
#[tauri::command]
async fn merge_tool(repo: String, path: String, tool: String, custom: String) -> R<git::tools::MergeResult> {
    blocking(move || git::tools::merge(&repo, &path, &tool, &custom)).await
}

#[tauri::command]
async fn rebase_continue(repo: String) -> R<git::rebase::RebaseOutcome> {
    blocking(move || git::rebase::continue_rebase(&repo)).await
}
#[tauri::command]
async fn rebase_skip(repo: String) -> R<git::rebase::RebaseOutcome> {
    blocking(move || git::rebase::skip_rebase(&repo)).await
}
#[tauri::command]
async fn fold_into(repo: String, oid: String) -> R<git::rebase::RebaseOutcome> {
    blocking(move || git::rebase::fold_into(&repo, &oid)).await
}
#[tauri::command]
async fn stash_snapshot(repo: String, name: Option<String>) -> R<String> {
    blocking(move || git::stash::snapshot(&repo, name.as_deref())).await
}
#[tauri::command]
async fn stash_restore_snapshot(repo: String, oid: String) -> R<Option<String>> {
    blocking(move || git::stash::restore_snapshot(&repo, &oid)).await
}
#[tauri::command]
async fn rebase_abort(repo: String) -> R<()> {
    blocking(move || git::rebase::abort_rebase(&repo)).await
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
async fn push(app: AppHandle, repo: String, branch: Option<String>, remote: Option<String>, force: bool, lease: Option<bool>, set_upstream: bool, creds: Option<Creds>) -> R<String> {
    let p = progress(&app);
    let lease = lease.unwrap_or(true);
    blocking(move || git::remote::push(&repo, branch.as_deref(), remote.as_deref(), force, lease, set_upstream, creds, p)).await
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
async fn stash_save(repo: String, message: Option<String>, include_untracked: bool, keep_index: bool, quiet: Option<bool>, paths: Option<Vec<String>>) -> R<()> {
    blocking(move || git::stash::save_paths(&repo, message.as_deref(), include_untracked, keep_index, quiet.unwrap_or(false), &paths.unwrap_or_default())).await
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

/// Quit Arbor. The UI saves the session and window state first.
#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

/// Open a file or folder with the desktop's default application.
#[tauri::command]
fn open_external(path: String) -> R<()> {
    platform::open_path(&path)
}

/// Open a terminal window in `path`.
#[tauri::command]
fn open_terminal(path: String) -> R<()> {
    platform::open_terminal(&path)
}

/// Add, refresh or remove Arbor's own launcher and icon (Linux/BSD; see desktop.rs).
#[tauri::command]
async fn desktop_entry(enable: bool) -> Result<desktop::Integration, String> {
    tauri::async_runtime::spawn_blocking(move || desktop::integrate(enable)).await.map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Before any window: the desktop matches windows to launchers by this name.
    desktop::set_names();
    // Before any thread: libgit2's cache settings are process-wide.
    git::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
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
            stage_lines, unstage_lines, discard_lines, resolve_conflict, conflict_file, resolve_with_content, signing_settings, set_signing, verify_commit, rebase_continue, rebase_skip, rebase_abort, fold_into, stash_snapshot, stash_restore_snapshot, search_log, predict_merge, grep_tree, remotes_info, edit_remote, read_ignore, write_ignore, repo_config, set_repo_config, lfs_info, lfs_checkout, lfs_pull, lfs_track, lfs_locks, lfs_lock, tools_list, diff_tool, merge_tool, flow_config, flow_suggest, flow_init, flow_start, flow_finish, stack_list, stack_set_parent, stack_restack, stack_descendants, bisect_start, bisect_mark, bisect_reset, worktrees, add_worktree, remove_worktree, prune_worktrees, submodules, update_submodules,
            commit, head_message, cherry_pick, revert_commit, reset_to,
            get_log, commit_info, tracked_files, reflog, branches_review, compare_branches, compare_file_diff,
            tree_at, file_content, export_zip, ignore_pattern, untrack_files, restore_file,
            save_commit_patch, save_changes_patch, apply_patch, get_identity, set_identity, commit_template, commit_file_diff, file_history, blame,
            get_refs, checkout_branch, checkout_commit, checkout_remote, create_branch,
            delete_branch, delete_branches, unmerged_branches, rename_branch, set_upstream, create_tag, delete_tag,
            add_remote, remove_remote,
            merge, merge_into, abort_merge, rebase, rebase_branch, interactive_rebase,
            fetch, pull, push, push_tag, delete_remote_branch,
            stash_list, stash_save, stash_apply, stash_pop, stash_drop, stash_drop_many, stash_branch,
            undo, undo_list,
            open_external, open_terminal, quit, startup_path, log_ui, desktop_entry,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Arbor");
}
