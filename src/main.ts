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
export default app;
