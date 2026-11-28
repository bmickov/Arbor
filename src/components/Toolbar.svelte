<script lang="ts">
  import { keyText, shortPath } from "../lib/platform";
  import { open as pickFolder } from "@tauri-apps/plugin-dialog";
  import { api } from "../lib/api";
  import * as act from "../lib/actions";
  import { library } from "../lib/library.svelte";
  import { app } from "../lib/state.svelte";
  import { menu, type MenuItem } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";

  const st = $derived(app.status);
  // Buttons that can only fail are disabled, with the reason as their tooltip.
  const noRemote = $derived(!!app.refs && app.refs.remotes.length === 0);
  const unborn = $derived(!!st && !st.head);
  const midOperation = $derived(!!st && st.state !== "clean");
  // Right-click on Pull: the upstream either way, or any other remote branch.
  function pullMenu() {
    const up = st?.upstream;
    const other = app.settings.pullRebase ? { rebase: false, label: "with merge" } : { rebase: true, label: "with rebase" };
    return [
      { label: up ? `Pull from ${up}` : "Pull", hint: keyText("Ctrl+Shift+Down"), disabled: !up, action: () => act.pull() },
      { label: up ? `Pull from ${up} ${other.label}` : `Pull ${other.label}`, disabled: !up, action: () => act.pull({ rebase: other.rebase }) },
      { separator: true },
      { label: "Pull from another branch…", hint: keyText("Ctrl+Shift+L"), action: () => (app.pullDialog = { from: "" }) },
    ];
  }
  const nothingToStash = $derived(!!st && !st.staged.length && !st.unstaged.length);
  const NO_REMOTE = "This repository has no remotes yet. Add one with + next to Remotes in the sidebar.";
  const NO_COMMIT = "Make a first commit first.";

  function repoMenu(e: MouseEvent) {
    const recent = [...library.repos]
      .filter((r) => r.path !== app.path && r.opened)
      .sort((a, b) => b.opened - a.opened)
      .slice(0, 8);
    const items: MenuItem[] = [
      { label: "All repositories", hint: "Ctrl+T", action: () => app.switchTo(-1) },
      { label: "Open repository…", action: async () => {
        const d = await pickFolder({ directory: true });
        if (typeof d === "string") app.open(d);
      } },
      ...(recent.length ? [{ separator: true } as MenuItem] : []),
      ...recent.map((r) => ({ label: r.name, hint: shortPath(r.path), action: () => app.open(r.path) })),
      { separator: true },
      { label: "Show in file manager", action: () => api.openExternal(app.path) },
      { label: "Open terminal here", action: () => api.openTerminal(app.path) },
      { label: "Add remote…", action: act.addRemote },
      { label: "Repository settings…", action: () => (app.repoSettings = true) },
      ...act.repoHostItems(),
      { separator: true },
      { label: "Apply patch…", action: act.applyPatch },
      { label: "Save changes as patch…", action: act.saveChangesPatch },
      { separator: true },
      { label: "Close tab", hint: "Ctrl+W", action: () => app.closeTab(app.active) },
    ];
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu.open(r.left, r.bottom + 4, items);
  }

  function branchMenu(e: MouseEvent) {
    const locals = app.refs?.local ?? [];
    // Remote branches no local branch tracks yet: checking one out creates it, tracking the remote one.
    const tracked = new Set(locals.map((b) => b.upstream).filter(Boolean));
    const remote = (app.refs?.remotes ?? []).flatMap((r) => r.branches).filter((b) => !tracked.has(b.fullName));
    const MAX = 12;
    const items: MenuItem[] = [
      { label: "New branch…", hint: keyText("Ctrl+B"), action: () => act.newBranch() },
      { label: "Find a branch…", hint: keyText("Ctrl+P"), action: () => (app.switcher = true) },
      { separator: true },
      ...locals.slice(0, MAX).map((b) => ({ label: b.name, hint: b.isHead ? "current" : undefined, disabled: b.isHead, action: () => act.checkout(b.name) })),
      ...(locals.length > MAX ? [{ label: `${locals.length - MAX} more…`, action: () => (app.switcher = true) }] : []),
      ...(remote.length ? [{ separator: true } as MenuItem] : []),
      ...remote.slice(0, MAX).map((b) => ({ label: b.fullName, hint: "remote", action: () => act.checkoutRemote(b.fullName) })),
      ...(remote.length > MAX ? [{ label: `${remote.length - MAX} more remote branches…`, action: () => (app.switcher = true) }] : []),
    ];
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu.open(r.left, r.bottom + 4, items, "Switch branch");
  }

  // "Fetched 3 min ago" for the Fetch button's tooltip; ticks every half minute.
  let now = $state(Date.now());
  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 30000);
    return () => clearInterval(t);
  });
  const fetchedAgo = $derived.by(() => {
    const t = app.lastFetched[app.path];
    if (!t) return "";
    const m = Math.round((now - t) / 60000);
    return m < 1 ? " · fetched just now" : ` · fetched ${m} min ago`;
  });

  const undoTitle = $derived(app.undoTop ? keyText(`Undo “${app.undoTop.label}” (Ctrl+Z) · right-click for history`) : "Nothing to undo");
</script>

