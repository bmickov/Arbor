// Application state and the operations the UI triggers. Components read
// `app.*` and call the methods here; every mutating call goes through
// `app.run`, which handles busy state, errors, auth prompts and refreshes.

import { listen } from "@tauri-apps/api/event";
import { saveWindowState, StateFlags } from "@tauri-apps/plugin-window-state";
import { SvelteSet } from "svelte/reactivity";
import {
  api, type CommitRow, type Creds, type DiffSettings, type Refs, type RepoInfo,
  type StashEntry, type UndoEntry, type WorkingStatus,
} from "./api";
import { library } from "./library.svelte";
import { applyTheme, type ThemeId } from "./themes";
import { dialog, toasts } from "./ui.svelte";

export type View = "changes" | "history" | "stashes" | "review" | "reflog" | "compare" | "browse";

/** What a tab was showing. Saved across restarts when session restore is on. */
export interface TabUI {
  view: View;
  scope: string;
  selectedCommit: string | null;
  selectedStash: number | null;
  selectedFile: { path: string; area: "staged" | "unstaged" | "conflicted" } | null;
  historyQuery: string;
  historyScroll: number;
  logLimit: number;
  fileHistoryPath: string | null;
  /** The two branches being compared (Compare view). */
  compare?: { base: string; target: string } | null;
  /** The commit, branch or tag whose files are shown (Browse view). */
  browse?: { rev: string; label: string } | null;
}

/** Data loaded for a tab, kept in memory so switching tabs is instant. */
interface TabData {
  repo: RepoInfo;
  status: WorkingStatus | null;
  refs: Refs | null;
  stashes: StashEntry[];
  undoTop: UndoEntry | null;
  log: CommitRow[];
  logMore: boolean;
  lastSig: string;
}

type TabState = TabUI & Partial<TabData>;

export interface Tab {
  path: string;
  name: string;
  /** Saved state while the tab is in the background. */
  saved: TabState | null;
}

/** Panel sizes and expanded sections, saved with the session. */
export interface Layout {
  sideW: number;
  listW: number;
  sidebarHidden: boolean;
  sections: { branches: boolean; remotes: boolean; tags: boolean };
  remotes: Record<string, boolean>;
}
const DEFAULT_LAYOUT: Layout = {
  sideW: 240, listW: 55, sidebarHidden: false,
  sections: { branches: true, remotes: true, tags: false }, remotes: {},
};

interface Session {
  version: 1;
  active: number;
  tabs: { path: string; ui: TabUI | null }[];
  layout: Layout;
}

