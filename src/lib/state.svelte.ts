// Application state and the operations the UI triggers. Components read
// `app.*` and call the methods here; every mutating call goes through
// `app.run`, which handles busy state, errors, auth prompts and refreshes.

import { listen } from "@tauri-apps/api/event";
import {
  api, type CommitRow, type Creds, type DiffSettings, type Refs, type RepoInfo,
  type StashEntry, type UndoEntry, type WorkingStatus,
} from "./api";
import { dialog, toasts } from "./ui.svelte";

export type View = "changes" | "history" | "stashes";

const SETTINGS_KEY = "arbor.settings";
interface Settings {
  context: number;
  ignoreWhitespace: boolean;
  pullRebase: boolean;
  splitDiff: boolean;
}
function loadSettings(): Settings {
  const d: Settings = { context: 3, ignoreWhitespace: false, pullRebase: false, splitDiff: false };
  try {
    return { ...d, ...JSON.parse(localStorage.getItem(SETTINGS_KEY) || "{}") };
  } catch {
    return d;
  }
}

class App {
  repo = $state<RepoInfo | null>(null);
  status = $state<WorkingStatus | null>(null);
  refs = $state<Refs | null>(null);
  stashes = $state<StashEntry[]>([]);
  undoTop = $state<UndoEntry | null>(null);

  view = $state<View>("changes");
  scope = $state("all");
  log = $state<CommitRow[]>([]);
  logMore = $state(false);
  logLimit = $state(2000);
  logLoading = $state(false);
  selectedCommit = $state<string | null>(null);
  selectedStash = $state<number | null>(null);
  /** Path selected in the working copy, and which list it is in. */
  selectedFile = $state<{ path: string; area: "staged" | "unstaged" | "conflicted" } | null>(null);
  /** File whose history/blame panel is open. */
  fileHistoryPath = $state<string | null>(null);

  busy = $state<string | null>(null);
  progress = $state("");
  /** Bumped after each refresh so open diffs reload. */
  version = $state(0);
  settings = $state<Settings>(loadSettings());

  private refreshing = false;
  private lastSig = "";
  /** Set when the window regains focus: file contents may differ even if status doesn't. */
  workdirMayHaveChanged = false;
  private queued = false;

  constructor() {
    listen<string>("git-progress", (e) => (this.progress = e.payload)).catch(() => {});
  }

  get diffSettings(): DiffSettings {
    return { context: this.settings.context, ignoreWhitespace: this.settings.ignoreWhitespace };
  }

  saveSettings() {
    try {
      localStorage.setItem(SETTINGS_KEY, JSON.stringify(this.settings));
    } catch {}
    this.version++;
  }

  get path(): string {
    return this.repo?.path ?? "";
  }

  async open(path: string) {
    try {
      const info = await api.openRepo(path);
      this.repo = info;
      this.log = [];
      this.selectedCommit = null;
      this.selectedFile = null;
      this.selectedStash = null;
      this.fileHistoryPath = null;
      this.scope = "all";
      this.view = "changes";
      document.title = `${info.name} — Arbor`;
      await this.refresh(true);
    } catch (e) {
      toasts.push("error", String(e));
    }
  }

  close() {
    try {
      localStorage.removeItem("arbor.last");
    } catch {}
    this.repo = null;
    this.status = null;
    this.refs = null;
    document.title = "Arbor";
  }

  async refresh(withLog = false) {
    if (!this.repo) return;
    if (this.refreshing) {
      this.queued = true;
      return;
    }
    this.refreshing = true;
    const p = this.path;
    try {
      const [status, refs, stashes, undo] = await Promise.all([
        api.status(p), api.refs(p), api.stashes(p), api.undoList(p),
      ]);
      if (p !== this.path) return;
      // Skip all UI work when a background refresh finds nothing new.
      const sig = JSON.stringify([status, refs, stashes, undo[0] ?? null]);
      const changed = sig !== this.lastSig;
      this.lastSig = sig;
      if (!changed && !withLog && !this.workdirMayHaveChanged) return;
      this.workdirMayHaveChanged = false;
      this.status = status;
      this.refs = refs;
      this.stashes = stashes;
      this.undoTop = undo[0] ?? null;
      if (this.selectedStash !== null && !stashes.some((s) => s.index === this.selectedStash)) this.selectedStash = null;
      if (withLog || changed) {
        if (withLog || this.view === "history" || this.log.length > 0) await this.loadLog();
      }
      this.version++;
    } catch (e) {
      toasts.push("error", String(e));
    } finally {
      this.refreshing = false;
      if (this.queued) {
        this.queued = false;
        this.refresh(withLog);
      }
    }
  }

