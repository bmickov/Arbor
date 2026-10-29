// The repository library: bookmarked repositories in groups, with live
// status summaries for the repository manager and tab bar.

import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { api, type Bookmark, type Creds, type Group, type Library, type RepoSummary } from "./api";
import { dialog, toasts } from "./ui.svelte";

class LibraryState {
  groups = $state<Group[]>([]);
  repos = $state<Bookmark[]>([]);
  summaries = $state<Record<string, RepoSummary>>({});
  loading = $state(false);
  /** Path → what a bulk operation is doing to it right now. */
  working = $state<Record<string, string>>({});
  bulk = $state<string | null>(null);
  private loaded = false;
  private refreshing = false;

  async load() {
    const lib = await api.library();
    this.groups = lib.groups;
    this.repos = lib.repos;
    this.loaded = true;
  }

  async ensure() {
    if (!this.loaded) await this.load();
  }

  private async save() {
    const lib: Library = { groups: $state.snapshot(this.groups), repos: $state.snapshot(this.repos) };
    try {
      await api.saveLibrary(lib);
    } catch (e) {
      toasts.push("error", `Couldn't save the repository list: ${e}`);
    }
  }

  async refreshSummaries(paths?: string[]) {
    if (this.refreshing && !paths) return;
    const list = paths ?? this.repos.map((r) => r.path);
    if (!list.length) return;
    if (!paths) this.refreshing = true;
    this.loading = true;
    try {
      const sums = await api.summaries(list);
      const next = { ...this.summaries };
      for (const s of sums) next[s.path] = s;
      this.summaries = next;
    } catch {
      // Summaries are best-effort; the manager just shows less detail.
    } finally {
      this.loading = false;
      if (!paths) this.refreshing = false;
    }
  }

  /** Called after a repository is opened: the backend has already bookmarked it. */
  async noteOpened(path: string) {
    const was = this.repos.find((r) => r.path === path);
    if (was) {
      was.opened = Math.floor(Date.now() / 1000);
    } else {
      await this.load();
    }
  }

  async addFolders() {
    const dir = await pickFolder({ directory: true, multiple: true, title: "Add repositories" });
    const dirs = Array.isArray(dir) ? dir : typeof dir === "string" ? [dir] : [];
    const added: string[] = [];
    for (const d of dirs) {
      try {
        const info = await api.openRepo(d);
        added.push(info.path);
      } catch (e) {
        toasts.push("error", String(e));
      }
    }
    if (added.length) {
      await this.load();
      await this.refreshSummaries(added);
      toasts.push("success", added.length === 1 ? "Added 1 repository." : `Added ${added.length} repositories.`);
    }
  }

  async scanFolder() {
    const dir = await pickFolder({ directory: true, title: "Find repositories in a folder" });
    if (typeof dir !== "string") return;
    this.bulk = "Looking for repositories…";
    let found: string[] = [];
    try {
      found = await api.scanFolder(dir, 4);
    } catch (e) {
      toasts.push("error", String(e));
      return;
    } finally {
      this.bulk = null;
    }
    const known = new Set(this.repos.map((r) => r.path));
    const fresh = found.filter((p) => !known.has(p));
    if (!fresh.length) {
      toasts.push("info", found.length ? "All repositories in that folder are already in your list." : "No Git repositories were found in that folder.");
      return;
    }
    const folderName = dir.replace(/\/+$/, "").split("/").pop() || dir;
    const r = await dialog.show({
      title: `Found ${fresh.length} ${fresh.length === 1 ? "repository" : "repositories"}`,
      body: fresh.slice(0, 12).map((p) => p.replace(dir.replace(/\/+$/, "") + "/", "")).join("\n") + (fresh.length > 12 ? `\n…and ${fresh.length - 12} more` : ""),
      fields: [{ key: "group", label: `Put them in a group named “${folderName}”`, type: "checkbox", checked: fresh.length > 1 }],
      confirm: `Add ${fresh.length}`,
    });
    if (r.button !== "confirm") return;
    let group: string | null = null;
    if (r.values.group) {
      group = this.groups.find((g) => g.name === folderName)?.id ?? null;
      if (!group) {
        group = this.newGroupId();
        this.groups.push({ id: group, name: folderName, collapsed: false });
        await this.save();
      }
    }
    const lib = await api.addManyToLibrary(fresh, group);
    this.groups = lib.groups;
    this.repos = lib.repos;
    toasts.push("success", `Added ${fresh.length} ${fresh.length === 1 ? "repository" : "repositories"}.`);
    await this.refreshSummaries(fresh);
  }

