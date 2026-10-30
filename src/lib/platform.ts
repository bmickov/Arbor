// Small differences between Linux/BSD, macOS and Windows in how things are
// shown: shortcut labels and file paths.

import { homeDir } from "@tauri-apps/api/path";

const ua = typeof navigator === "undefined" ? "" : `${navigator.platform} ${navigator.userAgent}`;
export const IS_MAC = /Mac/i.test(ua);
export const IS_WINDOWS = /Win/i.test(ua);

let home = "";

/** Look up the home folder once, before the UI starts. */
export async function initPlatform() {
  try {
    home = (await homeDir()).replace(/\\/g, "/").replace(/\/+$/, "");
  } catch {
    home = "";
  }
}

/** A path for display: the home folder becomes "~", and Windows gets backslashes. */
export function shortPath(p: string): string {
  let s = p.replace(/\\/g, "/");
  const h = home || (s.match(/^\/(?:home|Users|usr\/home)\/[^/]+/)?.[0] ?? "");
  const same = (a: string, b: string) => (IS_WINDOWS ? a.toLowerCase() === b.toLowerCase() : a === b);
  if (h && (same(s, h) || same(s.slice(0, h.length + 1), `${h}/`))) s = `~${s.slice(h.length)}`;
  return IS_WINDOWS ? s.replace(/\//g, "\\") : s;
}

/** An example project path for placeholders. */
export const EXAMPLE_PATH = IS_WINDOWS ? "C:\\Users\\you\\code\\project" : IS_MAC ? "/Users/you/code/project" : "/home/you/code/project";

const MAC_KEYS: Record<string, string> = { Ctrl: "⌘", Alt: "⌥", Shift: "⇧" };

/** Key names for one combo ("Ctrl+Shift+S"), as macOS shows them (⌘ ⇧ S) when on a Mac. */
export function keyParts(combo: string): string[] {
  const parts = combo.split("+");
  return IS_MAC ? parts.map((k) => MAC_KEYS[k] ?? k) : parts;
}

/** A combo as one label: "Ctrl+T", or "⌘T" on macOS. */
export function keyLabel(combo: string): string {
  return IS_MAC ? keyParts(combo).join("") : combo;
}

/** Replace every "Ctrl+…" style combo inside a sentence with the platform's label. */
export function keyText(text: string): string {
  if (!IS_MAC) return text;
  return text.replace(/\b(?:(?:Ctrl|Alt|Shift)\+)+(?:[A-Za-z0-9]+|[^\sA-Za-z0-9])/g, (m) =>
    // "Ctrl+click" reads "⌘-click" on a Mac.
    /\+click$/.test(m) ? `${keyLabel(m.slice(0, -6))}-click` : keyLabel(m),
  );
}