<header class="bar">
  <div class="left">
    <button class="repo" onclick={repoMenu} title={app.path}>
      <span class="repo-name">{app.repo?.name}</span>
      <Icon name="down" size={12} />
    </button>
    <span class="slash">/</span>
    <button class="branch" onclick={branchMenu} title="Switch branch">
      <Icon name="branch" size={14} />
      {#if st?.detached}
        <span class="detached">detached at {st.head?.slice(0, 7)}</span>
      {:else}
        <span>{st?.branch ?? "…"}</span>
      {/if}
      <Icon name="down" size={12} />
    </button>
  </div>

  <div class="center">
    <button class="tool" onclick={act.fetchAll} disabled={!!app.busy || noRemote} title={noRemote ? NO_REMOTE : `Fetch from all remotes${fetchedAgo}`}>
      <Icon name="fetch" /><span>Fetch</span>
    </button>
    <button class="tool" onclick={() => act.pull()} disabled={!!app.busy || !st?.branch || noRemote || unborn}
      title={noRemote ? NO_REMOTE : unborn ? NO_COMMIT : !st?.branch ? "Check out a branch to pull into it" : st?.upstream ? `Pull from ${st.upstream} (right-click for more)` : "Choose a remote branch to pull from"}
      oncontextmenu={(e) => menu.fromEvent(e, pullMenu())}>
      <Icon name="pull" /><span>Pull</span>
      {#if st?.behind}<b class="count">{st.behind}</b>{/if}
    </button>
    <button class="tool" onclick={() => act.push()} disabled={!!app.busy || !st?.branch || noRemote || unborn} title={noRemote ? NO_REMOTE : unborn ? NO_COMMIT : st?.upstream ? `Push to ${st.upstream}` : "Push and track a new remote branch"}>
      <Icon name="push" /><span>Push</span>
      {#if st?.ahead}<b class="count">{st.ahead}</b>{/if}
    </button>
    <button class="tool" onclick={act.sync} disabled={!!app.busy || !st?.branch || noRemote || unborn} title={noRemote ? NO_REMOTE : unborn ? NO_COMMIT : keyText(st?.upstream ? `Pull from ${st.upstream}, then push (Ctrl+Shift+Y)` : "Push and track a new remote branch (Ctrl+Shift+Y)")}>
      <Icon name="sync" /><span>Sync</span>
    </button>
    <span class="gap"></span>
    <button class="tool" onclick={() => act.stashChanges()} disabled={!!app.busy || unborn || midOperation || nothingToStash}
      title={unborn ? NO_COMMIT : midOperation ? "Finish or abort the merge, rebase or cherry-pick in progress first." : nothingToStash ? "Nothing to stash: the working copy is clean" : "Stash your changes (right-click to save a snapshot instead)"}
      oncontextmenu={(e) => menu.fromEvent(e, [
        { label: "Stash changes…", hint: keyText("Ctrl+Shift+S"), action: () => act.stashChanges() },
        { label: "Save a snapshot…", hint: "keeps your changes", action: () => act.snapshot() },
      ])}>
      <Icon name="stash" /><span>Stash</span>
    </button>
    <button class="tool" onclick={(e) => act.flowMenu(e)} disabled={!!app.busy} title="Git-flow: start and finish features, releases and hotfixes">
      <Icon name="flow" /><span>Git-flow</span>
    </button>
    <button class="tool" onclick={() => app.undo()} oncontextmenu={(e) => act.undoMenu(e)} data-undo disabled={!!app.busy || !app.undoTop} title={undoTitle}>
      <Icon name="undo" /><span>Undo</span>
    </button>
  </div>

  <div class="right">
    {#if app.busy}
      <span class="busy" role="status"><span class="spinner"></span>{app.busy}{app.progress ? ` · ${app.progress}` : "…"}</span>
    {/if}
    <button class="icon" onclick={() => app.refresh(true)} title={keyText("Refresh (Ctrl+R)")} aria-label="Refresh"><Icon name="refresh" /></button>
    <button class="icon" onclick={() => (app.prefs = "general")} title={keyText("Preferences (Ctrl+,)")} aria-label="Preferences"><Icon name="settings" /></button>
  </div>
</header>

<style>
  .bar {
    display: grid; grid-template-columns: 1fr auto 1fr; align-items: center; gap: 12px;
    height: 48px; padding: 0 10px; background: var(--bg); border-bottom: 1px solid var(--line);
  }
  .left { display: flex; align-items: center; gap: 2px; min-width: 0; }
  .repo, .branch {
    display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 8px;
    border-radius: var(--radius); min-width: 0;
  }
  .repo:hover, .branch:hover { background: var(--surface-3); }
  .repo-name { font-weight: 700; font-size: calc(var(--fs) * 14); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .branch { color: var(--ink-2); font-weight: 600; }
  .branch span { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 260px; }
  .detached { color: var(--warn); }
  .slash { color: var(--ink-3); }

  .center { display: flex; align-items: center; gap: 2px; }
  .gap { width: 14px; }
  .tool {
    position: relative; display: flex; flex-direction: column; align-items: center; gap: 1px;
    min-width: 52px; padding: 4px 8px 3px; border-radius: var(--radius); color: var(--ink-2);
  }
  .tool span { font-size: calc(var(--fs) * 11); font-weight: 600; }
  .tool:hover:not(:disabled) { background: var(--surface-3); color: var(--ink); }
  .count {
    position: absolute; top: 1px; right: 8px; min-width: 16px; height: 16px; padding: 0 4px;
    border-radius: 8px; font-size: calc(var(--fs) * 10); line-height: 16px; background: var(--accent); color: var(--accent-ink);
  }

  .right { display: flex; justify-content: flex-end; align-items: center; gap: 4px; min-width: 0; }
  .icon { display: grid; place-items: center; width: 30px; height: 30px; border-radius: var(--radius); color: var(--ink-2); }
  .icon:hover { background: var(--surface-3); color: var(--ink); }
  .busy {
    display: inline-flex; align-items: center; gap: 8px; color: var(--ink-2); font-size: calc(var(--fs) * 12);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; margin-right: 6px;
  }
  .spinner {
    width: 12px; height: 12px; border-radius: 50%; border: 2px solid var(--line);
    border-top-color: var(--accent); animation: spin 0.8s linear infinite; flex: none;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
