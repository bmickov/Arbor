// Application state and the operations the UI triggers. Components read
// `app.*` and call the methods here; every mutating call goes through
// `app.run`, which handles busy state, errors, auth prompts and refreshes.

import { listen } from "@tauri-apps/api/event";
import {
  api, type CommitRow, type Creds, type DiffSettings, type Refs, type RepoInfo,
  type StashEntry, type UndoEntry, type WorkingStatus,
} from "./api";
import { library } from "./library.svelte";
import { applyTheme, type ThemeId } from "./themes";
import { dialog, toasts } from "./ui.svelte";

export type View = "changes" | "history" | "stashes";

/** Per-repository state that is swapped in and out when switching tabs. */
interface TabState {
  repo: RepoInfo;
  status: WorkingStatus | null;
  refs: Refs | null;
  stashes: StashEntry[];
  undoTop: UndoEntry | null;
  view: View;
  scope: string;
  log: CommitRow[];
  logMore: boolean;
  logLimit: number;
  selectedCommit: string | null;
  selectedStash: number | null;
  selectedFile: App["selectedFile"];
  lastSig: string;
}

export interface Tab {
  path: string;
  name: string;
  /** Saved state while the tab is in the background. */
  saved: TabState | null;
}

const TABS_KEY = "arbor.tabs";

const SETTINGS_KEY = "arbor.settings";
export interface Settings {
  context: number;
  ignoreWhitespace: boolean;
  pullRebase: boolean;
  splitDiff: boolean;
  theme: ThemeId;
  uiFont: "atkinson" | "system";
  /** Interface text size in px (default 13). */
  uiSize: number;
  codeFont: "commit" | "system";
  /** Code and diff text size in px (default 12). */
  codeSize: number;
  /** Custom key bindings by command id; missing ids use the defaults. */
  shortcuts: Record<string, string[]>;
}
export const DEFAULT_SETTINGS: Settings = {
  context: 3, ignoreWhitespace: false, pullRebase: false, splitDiff: false,
  theme: "system", uiFont: "atkinson", uiSize: 13, codeFont: "commit", codeSize: 12, shortcuts: {},
};
export const UI_SIZES = { min: 10, max: 20, default: 13 };
export const CODE_SIZES = { min: 9, max: 22, default: 12 };

