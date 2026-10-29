<script lang="ts">
  import { open as pickFolder } from "@tauri-apps/plugin-dialog";
  import { api, type RecentRepo } from "../lib/api";
  import { relTime } from "../lib/format";
  import { app } from "../lib/state.svelte";
  import { dialog, toasts } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";

  let recent = $state<RecentRepo[]>([]);
  let cloning = $state(false);

  $effect(() => {
    api.recentRepos().then((r) => (recent = r)).catch(() => {});
  });

  async function openFolder() {
    const dir = await pickFolder({ directory: true, title: "Open a repository" });
    if (typeof dir === "string") await app.open(dir);
  }

  async function create() {
    const dir = await pickFolder({ directory: true, title: "Choose a folder for the new repository" });
    if (typeof dir !== "string") return;
    try {
      const info = await api.initRepo(dir);
      await app.open(info.path);
    } catch (e) {
      toasts.push("error", String(e));
    }
  }

  async function clone() {
    const r = await dialog.show({
      title: "Clone a repository",
      fields: [
        { key: "url", label: "Repository URL", placeholder: "https://github.com/owner/project.git" },
        { key: "dest", label: "Save to folder", placeholder: "/home/you/code/project", hint: "Leave empty to choose a parent folder next." },
      ],
      confirm: "Clone",
      validate: (v) => (String(v.url ?? "").trim() ? null : "Enter the repository URL."),
    });
    if (r.button !== "confirm") return;
    const url = String(r.values.url).trim();
    let dest = String(r.values.dest ?? "").trim();
    if (!dest) {
      const parent = await pickFolder({ directory: true, title: "Choose where to clone into" });
      if (typeof parent !== "string") return;
      const name = url.replace(/\/+$/, "").split(/[/:]/).pop()!.replace(/\.git$/, "");
      dest = `${parent.replace(/\/+$/, "")}/${name}`;
    }
    cloning = true;
    let creds = undefined;
    try {
      for (;;) {
        try {
          const info = await api.cloneRepo(url, dest, creds);
          await app.open(info.path);
          return;
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
          if (c.button !== "confirm") return;
          creds = { username: String(c.values.username), password: String(c.values.password) };
        }
      }
    } catch (e) {
      toasts.push("error", String(e));
    } finally {
      cloning = false;
    }
  }

  async function forget(e: MouseEvent, path: string) {
    e.stopPropagation();
    await api.forgetRepo(path);
    recent = recent.filter((r) => r.path !== path);
  }
</script>

<div class="welcome">
  <div class="col">
    <svg class="mark" viewBox="0 0 64 64" aria-hidden="true">
      <g fill="none" stroke-linecap="round" stroke-width="5">
        <path d="M24 54V14" stroke="var(--lane-0)" />
        <path d="M24 42c0-8 16-6 16-16v-6" stroke="var(--lane-1)" />
        <path d="M24 32c0-5-8-5-8-10" stroke="var(--lane-2)" />
      </g>
      <circle cx="24" cy="54" r="5" fill="var(--lane-0)" />
      <circle cx="24" cy="14" r="5" fill="var(--lane-0)" />
      <circle cx="40" cy="20" r="5" fill="var(--lane-1)" />
      <circle cx="16" cy="22" r="4.5" fill="var(--lane-2)" />
    </svg>
    <h1>Arbor</h1>
    <p class="muted">Open a repository to get started.</p>

    <div class="actions">
      <button class="btn primary" onclick={openFolder}><Icon name="folder" /> Open repository…</button>
      <button class="btn" onclick={clone} disabled={cloning}><Icon name="fetch" /> {cloning ? `Cloning… ${app.progress}` : "Clone…"}</button>
      <button class="btn" onclick={create}><Icon name="plus" /> New repository…</button>
    </div>

    {#if recent.length}
      <h2>Recent</h2>
      <ul class="recent">
        {#each recent as r (r.path)}
          <li>
            <button class="row" onclick={() => app.open(r.path)}>
              <span class="name">{r.name}</span>
              <span class="path faint">{r.path}</span>
              <span class="when faint">{relTime(r.opened)}</span>
            </button>
            <button class="forget" title="Remove from list" aria-label="Remove {r.name} from recent" onclick={(e) => forget(e, r.path)}><Icon name="x" size={13} /></button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .welcome { height: 100%; display: grid; place-items: start center; overflow: auto; padding: 12vh 16px 40px; }
  .col { width: min(560px, 100%); }
  .mark { width: 56px; height: 56px; }
  h1 { font-size: 26px; margin: 6px 0 2px; letter-spacing: -0.01em; font-weight: 700; }
  p { margin: 0 0 22px; }
  .actions { display: flex; gap: 8px; flex-wrap: wrap; }
  h2 { font-size: 13px; color: var(--ink-2); font-weight: 600; margin: 34px 0 6px; }
  .recent { list-style: none; margin: 0; padding: 0; border-top: 1px solid var(--line); }
  li { position: relative; border-bottom: 1px solid var(--line-soft); }
  .row {
    display: grid; grid-template-columns: auto 1fr auto; gap: 12px; align-items: baseline;
    width: 100%; text-align: left; padding: 9px 36px 9px 6px; border-radius: 4px;
  }
  .row:hover { background: var(--surface); }
  .name { font-weight: 600; }
  .path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; direction: rtl; text-align: left; }
  .when { font-size: 12px; }
  .forget { position: absolute; right: 6px; top: 50%; translate: 0 -50%; padding: 4px; border-radius: 4px; color: var(--ink-3); opacity: 0; }
  li:hover .forget { opacity: 1; }
  .forget:hover { background: var(--surface-3); color: var(--ink); }
</style>
