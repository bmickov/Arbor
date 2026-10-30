// Typed wrappers around the Rust commands. Every call goes to libgit2 on a
// worker thread; errors come back as plain strings.

import { invoke } from "@tauri-apps/api/core";

export interface RepoInfo { path: string; name: string }
export interface RecentRepo { path: string; name: string; opened: number }
export interface Group { id: string; name: string; collapsed: boolean }
export interface Bookmark { path: string; name: string; group: string | null; opened: number }
export interface Library { groups: Group[]; repos: Bookmark[] }
export interface RepoSummary {
  path: string; ok: boolean; error: string | null; branch: string | null; detached: boolean;
  upstream: string | null; ahead: number; behind: number; changes: number; conflicts: number;
  state: string; lastCommit: number | null; lastSummary: string | null; stashes: number;
}

export type FileStatus = "added" | "modified" | "deleted" | "renamed" | "typechange" | "untracked" | "conflicted" | "copied";
export interface FileEntry { path: string; oldPath: string | null; status: FileStatus }

export interface WorkingStatus {
  staged: FileEntry[];
  unstaged: FileEntry[];
  conflicted: FileEntry[];
  state: "clean" | "merge" | "rebase" | "cherrypick" | "revert" | "other";
  branch: string | null;
  head: string | null;
  headSummary: string | null;
  detached: boolean;
  upstream: string | null;
  ahead: number;
  behind: number;
  mergeMessage: string | null;
}

export interface DiffLine { origin: " " | "+" | "-"; content: string; nl: boolean; oldNo: number | null; newNo: number | null }
export interface Hunk { header: string; oldStart: number; oldLines: number; newStart: number; newLines: number; lines: DiffLine[] }
export interface FileDiff {
  path: string; oldPath: string | null; status: FileStatus;
  binary: boolean; tooLarge: boolean;
  /** More lines changed than `maxLines`: the hunks were left out until asked for. */
  heldBack: boolean;
  additions: number; deletions: number;
  oldMode: number; newMode: number; hunks: Hunk[];
}
/** A hunk as the backend sends it: column-wise, see `impl Serialize for Hunk`. */
interface WireHunk { header: string; oldStart: number; oldLines: number; newStart: number; newLines: number; origins: string; text: string; noNl: number[] }
type WireDiff = Omit<FileDiff, "hunks"> & { hunks: WireHunk[] };

function decodeHunk(w: WireHunk): Hunk {
  const n = w.origins.length;
  const lines: DiffLine[] = new Array(n);
  const noNl = new Set(w.noNl);
  let oldNo = w.oldStart, newNo = w.newStart, pos = 0;
  for (let i = 0; i < n; i++) {
    let end = w.text.indexOf("\n", pos);
    if (end < 0 || i === n - 1) end = w.text.length;
    const origin = w.origins[i] as DiffLine["origin"];
    lines[i] = {
      origin,
      content: w.text.slice(pos, end),
      nl: !noNl.has(i),
      oldNo: origin === "+" ? null : oldNo++,
      newNo: origin === "-" ? null : newNo++,
    };
    pos = end + 1;
  }
  return { header: w.header, oldStart: w.oldStart, oldLines: w.oldLines, newStart: w.newStart, newLines: w.newLines, lines };
}

const decodeDiff = (d: WireDiff | null): FileDiff | null => (d ? { ...d, hunks: d.hunks.map(decodeHunk) } : null);

export interface ReflogEntry { oid: string; short: string; message: string; time: number; summary: string | null; author: string | null; lost: boolean }
export interface ReviewRow {
  name: string; isHead: boolean; isBase: boolean; merged: "merged" | "squashed" | "unmerged";
  ahead: number; behind: number; upstream: string | null; upstreamGone: boolean; toPush: number; toPull: number;
  time: number; author: string; summary: string;
}
export interface Review { base: string; rows: ReviewRow[] }
export interface CompareCommit { oid: string; short: string; summary: string; author: string; time: number }
export interface Comparison {
  base: string; target: string; mergeBase: string | null;
  ahead: CompareCommit[]; behind: CompareCommit[]; aheadTotal: number; behindTotal: number; files: FileDiff[];
}
export interface TreeEntry { path: string; kind: "file" | "link" | "submodule"; size: number; executable: boolean }
export interface FileContent { text: string | null; binary: boolean; size: number }
export interface Applied { files: number; subject: string | null; body: string | null }
export interface Identity { name: string | null; email: string | null; local: boolean }
export interface DiffSettings { context?: number; ignoreWhitespace?: boolean; maxLines?: number }
export interface Selection { hunk: number; lines: number[] }