function loadSettings(): Settings {
  try {
    return { ...DEFAULT_SETTINGS, ...JSON.parse(localStorage.getItem(SETTINGS_KEY) || "{}") };
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

const UI_FONTS = {
  atkinson: '"Atkinson Hyperlegible Next", system-ui, sans-serif',
  system: 'system-ui, "Ubuntu Sans", Ubuntu, Cantarell, "Noto Sans", sans-serif',
};
const CODE_FONTS = {
  commit: '"Commit Mono", ui-monospace, "DejaVu Sans Mono", monospace',
  system: '"Ubuntu Sans Mono", "Ubuntu Mono", ui-monospace, "DejaVu Sans Mono", monospace',
};

/** Apply theme, fonts and sizes to the document. */
export function applyAppearance(s: Settings) {
  applyTheme(s.theme);
  const root = document.documentElement.style;
  root.setProperty("--fs", `${s.uiSize / UI_SIZES.default}px`);
  root.setProperty("--cs", `${s.codeSize / CODE_SIZES.default}px`);
  root.setProperty("--font-ui", UI_FONTS[s.uiFont] ?? UI_FONTS.atkinson);
  root.setProperty("--font-mono", CODE_FONTS[s.codeFont] ?? CODE_FONTS.commit);
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

  /** Preferences window: closed, or open at a section. */
  prefs = $state<null | "appearance" | "diffs" | "keyboard">(null);
  /** Branch switcher (Ctrl+P) visibility. */
  switcher = $state(false);
  /** Times (ms) of requests from shortcuts, picked up by the views even if
   *  they mount just after: commit, focus the commit summary, focus search. */
  commitRequest = $state(0);
  focusComposer = $state(0);
  focusSearch = $state(0);
  sidebarHidden = $state(false);

  busy = $state<string | null>(null);
  progress = $state("");
  /** Bumped after each refresh so open diffs reload. */
  version = $state(0);
  settings = $state<Settings>(loadSettings());

  /** Open repositories; `active` is -1 when the repository manager is showing. */
  tabs = $state<Tab[]>([]);
  active = $state(-1);

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
    applyAppearance(this.settings);
    this.version++;
  }

  setUiSize(n: number) {
    this.settings.uiSize = Math.min(UI_SIZES.max, Math.max(UI_SIZES.min, Math.round(n)));
    this.saveSettings();
  }

  setCodeSize(n: number) {
    this.settings.codeSize = Math.min(CODE_SIZES.max, Math.max(CODE_SIZES.min, Math.round(n)));
    this.saveSettings();
  }

  get path(): string {
    return this.repo?.path ?? "";
  }

  /** Open a repository in a tab (switching to it if it's already open). */
  async open(path: string, opts: { background?: boolean } = {}) {
    let info: RepoInfo;
    try {
      info = await api.openRepo(path);
    } catch (e) {
      toasts.push("error", String(e));
      return;
    }
    library.noteOpened(info.path);
    const existing = this.tabs.findIndex((t) => t.path === info.path);
    if (existing >= 0) {
      if (!opts.background) this.switchTo(existing);
      return;
    }
    const name = library.repos.find((r) => r.path === info.path)?.name ?? info.name;
    // Open next to the current tab, like a browser.
    const at = this.active >= 0 ? this.active + 1 : this.tabs.length;
    this.tabs.splice(at, 0, { path: info.path, name, saved: null });
    if (this.active >= at) this.active++;
    this.persistTabs();
    if (!opts.background) this.switchTo(at, info);
  }

  private snapshot(): TabState | null {
    if (!this.repo) return null;
    return {
      repo: this.repo, status: this.status, refs: this.refs, stashes: this.stashes, undoTop: this.undoTop,
      view: this.view, scope: this.scope, log: this.log, logMore: this.logMore, logLimit: this.logLimit,
      selectedCommit: this.selectedCommit, selectedStash: this.selectedStash, selectedFile: this.selectedFile,
      lastSig: this.lastSig,
    };
  }

  private restore(t: TabState | null, info: RepoInfo) {
    this.repo = t?.repo ?? info;
    this.status = t?.status ?? null;
    this.refs = t?.refs ?? null;
    this.stashes = t?.stashes ?? [];
    this.undoTop = t?.undoTop ?? null;
    this.view = t?.view ?? "changes";
    this.scope = t?.scope ?? "all";
    this.log = t?.log ?? [];
    this.logMore = t?.logMore ?? false;
    this.logLimit = t?.logLimit ?? 2000;
    this.selectedCommit = t?.selectedCommit ?? null;
    this.selectedStash = t?.selectedStash ?? null;
    this.selectedFile = t?.selectedFile ?? null;
    this.lastSig = t?.lastSig ?? "";
    this.fileHistoryPath = null;
  }

  /** Make tab `i` active (-1 shows the repository manager). */
  switchTo(i: number, info?: RepoInfo) {
    if (i === this.active && this.repo) return;
    if (this.active >= 0 && this.tabs[this.active]) this.tabs[this.active].saved = this.snapshot();
    this.active = i;
    this.persistTabs();
    if (i < 0 || !this.tabs[i]) {
      this.repo = null;
      this.status = null;
      this.refs = null;
      document.title = "Repositories — Arbor";
      library.refreshSummaries();
      return;
    }
    const tab = this.tabs[i];
    this.restore(tab.saved, info ?? { path: tab.path, name: tab.name });
    tab.saved = null;
    document.title = `${tab.name} — Arbor`;
    this.workdirMayHaveChanged = true;
    this.refresh(!this.log.length);
  }

  closeTab(i: number) {
    const t = this.tabs[i];
    if (!t) return;
    if (this.busy && i === this.active) {
      toasts.push("info", `Wait for “${this.busy}” to finish before closing this tab.`);
      return;
    }
    const wasActive = i === this.active;
    if (wasActive) {
      // Don't save the closing tab's state.
      this.active = -1;
      this.repo = null;
    }
    this.tabs.splice(i, 1);
    if (!wasActive) {
      if (this.active > i) this.active--;
      this.persistTabs();
      return;
    }
    const next = this.tabs.length ? Math.min(i, this.tabs.length - 1) : -1;
    this.switchTo(next);
  }

  moveTab(from: number, to: number) {
    if (from === to || !this.tabs[from]) return;
    const activePath = this.tabs[this.active]?.path;
    const [t] = this.tabs.splice(from, 1);
    this.tabs.splice(to > from ? to - 1 : to, 0, t);
    if (activePath) this.active = this.tabs.findIndex((x) => x.path === activePath);
    this.persistTabs();
  }

  cycleTab(dir: 1 | -1) {
    const n = this.tabs.length;
    if (!n) return;
    // The manager sits before the first tab in the cycle.
    const pos = (this.active + 1 + dir + n + 1) % (n + 1);
    this.switchTo(pos - 1);
  }

  /** Keep a tab's label in sync after renaming it in the library. */
  renameTab(path: string, name: string) {
    const t = this.tabs.find((x) => x.path === path);
    if (t) t.name = name;
    if (this.repo?.path === path) document.title = `${name} — Arbor`;
  }

  private persistTabs() {
    try {
      localStorage.setItem(TABS_KEY, JSON.stringify({ paths: this.tabs.map((t) => t.path), active: this.active }));
    } catch {}
  }

  /** Reopen the tabs from last time, plus a folder given on the command line. */
  async restoreTabs(extra: string | null) {
    let saved: { paths: string[]; active: number } = { paths: [], active: -1 };
    try {
      saved = JSON.parse(localStorage.getItem(TABS_KEY) || "null") ?? saved;
      // Earlier versions remembered a single repository.
      const last = localStorage.getItem("arbor.last");
      if (!saved.paths.length && last) saved = { paths: [last], active: 0 };
      localStorage.removeItem("arbor.last");
    } catch {}
    for (const p of saved.paths) {
      try {
        const info = await api.openRepo(p);
        if (!this.tabs.some((t) => t.path === info.path)) {
          this.tabs.push({ path: info.path, name: library.repos.find((r) => r.path === info.path)?.name ?? info.name, saved: null });
        }
      } catch {
        // The folder is gone; drop the tab quietly.
      }
    }
    if (extra) {
      await this.open(extra);
      return;
    }
    const idx = Math.min(saved.active, this.tabs.length - 1);
    this.switchTo(this.tabs.length ? idx : -1);
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
    const path = this.path;
    try {
      const l = await api.log(path, this.scope, this.logLimit);
      if (path !== this.path) return;
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
    const path = this.path;
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
            await api.stashSave(path, `Arbor: before ${label.toLowerCase()}`, false, false, true);
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
          await api.stashPop(path, 0, true);
        } catch (e) {
          toasts.push("error", String(e));
        }
      }
      this.busy = null;
      this.progress = "";
      this.workdirMayHaveChanged = true;
      library.refreshSummaries([path]);
      if (this.path === path) await this.refresh();
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
