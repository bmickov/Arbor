import "@fontsource/atkinson-hyperlegible-next/400.css";
import "@fontsource/atkinson-hyperlegible-next/500.css";
import "@fontsource/atkinson-hyperlegible-next/600.css";
import "@fontsource/atkinson-hyperlegible-next/700.css";
import "@fontsource/commit-mono/400.css";
import "@fontsource/commit-mono/600.css";
import "./styles.css";
import { mount } from "svelte";
import App from "./App.svelte";

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { restoreStateCurrent, saveWindowState, StateFlags } from "@tauri-apps/plugin-window-state";
import { toasts } from "./lib/ui.svelte";
import { app as state, applyAppearance } from "./lib/state.svelte";

// Apply the saved theme and fonts before the first paint.
applyAppearance(state.settings);

// Surface unexpected errors instead of failing silently: as a toast, and on
// stderr when Arbor runs from a terminal.
const toTerminal = (level: string, message: string) => invoke("log_ui", { level, message }).catch(() => {});
for (const level of ["error", "warn"] as const) {
  const orig = console[level].bind(console);
  console[level] = (...args: unknown[]) => {
    orig(...args);
    toTerminal(level, args.map((a) => (a instanceof Error ? `${a.message}\n${a.stack}` : String(a))).join(" "));
  };
}
window.addEventListener("error", (e) => {
  toTerminal("error", `${e.message}\n${e.error?.stack ?? ""}`);
  toasts.push("error", `Unexpected error: ${e.message}`);
});
window.addEventListener("unhandledrejection", (e) => {
  toTerminal("error", `${String(e.reason)}\n${e.reason?.stack ?? ""}`);
  toasts.push("error", `Unexpected error: ${String(e.reason)}`);
});

const app = mount(App, { target: document.getElementById("app")! });

// The window starts hidden. Put it back where it was last time (if session
// restore is on), then show it, so it doesn't visibly jump.
(async () => {
  try {
    if (state.settings.restoreSession) {
      await restoreStateCurrent(StateFlags.SIZE | StateFlags.POSITION | StateFlags.MAXIMIZED | StateFlags.FULLSCREEN);
    }
  } catch (e) {
    console.warn("Couldn't restore the window size and position:", e);
  }
  await getCurrentWindow().show().catch(() => {});

  // Also save the window's geometry shortly after it moves or resizes, so a
  // crash or forced quit doesn't lose it (the plugin otherwise saves on exit).
  let t: ReturnType<typeof setTimeout> | undefined;
  const later = () => {
    clearTimeout(t);
    t = setTimeout(() => saveWindowState(StateFlags.ALL).catch((e) => console.warn("Couldn't save the window size and position:", e)), 800);
  };
  const win = getCurrentWindow();
  await win.onResized(later).catch((e) => console.warn("Window resize events unavailable:", e));
  await win.onMoved(later).catch((e) => console.warn("Window move events unavailable:", e));
})();
export default app;