export interface RefLabel { kind: "head" | "local" | "remote" | "tag"; name: string }
export interface GraphRow { col: number; color: number; width: number; segs: [number, number, number, number][] }
export interface CommitRow {
  oid: string; short: string; summary: string; author: string; email: string;
  time: number; parents: string[]; refs: RefLabel[]; graph: GraphRow;
}
export interface Log { rows: CommitRow[]; more: boolean }
export interface Person { name: string; email: string; time: number }
export interface CommitInfo {
  oid: string; short: string; summary: string; body: string;
  author: Person; committer: Person; parents: string[]; refs: RefLabel[]; files: FileDiff[];
}
export interface FileHistoryEntry { commit: CommitRow; path: string }
export interface BlameHunk { start: number; count: number; oid: string; short: string; author: string; time: number; summary: string }
export interface Blame { lines: string[]; hunks: BlameHunk[]; binary: boolean }

export interface LocalBranch {
  name: string; oid: string; isHead: boolean; upstream: string | null; upstreamGone: boolean;
  ahead: number; behind: number; summary: string; time: number;
}
export interface RemoteBranch { name: string; fullName: string; oid: string }
export interface Remote { name: string; url: string; branches: RemoteBranch[] }
export interface Tag { name: string; oid: string; annotated: boolean; message: string | null }
export interface Refs { local: LocalBranch[]; remotes: Remote[]; tags: Tag[] }

export interface StashEntry { index: number; oid: string; message: string; branch: string | null; time: number; hasUntracked: boolean }
export interface UndoEntry { label: string; time: number; action: { kind: string } }
export interface MergeOutcome { result: "up-to-date" | "fast-forward" | "merged" | "conflicts"; conflicts: string[] }
export interface RebaseOutcome { result: "up-to-date" | "fast-forward" | "rebased" | "unchanged"; head: string }
export interface PullOutcome { result: string; conflicts: string[] }
export interface CommitResult { oid: string; short: string }
export interface Creds { username: string; password: string }
export type RebaseAction = "pick" | "reword" | "squash" | "fixup" | "drop";
export interface Step { oid: string; action: RebaseAction; message?: string | null }

const call = <T>(cmd: string, args: Record<string, unknown> = {}) => invoke<T>(cmd, args);

