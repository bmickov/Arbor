// New versions of Arbor: checks GitHub's releases (on startup and once a day,
// unless turned off in Preferences → Updates), says when there's one, and
// installs it when asked. The work happens in src-tauri/src/update.rs.

import { listen } from "@tauri-apps/api/event";
import { api, type Release, type UpdateContext } from "./api";
import { app } from "./state.svelte";
import { toasts } from "./ui.svelte";

const CHECKED_KEY = "arbor.updateChecked";
const DAY = 24 * 60 * 60 * 1000;

function loadChecked(): number {
  try {
    return Number(localStorage.getItem(CHECKED_KEY)) || 0;
  } catch {
    return 0;
  }
}

class Updates {
  /** This copy's version and how it was installed. */
  ctx = $state.raw<UpdateContext | null>(null);
  /** What the last check found. */
  release = $state.raw<Release | null>(null);
  phase = $state<"idle" | "checking" | "installing" | "restarting">("idle");
  error = $state("");
  /** Download progress while installing. */
  progress = $state.raw<{ done: number; total: number | null } | null>(null);
  /** When GitHub was last asked (ms), across restarts. */
  checked = $state(loadChecked());

  constructor() {
    api.updateContext().then((c) => (this.ctx = c)).catch(() => {});
    listen<[number, number | null]>("update-progress", (e) => (this.progress = { done: e.payload[0], total: e.payload[1] })).catch(() => {});
  }

  /** A newer release than this one was found. */
  get available(): boolean {
    return !!this.release?.newer;
  }

  /**
   * Ask GitHub. `manual`: from the button, so errors and "up to date" show in
   * Preferences; otherwise a toast only when there's something new.
   */
  async check(manual = false) {
    if (this.phase !== "idle") return;
    this.phase = "checking";
    this.error = "";
    try {
      const r = await api.updateCheck();
      this.release = r;
      this.checked = Date.now();
      try {
        localStorage.setItem(CHECKED_KEY, String(this.checked));
      } catch {
        /* not remembered: checks again next start */
      }
      if (!manual && r.newer && app.settings.skipVersion !== r.version && app.prefs !== "updates") {
        toasts.push("info", `Arbor ${r.version} is available. You have ${r.current}.`, { label: "See what's new", run: () => (app.prefs = "updates") }, 20000);
      }
    } catch (e) {
      this.error = String(e);
    } finally {
      this.phase = "idle";
    }
  }

  /** Startup and every hour: check if automatic checks are on and the last one was a day ago. */
  auto(startup = false) {
    if (!app.settings.autoUpdate || !navigator.onLine) return;
    if (startup || Date.now() - this.checked >= DAY) this.check(false);
  }

  /** Download and install the release found, then restart into it. */
  async install() {
    const r = this.release;
    if (!r?.newer || this.phase !== "idle") return;
    if (!this.ctx?.installable || !r.asset) {
      api.openExternal(r.page).catch((e) => toasts.push("error", String(e)));
      return;
    }
    if (app.busy) {
      toasts.push("info", `Wait for “${app.busy}” to finish, then install the update.`);
      return;
    }
    this.phase = "installing";
    this.error = "";
    this.progress = null;
    try {
      await api.updateInstall();
    } catch (e) {
      this.error = String(e);
      this.phase = "idle";
      return;
    }
    this.phase = "restarting";
    await app.quit();
  }

  skip() {
    if (!this.release) return;
    app.settings.skipVersion = this.release.version;
    app.saveSettings();
  }
}

export const updates = new Updates();
