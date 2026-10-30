// Keyboard shortcuts: one registry of commands with default key bindings,
// user overrides from settings, and a single keydown dispatcher.

import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import { keyParts } from "./platform";
import * as act from "./actions";
import { app, CODE_SIZES, UI_SIZES, type View } from "./state.svelte";
import { dialog, menu } from "./ui.svelte";

export type Group = "General" | "Text size" | "Tabs" | "Views" | "Working copy" | "Remote" | "Branches & history";

export interface Command {
  id: string;
  label: string;
  group: Group;
  keys: string[];
  /** Needs an open repository. */
  repo?: boolean;
  /** Also fires while typing in a text field. */
  inInputs?: boolean;
  run: () => void;
}

function view(v: View) {
  app.view = v;
  if (v === "history" && !app.log.length) app.loadLog();
}

export const COMMANDS: Command[] = [
  // General
  { id: "prefs", label: "Open preferences", group: "General", keys: ["Ctrl+,"], inInputs: true, run: () => (app.prefs = "general") },
  { id: "shortcuts", label: "Show keyboard shortcuts", group: "General", keys: ["Ctrl+/"], inInputs: true, run: () => (app.prefs = "keyboard") },
  { id: "repos", label: "Show all repositories", group: "General", keys: ["Ctrl+T"], inInputs: true, run: () => app.switchTo(-1) },
  { id: "open", label: "Open a repository…", group: "General", keys: ["Ctrl+O"], run: async () => {
    const d = await pickFolder({ directory: true, title: "Open a repository" });
    if (typeof d === "string") app.open(d);
  } },
  { id: "sidebar", label: "Show or hide the sidebar", group: "General", keys: ["Ctrl+\\"], repo: true, run: () => (app.layout.sidebarHidden = !app.layout.sidebarHidden) },
  { id: "refresh", label: "Refresh", group: "General", keys: ["Ctrl+R", "F5"], repo: true, inInputs: true, run: () => app.refresh(true) },
  { id: "undo", label: "Undo the last operation", group: "General", keys: ["Ctrl+Z"], repo: true, run: () => app.undo() },

  // Text size
  { id: "ui.bigger", label: "Larger text", group: "Text size", keys: ["Ctrl+=", "Ctrl+Shift+="], inInputs: true, run: () => app.setUiSize(app.settings.uiSize + 1) },
  { id: "ui.smaller", label: "Smaller text", group: "Text size", keys: ["Ctrl+-"], inInputs: true, run: () => app.setUiSize(app.settings.uiSize - 1) },
  { id: "ui.reset", label: "Reset text sizes", group: "Text size", keys: ["Ctrl+0"], inInputs: true, run: () => {
    app.settings.codeSize = CODE_SIZES.default;
    app.settings.uiSize = UI_SIZES.default;
    app.saveSettings();
  } },
  { id: "code.bigger", label: "Larger code text", group: "Text size", keys: ["Ctrl+Alt+="], inInputs: true, run: () => app.setCodeSize(app.settings.codeSize + 1) },
  { id: "code.smaller", label: "Smaller code text", group: "Text size", keys: ["Ctrl+Alt+-"], inInputs: true, run: () => app.setCodeSize(app.settings.codeSize - 1) },

  // Tabs
  { id: "tab.close", label: "Close tab", group: "Tabs", keys: ["Ctrl+W"], inInputs: true, run: () => app.active >= 0 && app.closeTab(app.active) },
  { id: "tab.next", label: "Next tab", group: "Tabs", keys: ["Ctrl+Tab", "Ctrl+PageDown"], inInputs: true, run: () => app.cycleTab(1) },
  { id: "tab.prev", label: "Previous tab", group: "Tabs", keys: ["Ctrl+Shift+Tab", "Ctrl+PageUp"], inInputs: true, run: () => app.cycleTab(-1) },

  // Views
  { id: "view.changes", label: "Show changes", group: "Views", keys: ["Ctrl+1"], repo: true, inInputs: true, run: () => view("changes") },
  { id: "view.history", label: "Show history", group: "Views", keys: ["Ctrl+2"], repo: true, inInputs: true, run: () => view("history") },
  { id: "view.stashes", label: "Show stashes", group: "Views", keys: ["Ctrl+3"], repo: true, inInputs: true, run: () => view("stashes") },
  { id: "diff.split", label: "Side-by-side diffs in history", group: "Views", keys: ["Ctrl+Shift+D"], repo: true, run: () => {
    app.settings.splitDiff = !app.settings.splitDiff;
    app.saveSettings();
  } },

  // Working copy
  { id: "commit", label: "Commit", group: "Working copy", keys: ["Ctrl+Enter"], repo: true, inInputs: true, run: () => {
    view("changes");
    app.commitRequest = Date.now();
  } },
  { id: "commit.focus", label: "Write commit message", group: "Working copy", keys: ["Ctrl+M"], repo: true, inInputs: true, run: () => {
    view("changes");
    app.focusComposer = Date.now();
  } },
  { id: "stage.all", label: "Stage all changes", group: "Working copy", keys: ["Ctrl+Shift+A"], repo: true, run: () => app.act("Staging", () => api.stageAll(app.path)) },
  { id: "unstage.all", label: "Unstage all changes", group: "Working copy", keys: ["Ctrl+Shift+U"], repo: true, run: () => app.act("Unstaging", () => api.unstageAll(app.path)) },
  { id: "stash", label: "Stash changes…", group: "Working copy", keys: ["Ctrl+Shift+S"], repo: true, run: () => act.stashChanges() },
  { id: "stash.pop", label: "Apply and remove latest stash", group: "Working copy", keys: ["Ctrl+Alt+S"], repo: true, run: () => {
    if (app.stashes.length) act.applyStash(app.stashes[0].index, true);
  } },

  // Remote
  { id: "fetch", label: "Fetch", group: "Remote", keys: ["Ctrl+Shift+F"], repo: true, run: () => act.fetchAll() },
  { id: "pull", label: "Pull", group: "Remote", keys: ["Ctrl+Shift+Down"], repo: true, run: () => act.pull() },
  { id: "push", label: "Push", group: "Remote", keys: ["Ctrl+Shift+Up"], repo: true, run: () => act.push() },

  // Branches & history
  { id: "branch.switch", label: "Switch branch…", group: "Branches & history", keys: ["Ctrl+P"], repo: true, inInputs: true, run: () => (app.switcher = true) },
  { id: "branch.new", label: "New branch…", group: "Branches & history", keys: ["Ctrl+B"], repo: true, run: () => act.newBranch() },
  { id: "history.search", label: "Search history", group: "Branches & history", keys: ["Ctrl+F"], repo: true, inInputs: true, run: () => {
    view("history");
    app.focusSearch = Date.now();
  } },
];