export const api = {
  openRepo: (path: string) => call<RepoInfo>("open_repo", { path }),
  initRepo: (path: string) => call<RepoInfo>("init_repo", { path }),
  cloneRepo: (url: string, dest: string, creds?: Creds) => call<RepoInfo>("clone_repo", { url, dest, creds }),
  recentRepos: () => call<RecentRepo[]>("recent_repos"),
  forgetRepo: (path: string) => call<void>("forget_repo", { path }),

  library: () => call<Library>("library_get"),
  saveLibrary: (library: Library) => call<void>("library_save", { library }),
  addManyToLibrary: (paths: string[], group: string | null) => call<Library>("library_add_many", { paths, group }),
  scanFolder: (path: string, depth = 4) => call<string[]>("scan_folder", { path, depth }),
  summaries: (paths: string[]) => call<RepoSummary[]>("repo_summaries", { paths }),

  status: (repo: string) => call<WorkingStatus>("get_status", { repo }),
  diffUnstaged: (repo: string, path: string, settings?: DiffSettings) =>
    call<WireDiff | null>("diff_unstaged", { repo, path, settings }).then(decodeDiff),
  diffStaged: (repo: string, path: string, oldPath: string | null, settings?: DiffSettings) =>
    call<WireDiff | null>("diff_staged", { repo, path, oldPath, settings }).then(decodeDiff),
  fileAt: (repo: string, rev: string | null, path: string) => call<number[] | null>("file_at", { repo, rev, path }),
  stageFiles: (repo: string, paths: string[]) => call<void>("stage_files", { repo, paths }),
  unstageFiles: (repo: string, paths: string[]) => call<void>("unstage_files", { repo, paths }),
  stageAll: (repo: string) => call<void>("stage_all", { repo }),
  unstageAll: (repo: string) => call<void>("unstage_all", { repo }),
  discardFiles: (repo: string, paths: string[]) => call<void>("discard_files", { repo, paths }),
  stageLines: (repo: string, path: string, selection: Selection[], settings?: DiffSettings) =>
    call<void>("stage_lines", { repo, path, selection, settings }),
  unstageLines: (repo: string, path: string, oldPath: string | null, selection: Selection[], settings?: DiffSettings) =>
    call<void>("unstage_lines", { repo, path, oldPath, selection, settings }),
  discardLines: (repo: string, path: string, selection: Selection[], settings?: DiffSettings) =>
    call<void>("discard_lines", { repo, path, selection, settings }),
  resolveConflict: (repo: string, path: string, side: "ours" | "theirs") => call<void>("resolve_conflict", { repo, path, side }),

  commit: (repo: string, message: string, amend: boolean, skipHooks: boolean) =>
    call<CommitResult>("commit", { repo, message, amend, skipHooks }),
  headMessage: (repo: string) => call<string | null>("head_message", { repo }),
  cherryPick: (repo: string, oid: string) => call<CommitResult>("cherry_pick", { repo, oid }),
  revert: (repo: string, oid: string) => call<CommitResult>("revert_commit", { repo, oid }),
  reset: (repo: string, oid: string, mode: "soft" | "mixed" | "hard") => call<void>("reset_to", { repo, oid, mode }),

  log: (repo: string, scope: string, limit: number) => call<Log>("get_log", { repo, scope, limit }),
  commitInfo: (repo: string, oid: string) => call<CommitInfo>("commit_info", { repo, oid }),
  commitFileDiff: (repo: string, oid: string, path: string, settings?: DiffSettings) =>
    call<WireDiff | null>("commit_file_diff", { repo, oid, path, settings }).then(decodeDiff),
  fileHistory: (repo: string, path: string, limit = 300) => call<FileHistoryEntry[]>("file_history", { repo, path, limit }),
  blame: (repo: string, path: string) => call<Blame>("blame", { repo, path }),

  refs: (repo: string) => call<Refs>("get_refs", { repo }),
  checkoutBranch: (repo: string, name: string) => call<void>("checkout_branch", { repo, name }),
  checkoutCommit: (repo: string, oid: string) => call<void>("checkout_commit", { repo, oid }),
  checkoutRemote: (repo: string, fullName: string, localName?: string) => call<string>("checkout_remote", { repo, fullName, localName }),
  createBranch: (repo: string, name: string, from: string | null, checkout: boolean) =>
    call<void>("create_branch", { repo, name, from, checkout }),
  deleteBranch: (repo: string, name: string, force: boolean) => call<void>("delete_branch", { repo, name, force }),
  /** Delete several branches as one undo step; `force` also deletes unmerged ones. */
  deleteBranches: (repo: string, names: string[], force: boolean) => call<number>("delete_branches", { repo, names, force }),
  /** Which of these branches have commits that aren't merged or pushed. */
  unmergedBranches: (repo: string, names: string[]) => call<string[]>("unmerged_branches", { repo, names }),
  renameBranch: (repo: string, from: string, to: string) => call<void>("rename_branch", { repo, from, to }),
  setUpstream: (repo: string, branch: string, upstream: string | null) => call<void>("set_upstream", { repo, branch, upstream }),
  createTag: (repo: string, name: string, target: string, message: string | null) =>
    call<void>("create_tag", { repo, name, target, message }),
  deleteTag: (repo: string, name: string) => call<void>("delete_tag", { repo, name }),
  addRemote: (repo: string, name: string, url: string) => call<void>("add_remote", { repo, name, url }),
  removeRemote: (repo: string, name: string) => call<void>("remove_remote", { repo, name }),

  merge: (repo: string, source: string, noFf = false) => call<MergeOutcome>("merge", { repo, source, noFf }),
  mergeInto: (repo: string, source: string, target: string, noFf = false) =>
    call<MergeOutcome>("merge_into", { repo, source, target, noFf }),
  abortMerge: (repo: string) => call<void>("abort_merge", { repo }),
  rebase: (repo: string, onto: string) => call<RebaseOutcome>("rebase", { repo, onto }),
  rebaseBranch: (repo: string, branch: string, onto: string) => call<RebaseOutcome>("rebase_branch", { repo, branch, onto }),
  interactiveRebase: (repo: string, base: string | null, steps: Step[], label: string) =>
    call<RebaseOutcome>("interactive_rebase", { repo, base, steps, label }),

  getIdentity: (repo: string) => call<Identity>("get_identity", { repo }),
  /** Commit as name/email in this repository; null for both goes back to the global identity. */
  setIdentity: (repo: string, name: string | null, email: string | null) => call<void>("set_identity", { repo, name, email }),
  commitTemplate: (repo: string) => call<string | null>("commit_template", { repo }),
  ignorePattern: (repo: string, pattern: string) => call<boolean>("ignore_pattern", { repo, pattern }),
  untrackFiles: (repo: string, paths: string[]) => call<void>("untrack_files", { repo, paths }),
  restoreFile: (repo: string, rev: string, path: string) => call<void>("restore_file", { repo, rev, path }),
  saveCommitPatch: (repo: string, oid: string, dest: string) => call<void>("save_commit_patch", { repo, oid, dest }),
  saveChangesPatch: (repo: string, staged: boolean, dest: string) => call<number>("save_changes_patch", { repo, staged, dest }),
  applyPatch: (repo: string, file: string) => call<Applied>("apply_patch", { repo, file }),
  reflog: (repo: string, refname: string, limit = 300) => call<ReflogEntry[]>("reflog", { repo, refname, limit }),
  branchesReview: (repo: string, base: string | null) => call<Review>("branches_review", { repo, base }),
  compareBranches: (repo: string, base: string, target: string) => call<Comparison>("compare_branches", { repo, base, target }),
  compareFileDiff: (repo: string, base: string, target: string, path: string, settings?: DiffSettings) =>
    call<WireDiff | null>("compare_file_diff", { repo, base, target, path, settings }).then(decodeDiff),
  treeAt: (repo: string, rev: string) => call<TreeEntry[]>("tree_at", { repo, rev }),
  fileContent: (repo: string, rev: string, path: string) => call<FileContent>("file_content", { repo, rev, path }),
  exportZip: (repo: string, rev: string, dest: string, folder: string) => call<number>("export_zip", { repo, rev, dest, folder }),
  trackedFiles: (repo: string) => call<string[]>("tracked_files", { repo }),
  fetch: (repo: string, remote: string | null, prune: boolean, creds?: Creds) => call<void>("fetch", { repo, remote, prune, creds }),
  pull: (repo: string, rebase: boolean, creds?: Creds) => call<PullOutcome>("pull", { repo, rebase, creds }),
  /** `force` pushes with lease (refuses if someone pushed since the last fetch) unless `lease` is false. */
  push: (repo: string, opts: { branch?: string; remote?: string; force?: boolean; lease?: boolean; setUpstream?: boolean }, creds?: Creds) =>
    call<string>("push", {
      repo, branch: opts.branch ?? null, remote: opts.remote ?? null,
      force: opts.force ?? false, lease: opts.lease ?? true, setUpstream: opts.setUpstream ?? false, creds,
    }),
  pushTag: (repo: string, tag: string, remote: string, creds?: Creds) => call<void>("push_tag", { repo, tag, remote, creds }),
  deleteRemoteBranch: (repo: string, fullName: string, creds?: Creds) => call<void>("delete_remote_branch", { repo, fullName, creds }),

  stashes: (repo: string) => call<StashEntry[]>("stash_list", { repo }),
  stashSave: (repo: string, message: string | null, includeUntracked: boolean, keepIndex: boolean, quiet = false) =>
    call<void>("stash_save", { repo, message, includeUntracked, keepIndex, quiet }),
  stashApply: (repo: string, index: number) => call<void>("stash_apply", { repo, index }),
  stashPop: (repo: string, index: number, quiet = false) => call<void>("stash_pop", { repo, index, quiet }),
  stashDrop: (repo: string, index: number) => call<void>("stash_drop", { repo, index }),
  /** Delete several stashes (by commit id) as one undo step. */
  stashDropMany: (repo: string, oids: string[]) => call<number>("stash_drop_many", { repo, oids }),
  stashBranch: (repo: string, index: number, name: string) => call<void>("stash_branch", { repo, index, name }),

  undo: (repo: string) => call<string>("undo", { repo }),
  undoList: (repo: string) => call<UndoEntry[]>("undo_list", { repo }),

  startupPath: () => call<string | null>("startup_path"),
  /** Exit the app (call after saving the session). */
  quit: () => call<void>("quit"),
  openExternal: (path: string) => call<void>("open_external", { path }),
  openTerminal: (path: string) => call<void>("open_terminal", { path }),
};