  /** Returns the cloned repository's path, or null. */
  async clone(): Promise<string | null> {
    const r = await dialog.show({
      title: "Clone a repository",
      fields: [
        { key: "url", label: "Repository URL", placeholder: "https://github.com/owner/project.git" },
        { key: "dest", label: "Save to folder", placeholder: "/home/you/code/project", hint: "Leave empty to choose a parent folder next." },
      ],
      confirm: "Clone",
      validate: (v) => (String(v.url ?? "").trim() ? null : "Enter the repository URL."),
    });
    if (r.button !== "confirm") return null;
    const url = String(r.values.url).trim();
    let dest = String(r.values.dest ?? "").trim();
    if (!dest) {
      const parent = await pickFolder({ directory: true, title: "Choose where to clone into" });
      if (typeof parent !== "string") return null;
      const name = url.replace(/\/+$/, "").split(/[/:]/).pop()!.replace(/\.git$/, "");
      dest = `${parent.replace(/\/+$/, "")}/${name}`;
    }
    this.bulk = `Cloning ${url.split(/[/:]/).pop()?.replace(/\.git$/, "")}…`;
    let creds: Creds | undefined;
    try {
      for (;;) {
        try {
          const info = await api.cloneRepo(url, dest, creds);
          await this.load();
          return info.path;
        } catch (e) {
          const err = String(e);
          if (!err.startsWith("AUTH_REQUIRED:")) throw e;
          const c = await dialog.show({
            title: "Sign in to clone",
            body: "For GitHub and GitLab, use a personal access token as the password.",
            fields: [
              { key: "username", label: "Username" },
              { key: "password", label: "Password or token", type: "password" },
            ],
            confirm: "Sign in",
          });
          if (c.button !== "confirm") return null;
          creds = { username: String(c.values.username), password: String(c.values.password) };
        }
      }
    } catch (e) {
      toasts.push("error", String(e));
      return null;
    } finally {
      this.bulk = null;
    }
  }

  async create(): Promise<string | null> {
    const dir = await pickFolder({ directory: true, title: "Choose a folder for the new repository" });
    if (typeof dir !== "string") return null;
    try {
      const info = await api.initRepo(dir);
      await this.load();
      return info.path;
    } catch (e) {
      toasts.push("error", String(e));
      return null;
    }
  }

  /** Fetch every repository (or the given ones) a few at a time. */
  async fetchAll(paths?: string[]) {
    const list = (paths ?? this.repos.map((r) => r.path)).filter((p) => this.summaries[p]?.ok !== false);
    if (!list.length) return;
    this.bulk = `Fetching ${list.length} ${list.length === 1 ? "repository" : "repositories"}…`;
    const failed: string[] = [];
    let noRemote = 0;
    let done = 0;
    const queue = [...list];
    const worker = async () => {
      for (let p = queue.shift(); p; p = queue.shift()) {
        this.working = { ...this.working, [p]: "Fetching" };
        try {
          await api.fetch(p, null, true);
        } catch (e) {
          // No remotes is not a failure worth reporting in bulk.
          if (String(e).includes("no remotes")) noRemote++;
          else failed.push(`${this.name(p)}: ${String(e).replace(/^AUTH_REQUIRED:/, "sign-in needed — ")}`);
        } finally {
          const w = { ...this.working };
          delete w[p];
          this.working = w;
          done++;
          this.bulk = `Fetching… ${done} of ${list.length}`;
          this.refreshSummaries([p]);
        }
      }
    };
    await Promise.all(Array.from({ length: Math.min(4, list.length) }, worker));
    this.bulk = null;
    const fetched = list.length - failed.length - noRemote;
    const plural = (n: number) => `${n} ${n === 1 ? "repository" : "repositories"}`;
    const skipped = noRemote ? ` ${plural(noRemote)} without a remote ${noRemote === 1 ? "was" : "were"} skipped.` : "";
    if (failed.length) {
      toasts.push("error", `Fetched ${fetched} of ${list.length - noRemote}.${skipped} These failed:\n${failed.slice(0, 6).join("\n")}${failed.length > 6 ? `\n…and ${failed.length - 6} more` : ""}`);
    } else {
      toasts.push("success", `Fetched ${plural(fetched)}.${skipped}`);
    }
  }