export const GROUPS: Group[] = ["General", "Text size", "Tabs", "Views", "Working copy", "Remote", "Branches & history"];

/** Shortcuts that can't be rebound, listed for reference. */
export const FIXED: { label: string; keys: string }[] = [
  { label: "Go to tab 1–8, or the last tab", keys: "Alt+1 … Alt+9" },
  { label: "Move through files or commits", keys: "Up / Down" },
  { label: "Stage or unstage the focused file", keys: "Space" },
  { label: "Close a dialog or menu", keys: "Escape" },
];

export function bindings(id: string): string[] {
  const custom = app.settings.shortcuts[id];
  return custom ?? COMMANDS.find((c) => c.id === id)?.keys ?? [];
}

const CODE_KEYS: Record<string, string> = {
  Equal: "=", Minus: "-", Comma: ",", Period: ".", Slash: "/", Backslash: "\\", BracketLeft: "[", BracketRight: "]",
  Semicolon: ";", Quote: "'", Backquote: "`", Space: "Space", Enter: "Enter", NumpadEnter: "Enter", Tab: "Tab",
  Escape: "Escape", Backspace: "Backspace", Delete: "Delete", ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left",
  ArrowRight: "Right", PageUp: "PageUp", PageDown: "PageDown", Home: "Home", End: "End", Insert: "Insert",
  NumpadAdd: "=", NumpadSubtract: "-",
};

/** Normalise a key event into "Ctrl+Alt+Shift+Key", using physical keys for letters and digits. */
export function comboFromEvent(e: KeyboardEvent): string | null {
  if (["Control", "Shift", "Alt", "Meta", "AltGraph", "CapsLock"].includes(e.key)) return null;
  let key: string | undefined;
  if (/^Key[A-Z]$/.test(e.code)) key = e.code.slice(3);
  else if (/^Digit\d$/.test(e.code)) key = e.code.slice(5);
  else if (/^Numpad\d$/.test(e.code)) key = e.code.slice(6);
  else if (/^F\d{1,2}$/.test(e.key)) key = e.key;
  else key = CODE_KEYS[e.code] ?? (e.key === "ISO_Left_Tab" ? "Tab" : e.key.length === 1 ? e.key.toUpperCase() : e.key);
  if (!key) return null;
  const parts = [];
  if (e.ctrlKey || e.metaKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  parts.push(key);
  return parts.join("+");
}

const PRETTY: Record<string, string> = { Up: "↑", Down: "↓", Left: "←", Right: "→", Enter: "Enter", Backspace: "⌫" };
export function prettyKeys(combo: string): string[] {
  return keyParts(combo).map((k) => PRETTY[k] ?? k);
}

/** The command bound to `combo`, other than `except`. */
export function commandFor(combo: string, except?: string): Command | undefined {
  return COMMANDS.find((c) => c.id !== except && bindings(c.id).includes(combo));
}

export function setBinding(id: string, keys: string[]) {
  const def = COMMANDS.find((c) => c.id === id)?.keys ?? [];
  const next = { ...app.settings.shortcuts };
  if (keys.length === def.length && keys.every((k, i) => k === def[i])) delete next[id];
  else next[id] = keys;
  app.settings.shortcuts = next;
  app.saveSettings();
}

export function resetBindings() {
  app.settings.shortcuts = {};
  app.saveSettings();
}

/** Set while the Preferences window is recording a new binding. */
export const recording = $state({ active: false });

export function handleKey(e: KeyboardEvent) {
  if (recording.active || dialog.spec || menu.items.length || e.defaultPrevented) return;
  const combo = comboFromEvent(e);
  if (!combo) return;
  const target = e.target as HTMLElement;
  const typing = !!target.closest?.("input, textarea, select, [contenteditable]");

  // Alt+1..9 jumps to a tab (fixed).
  const m = /^Alt\+(\d)$/.exec(combo);
  if (m && m[1] !== "0") {
    e.preventDefault();
    const n = Number(m[1]);
    if (app.tabs.length) app.switchTo(n === 9 ? app.tabs.length - 1 : Math.min(n - 1, app.tabs.length - 1));
    return;
  }

  const cmd = commandFor(combo);
  if (!cmd) return;
  if (typing && !cmd.inInputs) return;
  if (cmd.repo && !app.repo) return;
  // While a modal (preferences, switcher) is open, only its own keys apply.
  if ((app.prefs || app.switcher) && !["prefs", "shortcuts"].includes(cmd.id)) return;
  e.preventDefault();
  cmd.run();
}
