// Typed wrappers around the Rust commands. Every call goes to libgit2 on a
// worker thread; errors come back as plain strings.

import { invoke } from "@tauri-apps/api/core";

export interface RepoInfo { path: string; name: string }
export interface RecentRepo { path: string; name: string; opened: number }
/** What desktop_entry did: see src-tauri/src/desktop.rs. */
export type DesktopIntegration = "unsupported" | "packaged" | "development" | "installed" | "removed";
export interface Group { id: string; name: string; collapsed: boolean }
export interface Bookmark { path: string; name: string; group: string | null; opened: number }
export interface Library { groups: Group[]; repos: Bookmark[] }
/** How this copy of Arbor was installed, which decides how it updates (src-tauri/src/update.rs). */
export interface UpdateContext {
  version: string;
  kind: "appimage" | "deb" | "rpm" | "dmg" | "nsis" | "msi" | "manual";
  label: string;
  installable: boolean;
  releases: string;
}
export interface Release {
  version: string; tag: string; current: string; newer: boolean; name: string;
  notes: string; published: string | null; page: string;
  asset: { name: string; url: string; sha256: string } | null;
}
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
  /** A rebase Arbor paused on a conflict. */
  pausedRebase: PausedRebase | null;
  /** A bisect in progress. */
  bisect: BisectState | null;
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
  author: Person; committer: Person; parents: string[]; files: FileDiff[];
}
/** A commit with the diff of one of its files, loaded together. */
export interface CommitView { info: CommitInfo; diff: FileDiff | null }
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
export interface WorktreeInfo {
  name: string | null;
  path: string;
  branch: string | null;
  head: string | null;
  summary: string;
  isMain: boolean;
  isCurrent: boolean;
  changes: number;
  locked: string | null;
  missing: boolean;
}
export interface SubmoduleInfo {
  name: string;
  path: string;
  fullPath: string;
  url: string | null;
  branch: string | null;
  recorded: string | null;
  checkedOut: string | null;
  state: "uninitialized" | "moved" | "dirty" | "ok";
  dirty: boolean;
}
export interface FlowConfig { main: string; develop: string; feature: string; release: string; hotfix: string; versionTag: string }
export interface FinishOutcome { result: "finished" | "conflicts"; conflicts: string[]; mergingInto: string | null; tag: string | null }
export interface Stacked { branch: string; parent: string; behind: number; own: number }
export interface BisectState {
  bad: string | null; good: string[]; skipped: string[]; current: string | null;
  remaining: number; steps: number; found: string | null; ambiguous: string[]; original: string;
}
export interface LfsInfo { used: boolean; installed: boolean; patterns: string[]; missing: number; files: number }
export interface LfsLock { id: string; path: string; owner: { name: string } | null; lockedAt: string | null }
export interface ToolInfo { id: string; name: string; installed: boolean; merges: boolean }
export interface MergePrediction { result: "up-to-date" | "fast-forward" | "clean" | "conflicts"; conflicts: string[]; bothChanged: number }
export interface GrepMatch { path: string; line: number; text: string; ranges: [number, number][] }
export interface GrepResult { matches: GrepMatch[]; files: number; more: boolean }
export interface RemoteInfo { name: string; url: string; pushUrl: string | null }
export interface ConfigEntry { key: string; value: string; level: "local" | "global" | "system" | "other" }
export interface LogFilter { text?: string | null; author?: string | null; since?: number | null; until?: number | null; path?: string | null; contains?: string | null }
export interface SearchResult { rows: CommitRow[]; scanned: number; more: boolean }
export interface SigningSettings {
  enabled: boolean;
  format: "openpgp" | "ssh" | "x509";
  key: string;
  /** Where commit.gpgsign is set. */
  scope: "local" | "global" | "";
  gpgAvailable: boolean;
  sshAvailable: boolean;
  allowedSigners: string;
}
export interface Verification {
  format: "openpgp" | "ssh" | "x509";
  status: "good" | "unverified" | "bad" | "error";
  signer: string | null;
  key: string | null;
  detail: string;
}
export interface ConflictFile {
  path: string;
  /** The working-tree file, when it exists and is text. */
  text: string | null;
  /** The stages merged afresh, with diff3 markers. */
  merged: string | null;
  hasBase: boolean; hasOurs: boolean; hasTheirs: boolean; binary: boolean;
  oursLabel: string; theirsLabel: string;
  operation: "merge" | "cherry-pick" | "revert" | "rebase" | "stash";
}
export interface MergeOutcome { result: "up-to-date" | "fast-forward" | "merged" | "conflicts"; conflicts: string[] }
export interface RebaseOutcome { result: "up-to-date" | "fast-forward" | "rebased" | "unchanged" | "conflicts"; head: string; conflicts: string[] }
export interface PullOutcome { result: string; conflicts: string[] }
export interface CommitResult { oid: string; short: string; conflicts: string[] }
export interface PausedRebase { label: string; branch: string | null; step: number; total: number; action: string; commit: string; summary: string }
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
  conflictFile: (repo: string, path: string) => call<ConflictFile>("conflict_file", { repo, path }),
  resolveWithContent: (repo: string, path: string, content: string) => call<void>("resolve_with_content", { repo, path, content }),

  commit: (repo: string, message: string, amend: boolean, skipHooks: boolean, sign?: boolean) =>
    call<CommitResult>("commit", { repo, message, amend, skipHooks, sign: sign ?? null }),
  signingSettings: (repo: string) => call<SigningSettings>("signing_settings", { repo }),
  setSigning: (repo: string, enabled: boolean, format: string, key: string, global: boolean) =>
    call<void>("set_signing", { repo, enabled, format, key, global }),
  verifyCommit: (repo: string, id: string) => call<Verification | null>("verify_commit", { repo, id }),
  worktrees: (repo: string) => call<WorktreeInfo[]>("worktrees", { repo }),
  addWorktree: (repo: string, path: string, branch: string, create: boolean, start: string | null) =>
    call<string>("add_worktree", { repo, path, branch, create, start }),
  removeWorktree: (repo: string, name: string, force: boolean) => call<void>("remove_worktree", { repo, name, force }),
  pruneWorktrees: (repo: string) => call<number>("prune_worktrees", { repo }),
  submodules: (repo: string) => call<SubmoduleInfo[]>("submodules", { repo }),
  updateSubmodules: (repo: string, paths: string[], creds?: Creds) => call<number>("update_submodules", { repo, paths, creds }),
  headMessage: (repo: string) => call<string | null>("head_message", { repo }),
  cherryPick: (repo: string, oid: string) => call<CommitResult>("cherry_pick", { repo, oid }),
  revert: (repo: string, oid: string) => call<CommitResult>("revert_commit", { repo, oid }),
  reset: (repo: string, oid: string, mode: "soft" | "mixed" | "hard") => call<void>("reset_to", { repo, oid, mode }),

  log: (repo: string, scope: string, limit: number) => call<Log>("get_log", { repo, scope, limit }),
  /** How many commits the log of `scope` must load to include `oid` (null: not in it). */
  logDepth: (repo: string, scope: string, oid: string) => call<number | null>("log_depth", { repo, scope, oid }),
  commitInfo: (repo: string, oid: string) => call<CommitInfo>("commit_info", { repo, oid }),
  /** The commit, its files and the diff of `want` (or the first file) in one go. */
  commitView: (repo: string, oid: string, want: string | null, settings?: DiffSettings) =>
    call<{ info: CommitInfo; diff: WireDiff | null }>("commit_view", { repo, oid, want, settings }).then((v) => ({ info: v.info, diff: decodeDiff(v.diff) })),
  /** A commit's message, without its changes. */
  commitMessage: (repo: string, oid: string) => call<{ summary: string; body: string }>("commit_message", { repo, oid }),
  /**
   * One file of a commit. Pass `listed` (the entry from the commit's file list)
   * when you have it: then only that file is compared, not the whole commit.
   */
  commitFileDiff: (repo: string, oid: string, path: string, settings?: DiffSettings, listed?: { oldPath: string | null }) =>
    call<WireDiff | null>("commit_file_diff", { repo, oid, path, settings, oldPath: listed?.oldPath ?? null, listed: !!listed }).then(decodeDiff),
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
  rebaseContinue: (repo: string) => call<RebaseOutcome>("rebase_continue", { repo }),
  rebaseSkip: (repo: string) => call<RebaseOutcome>("rebase_skip", { repo }),
  rebaseAbort: (repo: string) => call<void>("rebase_abort", { repo }),
  /** Fold the staged changes into an earlier commit on this branch; the working tree isn't touched. */
  foldInto: (repo: string, oid: string) => call<RebaseOutcome>("fold_into", { repo, oid }),
  /** Save every current change as a stash without changing the working copy; returns its label. */
  stashSnapshot: (repo: string, name: string | null) => call<string>("stash_snapshot", { repo, name }),
  /** Snapshot the current changes, clear the working copy and restore the stash `oid`; returns the new snapshot's label. */
  stashRestoreSnapshot: (repo: string, oid: string) => call<string | null>("stash_restore_snapshot", { repo, oid }),
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
  stashSave: (repo: string, message: string | null, includeUntracked: boolean, keepIndex: boolean, quiet = false, paths?: string[]) =>
    call<void>("stash_save", { repo, message, includeUntracked, keepIndex, quiet, paths: paths ?? null }),
  flowConfig: (repo: string) => call<FlowConfig | null>("flow_config", { repo }),
  flowSuggest: (repo: string) => call<FlowConfig>("flow_suggest", { repo }),
  flowInit: (repo: string, config: FlowConfig) => call<void>("flow_init", { repo, config }),
  flowStart: (repo: string, kind: string, name: string) => call<string>("flow_start", { repo, kind, name }),
  flowFinish: (repo: string, branch: string, tagMessage: string | null, keepBranch: boolean) =>
    call<FinishOutcome>("flow_finish", { repo, branch, tagMessage, keepBranch }),
  stackList: (repo: string) => call<Stacked[]>("stack_list", { repo }),
  stackSetParent: (repo: string, branch: string, parent: string | null) => call<void>("stack_set_parent", { repo, branch, parent }),
  stackRestack: (repo: string, branch: string) => call<RebaseOutcome>("stack_restack", { repo, branch }),
  stackDescendants: (repo: string, branch: string) => call<string[]>("stack_descendants", { repo, branch }),
  bisectStart: (repo: string, bad: string | null, good: string) => call<BisectState>("bisect_start", { repo, bad, good }),
  bisectMark: (repo: string, verdict: "good" | "bad" | "skip", id: string | null = null) => call<BisectState>("bisect_mark", { repo, verdict, id }),
  bisectReset: (repo: string) => call<void>("bisect_reset", { repo }),
  lfsInfo: (repo: string) => call<LfsInfo>("lfs_info", { repo }),
  lfsCheckout: (repo: string) => call<void>("lfs_checkout", { repo }),
  lfsPull: (repo: string) => call<void>("lfs_pull", { repo }),
  lfsTrack: (repo: string, pattern: string, untrack = false) => call<void>("lfs_track", { repo, pattern, untrack }),
  lfsLocks: (repo: string) => call<LfsLock[]>("lfs_locks", { repo }),
  lfsLock: (repo: string, path: string, unlock = false, force = false) => call<void>("lfs_lock", { repo, path, unlock, force }),
  toolsList: () => call<ToolInfo[]>("tools_list"),
  diffTool: (repo: string, path: string, area: string, tool: string, custom: string) => call<void>("diff_tool", { repo, path, area, tool, custom }),
  mergeTool: (repo: string, path: string, tool: string, custom: string) => call<{ markersLeft: boolean }>("merge_tool", { repo, path, tool, custom }),
  predictMerge: (repo: string, source: string, target: string) => call<MergePrediction>("predict_merge", { repo, source, target }),
  grepTree: (repo: string, rev: string, query: string, caseSensitive: boolean, regex: boolean, limit = 2000) =>
    call<GrepResult>("grep_tree", { repo, rev, query, case: caseSensitive, regex, limit }),
  remotesInfo: (repo: string) => call<RemoteInfo[]>("remotes_info", { repo }),
  editRemote: (repo: string, name: string, newName: string, url: string, pushUrl: string | null) =>
    call<void>("edit_remote", { repo, name, newName, url, pushUrl }),
  readIgnore: (repo: string) => call<string>("read_ignore", { repo }),
  writeIgnore: (repo: string, text: string) => call<void>("write_ignore", { repo, text }),
  repoConfig: (repo: string) => call<ConfigEntry[]>("repo_config", { repo }),
  setRepoConfig: (repo: string, key: string, value: string | null) => call<void>("set_repo_config", { repo, key, value }),
  searchLog: (repo: string, scope: string, filter: LogFilter, limit: number) => call<SearchResult>("search_log", { repo, scope, filter, limit }),
  stashApply: (repo: string, index: number) => call<void>("stash_apply", { repo, index }),
  stashPop: (repo: string, index: number, quiet = false) => call<void>("stash_pop", { repo, index, quiet }),
  stashDrop: (repo: string, index: number) => call<void>("stash_drop", { repo, index }),
  /** Delete several stashes (by commit id) as one undo step. */
  stashDropMany: (repo: string, oids: string[]) => call<number>("stash_drop_many", { repo, oids }),
  stashBranch: (repo: string, index: number, name: string) => call<void>("stash_branch", { repo, index, name }),

  undo: (repo: string) => call<string>("undo", { repo }),
  undoList: (repo: string) => call<UndoEntry[]>("undo_list", { repo }),

  startupPath: () => call<string | null>("startup_path"),
  desktopEntry: (enable: boolean) => call<DesktopIntegration>("desktop_entry", { enable }),
  /** Exit the app (call after saving the session). */
  quit: () => call<void>("quit"),
  openExternal: (path: string) => call<void>("open_external", { path }),

  updateContext: () => call<UpdateContext>("update_context"),
  /** Ask GitHub for the newest release. */
  updateCheck: () => call<Release>("update_check"),
  /** Download and install the release the last check found (progress: "update-progress" events). Quit afterwards. */
  updateInstall: () => call<void>("update_install"),
  openTerminal: (path: string) => call<void>("open_terminal", { path }),
};