  name(path: string) {
    return this.repos.find((r) => r.path === path)?.name ?? path.split("/").pop() ?? path;
  }

  private newGroupId() {
    return `g${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  }

  async newGroup(withPaths: string[] = []) {
    const name = await dialog.prompt("New group", "Name", { confirm: "Create group", placeholder: "Work, Side projects, Clients…", validate: (v) => (v ? null : "Enter a name.") });
    if (!name) return;
    const id = this.newGroupId();
    this.groups.push({ id, name, collapsed: false });
    for (const r of this.repos) if (withPaths.includes(r.path)) r.group = id;
    await this.save();
  }

  async renameGroup(id: string) {
    const g = this.groups.find((x) => x.id === id);
    if (!g) return;
    const name = await dialog.prompt("Rename group", "Name", { value: g.name, confirm: "Rename", validate: (v) => (v ? null : "Enter a name.") });
    if (!name) return;
    g.name = name;
    await this.save();
  }

  async deleteGroup(id: string) {
    const g = this.groups.find((x) => x.id === id);
    if (!g) return;
    const n = this.repos.filter((r) => r.group === id).length;
    if (n && !(await dialog.confirm(`Delete “${g.name}”?`, `Its ${n} ${n === 1 ? "repository moves" : "repositories move"} to Ungrouped. Nothing is deleted from disk.`, "Delete group"))) return;
    for (const r of this.repos) if (r.group === id) r.group = null;
    this.groups = this.groups.filter((x) => x.id !== id);
    await this.save();
  }

  async toggleGroup(id: string) {
    const g = this.groups.find((x) => x.id === id);
    if (!g) return;
    g.collapsed = !g.collapsed;
    await this.save();
  }

  async moveGroup(id: string, beforeId: string | null) {
    const g = this.groups.find((x) => x.id === id);
    if (!g || id === beforeId) return;
    const rest = this.groups.filter((x) => x.id !== id);
    const at = beforeId ? rest.findIndex((x) => x.id === beforeId) : rest.length;
    rest.splice(at < 0 ? rest.length : at, 0, g);
    this.groups = rest;
    await this.save();
  }

  async moveTo(paths: string[], group: string | null) {
    let moved = 0;
    for (const r of this.repos) {
      if (paths.includes(r.path) && r.group !== group) {
        r.group = group;
        moved++;
      }
    }
    if (moved) await this.save();
  }

  async rename(path: string) {
    const r = this.repos.find((x) => x.path === path);
    if (!r) return;
    const name = await dialog.prompt("Rename in list", "Display name", {
      value: r.name, confirm: "Rename", body: "Only changes how it's shown in Arbor. The folder isn't renamed.",
      validate: (v) => (v ? null : "Enter a name."),
    });
    if (!name) return;
    r.name = name;
    await this.save();
  }

  async remove(paths: string[]) {
    this.repos = this.repos.filter((r) => !paths.includes(r.path));
    await this.save();
    toasts.push("info", paths.length === 1 ? "Removed from the list. The folder is untouched." : `Removed ${paths.length} repositories from the list. The folders are untouched.`);
  }

  async locate(path: string): Promise<string | null> {
    const dir = await pickFolder({ directory: true, title: "Where is this repository now?" });
    if (typeof dir !== "string") return null;
    try {
      const info = await api.openRepo(dir);
      const old = this.repos.find((r) => r.path === path);
      // openRepo bookmarked the new location; carry over name and group.
      await this.load();
      const fresh = this.repos.find((r) => r.path === info.path);
      if (old && fresh && old.path !== fresh.path) {
        fresh.group = old.group;
        this.repos = this.repos.filter((r) => r.path !== path);
        await this.save();
      }
      await this.refreshSummaries([info.path]);
      return info.path;
    } catch (e) {
      toasts.push("error", String(e));
      return null;
    }
  }
}

export const library = new LibraryState();