const SESSION_KEY = "arbor.session";
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
  /** Changing the interface size changes the code size by the same amount. */
  codeFollowsUi: boolean;
  /** Ask before showing diffs with more changed lines than this; 0 never asks. */
  largeDiffLines: number;
  /** Fetch open repositories in the background every N minutes; 0 is off. */
  autoFetchMinutes: number;
  /** Saved commit message templates (first line is the summary). */
  commitTemplates: { name: string; text: string }[];
  /** Author identities to switch between per repository. */
  profiles: { name: string; email: string }[];
  /** Custom key bindings by command id; missing ids use the defaults. */
  shortcuts: Record<string, string[]>;
  /** Reopen tabs, views, selections, layout and window size on startup. */
  restoreSession: boolean;
}
export const DEFAULT_SETTINGS: Settings = {
  context: 3, ignoreWhitespace: false, pullRebase: false, splitDiff: false,
  theme: "system", uiFont: "atkinson", uiSize: 13, codeFont: "commit", codeSize: 12, shortcuts: {},
  restoreSession: true, codeFollowsUi: true, largeDiffLines: 10000, autoFetchMinutes: 10,
  commitTemplates: [], profiles: [],
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
  compare = $state<{ base: string; target: string } | null>(null);
  browse = $state<{ rev: string; label: string } | null>(null);
  historyQuery = $state("");
  historyScroll = $state(0);
  layout = $state<Layout>(structuredClone(DEFAULT_LAYOUT));

  /** Preferences window: closed, or open at a section. */
  prefs = $state<null | "general" | "appearance" | "diffs" | "commits" | "keyboard">(null);
  /** Branch switcher (Ctrl+P) visibility. */
  switcher = $state(false);
  /** A commit message for the composer to pick up (from an applied email patch). */
  prefillMessage = $state<{ subject: string; body: string; at: number } | null>(null);
  /** Quick Actions palette (Ctrl+Shift+P) visibility. */
  quick = $state(false);
  /** Times (ms) of requests from shortcuts, picked up by the views even if
   *  they mount just after: commit, focus the commit summary, focus search. */
  commitRequest = $state(0);
  focusComposer = $state(0);
  focusSearch = $state(0);
  /** Set to focus the repository list's filter box. */
  focusRepoFilter = $state(0);
  /** When each repository was last fetched (ms), by path. */
  lastFetched = $state<Record<string, number>>({});
  /** Repositories whose background fetch failed (e.g. needs a password); skipped until restart. */
  private autoFetchOff = new Set<string>();
  private autoFetching = false;
  /** Recently closed tabs, newest last, for "Reopen closed tab". */
  closedTabs: { path: string; name: string; ui: TabUI | null }[] = [];

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
    return { context: this.settings.context, ignoreWhitespace: this.settings.ignoreWhitespace, maxLines: this.settings.largeDiffLines };
  }

  /** Large diffs the user chose to show this session ("repo|commit or area|path"). */
  largeDiffs = new SvelteSet<string>();

  /** Diff settings for one file: no line cap once its large diff was shown. */
  diffSettingsFor(key: string): DiffSettings {
    const s = this.diffSettings;
    return this.largeDiffs.has(key) ? { ...s, maxLines: 0 } : s;
  }

  saveSettings() {
    try {
      localStorage.setItem(SETTINGS_KEY, JSON.stringify(this.settings));
    } catch {}
    applyAppearance(this.settings);
    // No reload needed: diffs re-fetch on their own when a setting they use
    // (context, whitespace, large-diff limit) changes, and text-size changes
    // are pure CSS — reloading would refetch big diffs on every Ctrl+=.
  }

  setUiSize(n: number) {
    const before = this.settings.uiSize;
    this.settings.uiSize = Math.min(UI_SIZES.max, Math.max(UI_SIZES.min, Math.round(n)));
    // Code keeps its offset from the interface size (12 vs 13 by default), so
    // zooming with Ctrl+= / Ctrl+- scales diffs too.
    if (this.settings.codeFollowsUi) {
      const d = this.settings.uiSize - before;
      this.settings.codeSize = Math.min(CODE_SIZES.max, Math.max(CODE_SIZES.min, this.settings.codeSize + d));
    }
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
  async open(path: string, opts: { background?: boolean; ui?: TabUI | null } = {}) {
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
    this.tabs.splice(at, 0, { path: info.path, name, saved: opts.ui ? { ...opts.ui } : null });
    if (this.active >= at) this.active++;
    this.persistTabs();
    if (!opts.background) this.switchTo(at, info);
  }

  /** Just the view-and-selection part of a tab's saved state. */
  private pickUI(t: TabState): TabUI {
    const { view, scope, selectedCommit, selectedStash, selectedFile, historyQuery, historyScroll, logLimit, fileHistoryPath, compare, browse } = t;
    return { view, scope, selectedCommit, selectedStash, selectedFile, historyQuery, historyScroll, logLimit, fileHistoryPath, compare, browse };
  }

  private uiSnapshot(): TabUI {
    return {
      view: this.view, scope: this.scope, selectedCommit: this.selectedCommit, selectedStash: this.selectedStash,
      selectedFile: this.selectedFile, historyQuery: this.historyQuery, historyScroll: this.historyScroll,
      logLimit: this.logLimit, fileHistoryPath: this.fileHistoryPath,
      compare: this.compare ? { ...this.compare } : null, browse: this.browse ? { ...this.browse } : null,
    };
  }

  private snapshot(): TabState | null {
    if (!this.repo) return null;
    return {
      ...this.uiSnapshot(),
      repo: this.repo, status: this.status, refs: this.refs, stashes: this.stashes, undoTop: this.undoTop,
      log: this.log, logMore: this.logMore, lastSig: this.lastSig,
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
    this.historyQuery = t?.historyQuery ?? "";
    this.historyScroll = t?.historyScroll ?? 0;
    this.fileHistoryPath = t?.fileHistoryPath ?? null;
    this.compare = t?.compare ?? null;
    this.browse = t?.browse ?? null;
    // A view that needs something it no longer has falls back to history.
    if ((this.view === "compare" && !this.compare) || (this.view === "browse" && !this.browse)) this.view = "history";
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
    // Remember it (with its view and selection) for "Reopen closed tab".
    const ui = wasActive && this.repo ? this.uiSnapshot() : t.saved ? this.pickUI(t.saved) : null;
    this.closedTabs = [...this.closedTabs.filter((c) => c.path !== t.path), { path: t.path, name: t.name, ui }].slice(-10);
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

  noteFetched(path: string) {
    this.lastFetched[path] = Date.now();
  }

  /**
   * Background fetch, run every half minute: fetches each open repository
   * whose last fetch is older than the "fetch automatically" interval. It's
   * silent: no busy state, no sign-in prompts. A repository that fails (it
   * needs credentials, or is offline) is left alone until Arbor restarts.
   */
  async autoFetch() {
    const every = this.settings.autoFetchMinutes * 60_000;
    if (!every || this.autoFetching || !navigator.onLine) return;
    const now = Date.now();
    const paths = [this.path, ...this.tabs.map((t) => t.path)].filter((p, i, a) => p && a.indexOf(p) === i);
    const due = paths.filter((p) => !this.autoFetchOff.has(p) && now - (this.lastFetched[p] ?? 0) >= every);
    if (!due.length) return;
    this.autoFetching = true;
    try {
      for (const p of due) {
        if (p === this.path && this.busy) continue;
        try {
          await api.fetch(p, null, false);
          this.noteFetched(p);
          if (p === this.path) await this.refresh();
          else await library.refreshSummaries([p]);
        } catch {
          this.autoFetchOff.add(p);
        }
      }
    } finally {
      this.autoFetching = false;
    }
  }

  /** Reopen the most recently closed tab, as it was. */
  async reopenClosedTab() {
    const c = this.closedTabs.pop();
    if (!c) {
      toasts.push("info", "No closed tabs to reopen.");
      return;
    }
    await this.open(c.path, { ui: c.ui });
  }

  /** Move the current tab one place left (-1) or right (1). */
  shiftTab(dir: 1 | -1) {
    const i = this.active;
    const to = i + dir;
    if (i < 0 || to < 0 || to >= this.tabs.length) return;
    this.moveTab(i, dir > 0 ? i + 2 : i - 1);
  }

  /** Save the session and window size, then exit. Asks first if an operation is still running. */
  async quit() {
    if (this.busy) {
      const ok = await dialog.confirm(
        "Quit while Arbor is busy?",
        `“${this.busy}” hasn't finished yet. Quitting now may stop it halfway.`,
        "Quit anyway",
        true,
      );
      if (!ok) return;
    }
    this.saveSession();
    await saveWindowState(StateFlags.ALL).catch(() => {});
    await api.quit();
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

  /** Everything needed to pick up where the user left off. */
  session(): Session {
    const pick = (t: TabState | null): TabUI | null => (t ? this.pickUI(t) : null);
    return {
      version: 1,
      active: this.active,
      tabs: this.tabs.map((t, i) => ({ path: t.path, ui: i === this.active && this.repo ? this.uiSnapshot() : pick(t.saved) })),
      layout: $state.snapshot(this.layout),
    };
  }

  /** False until startup has restored the last session. Saving before then
   *  would overwrite the saved tabs with the empty startup state, and lose
   *  them when restoring is slow (many repositories, a slow disk). */
  private sessionRestored = false;

  saveSession() {
    if (!this.sessionRestored) return;
    try {
      if (this.settings.restoreSession) localStorage.setItem(SESSION_KEY, JSON.stringify(this.session()));
      else localStorage.removeItem(SESSION_KEY);
    } catch {}
  }

  forgetSession() {
    try {
      for (const k of [SESSION_KEY, TABS_KEY, "arbor.last"]) localStorage.removeItem(k);
    } catch {}
  }

  private persistTabs() {
    this.saveSession();
  }

  private loadSession(): Session | null {
    try {
      const s = JSON.parse(localStorage.getItem(SESSION_KEY) || "null") as Session | null;
      if (s?.version === 1) return s;
      // Older versions saved only the open tabs, or a single repository.
      const tabs = JSON.parse(localStorage.getItem(TABS_KEY) || "null") as { paths: string[]; active: number } | null;
      const last = localStorage.getItem("arbor.last");
      const paths = tabs?.paths?.length ? tabs.paths : last ? [last] : [];
      return { version: 1, active: tabs?.active ?? 0, tabs: paths.map((path) => ({ path, ui: null })), layout: structuredClone(DEFAULT_LAYOUT) };
    } catch {
      return null;
    }
  }

  /**
   * On startup: reopen last session's tabs and layout (when session restore
   * is on), then a folder given on the command line.
   */
  async restoreTabs(extra: string | null) {
    try {
      await this.restoreTabsInner(extra);
    } finally {
      this.sessionRestored = true;
      this.saveSession();
    }
  }

  private async restoreTabsInner(extra: string | null) {
    const session = this.settings.restoreSession ? this.loadSession() : null;
    if (!this.settings.restoreSession) this.forgetSession();
    try {
      localStorage.removeItem(TABS_KEY);
      localStorage.removeItem("arbor.last");
    } catch {}
    if (session) {
      this.layout = { ...structuredClone(DEFAULT_LAYOUT), ...session.layout, sections: { ...DEFAULT_LAYOUT.sections, ...session.layout?.sections } };
      for (const t of session.tabs) {
        try {
          const info = await api.openRepo(t.path);
          if (this.tabs.some((x) => x.path === info.path)) continue;
          const name = library.repos.find((r) => r.path === info.path)?.name ?? info.name;
          this.tabs.push({ path: info.path, name, saved: t.ui ? { ...t.ui } : null });
        } catch {
          // The folder is gone; drop the tab quietly.
        }
      }
    }
    if (extra) {
      await this.open(extra);
      return;
    }
    const idx = Math.min(session?.active ?? -1, this.tabs.length - 1);
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