  async loadLog(): Promise<void> {
    if (!this.repo) return;
    this.logLoading = true;
    try {
      const l = await api.log(this.path, this.scope, this.logLimit);
      this.log = l.rows;
      this.logMore = l.more;
      if (this.selectedCommit && !l.rows.some((r) => r.oid === this.selectedCommit)) {
        this.selectedCommit = l.rows.find((r) => r.oid === this.status?.head)?.oid ?? l.rows[0]?.oid ?? null;
      }
    } catch (e) {
      // A deleted branch scope falls back to all branches.
      if (this.scope !== "all") {
        this.scope = "all";
        return this.loadLog();
      }
      toasts.push("error", String(e));
    } finally {
      this.logLoading = false;
    }
  }

  async loadMoreLog() {
    this.logLimit += 3000;
    await this.loadLog();
  }

  /**
   * Run a repository operation. `fn` receives credentials when the remote
   * asked for them on an earlier attempt.
   */
  async run<T>(label: string, fn: (creds?: Creds) => Promise<T>, opts: { success?: string | ((r: T) => string | null); quiet?: boolean } = {}): Promise<T | undefined> {
    if (!this.repo) return;
    this.busy = label;
    this.progress = "";
    let creds: Creds | undefined;
    let stashed = false;
    try {
      for (let attempt = 0; ; attempt++) {
        try {
          const r = await fn(creds);
          const msg = typeof opts.success === "function" ? opts.success(r) : opts.success;
          if (msg) toasts.push("success", msg);
          return r;
        } catch (e) {
          const err = String(e);
          if (isDirtyError(err) && !stashed && attempt < 3) {
            const ok = await dialog.confirm(
              "You have uncommitted changes",
              "Arbor can stash them, finish this, then put them back.",
              "Stash and continue",
            );
            if (!ok) return;
            await api.stashSave(this.path, `Arbor: before ${label.toLowerCase()}`, false, false, true);
            stashed = true;
            continue;
          }
          if (err.startsWith("AUTH_REQUIRED:") && attempt < 3) {
            const c = await askCredentials(err.slice("AUTH_REQUIRED:".length), attempt > 0);
            if (!c) return;
            creds = c;
            continue;
          }
          throw e;
        }
      }
    } catch (e) {
      if (!opts.quiet) showError(String(e));
      throw e;
    } finally {
      if (stashed) {
        try {
          await api.stashPop(this.path, 0, true);
        } catch (e) {
          toasts.push("error", `Your changes couldn't be put back automatically, so they're kept in the stash list. ${String(e)}`);
        }
      }
      this.busy = null;
      this.progress = "";
      this.workdirMayHaveChanged = true;
      await this.refresh();
    }
  }

  /** Like run, but swallows the rethrown error (the toast already showed it). */
  async act<T>(label: string, fn: (creds?: Creds) => Promise<T>, opts: { success?: string | ((r: T) => string | null) } = {}) {
    try {
      return await this.run(label, fn, opts);
    } catch {
      return undefined;
    }
  }

  async undo() {
    if (!this.repo || this.busy) return;
    if (!this.undoTop) {
      toasts.push("info", "Nothing to undo.");
      return;
    }
    await this.act("Undo", () => api.undo(this.path), { success: (label) => `Undid: ${label}` });
  }

  get headBranch(): string | null {
    return this.status?.branch ?? null;
  }
}

function isDirtyError(err: string) {
  return err.startsWith("Commit or stash your changes") || err.startsWith("Your local changes would be overwritten");
}

function showError(err: string) {
  const text = err.replace(/^(NOT_MERGED|REJECTED|AUTH_REQUIRED):/, "");
  toasts.push("error", text);
}

async function askCredentials(reason: string, retry: boolean): Promise<Creds | null> {
  const r = await dialog.show({
    title: retry ? "Those credentials didn't work" : "Sign in to the remote",
    body: `The remote needs a username and password. For GitHub and GitLab, use a personal access token as the password.\n\n${reason}`,
    fields: [
      { key: "username", label: "Username" },
      { key: "password", label: "Password or token", type: "password" },
    ],
    confirm: "Sign in",
  });
  if (r.button !== "confirm") return null;
  return { username: String(r.values.username ?? ""), password: String(r.values.password ?? "") };
}

export const app = new App();
