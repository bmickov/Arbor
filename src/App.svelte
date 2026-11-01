<script lang="ts">
  import { api } from "./lib/api";
  import { library } from "./lib/library.svelte";
  import { app } from "./lib/state.svelte";
  import { handleKey } from "./lib/shortcuts.svelte";
  import { dialog, menu } from "./lib/ui.svelte";
  import BranchSwitcher from "./components/BranchSwitcher.svelte";
  import QuickActions from "./components/QuickActions.svelte";
  import BranchesReview from "./components/BranchesReview.svelte";
  import BisectBar from "./components/BisectBar.svelte";
  import BrowseView from "./components/BrowseView.svelte";
  import CompareView from "./components/CompareView.svelte";
  import ReflogView from "./components/ReflogView.svelte";
  import Preferences from "./components/Preferences.svelte";
  import Dialog from "./components/Dialog.svelte";
  import DragGhost from "./components/DragGhost.svelte";
  import FileHistory from "./components/FileHistory.svelte";
  import History from "./components/History.svelte";
  import Menu from "./components/Menu.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import StashView from "./components/StashView.svelte";
  import Toasts from "./components/Toasts.svelte";
  import Toolbar from "./components/Toolbar.svelte";
  import RepoManager from "./components/RepoManager.svelte";
  import TabBar from "./components/TabBar.svelte";
  import WorkingCopy from "./components/WorkingCopy.svelte";


  // Load the library, then reopen last session's tabs (plus any folder
  // given on the command line).
  $effect(() => {
    (async () => {
      await library.ensure().catch(() => {});
      const p = await api.startupPath().catch(() => null);
      await app.restoreTabs(p);
    })();
  });

  // Fetch open repositories in the background ("fetch automatically" preference).
  $effect(() => {
    if (!app.settings.autoFetchMinutes) return;
    const first = setTimeout(() => app.autoFetch(), 5000);
    const t = setInterval(() => app.autoFetch(), 30000);
    return () => {
      clearTimeout(first);
      clearInterval(t);
    };
  });

  // Keep the status dots on background tabs reasonably fresh.
  $effect(() => {
    const t = setInterval(() => {
      const bg = app.tabs.filter((_, i) => i !== app.active).map((t) => t.path);
      if (bg.length && document.hasFocus() && app.active >= 0) library.refreshSummaries(bg);
    }, 30000);
    return () => clearInterval(t);
  });

  // Pick up changes made in an editor or terminal.
  $effect(() => {
    const onFocus = () => {
      app.workdirMayHaveChanged = true;
      app.refresh();
    };
    window.addEventListener("focus", onFocus);
    const t = setInterval(() => {
      if (document.hasFocus() && !app.busy && !dialog.spec && !menu.items.length) app.refresh();
    }, 4000);
    return () => {
      window.removeEventListener("focus", onFocus);
      clearInterval(t);
    };
  });

  // Save the session shortly after anything in it changes, and on quit.
  $effect(() => {
    JSON.stringify(app.session()); // read everything in the session so changes re-run this
    void app.settings.restoreSession;
    const t = setTimeout(() => app.saveSession(), 400);
    return () => clearTimeout(t);
  });
  $effect(() => {
    const save = () => app.saveSession();
    window.addEventListener("pagehide", save);
    window.addEventListener("beforeunload", save);
    return () => {
      window.removeEventListener("pagehide", save);
      window.removeEventListener("beforeunload", save);
    };
  });

  function resizeSide(e: PointerEvent) {
    const move = (ev: PointerEvent) => (app.layout.sideW = Math.min(420, Math.max(180, ev.clientX)));
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", () => window.removeEventListener("pointermove", move), { once: true });
  }
</script>

<svelte:window onkeydown={handleKey} />
<!-- The UI has its own menus; suppress the browser's. -->
<svelte:document oncontextmenu={(e) => { if (!(e.target as HTMLElement).closest("input, textarea")) e.preventDefault(); }} />

<div class="window">
  <TabBar />
  {#if app.repo}
    <div class="shell" class:noside={app.layout.sidebarHidden} style:--side-w="{app.layout.sideW}px">
      <div class="top"><Toolbar /></div>
      {#if !app.layout.sidebarHidden}
        <aside><Sidebar /></aside>
        <div class="resize" role="separator" aria-orientation="vertical" title="Drag to resize · double-click to reset" onpointerdown={resizeSide} ondblclick={() => (app.layout.sideW = 240)}></div>
      {/if}
      <main>
        <BisectBar />
        <div class="view">
        <!-- Remount per tab so each view restores that tab's scroll and selection. -->
        {#key app.repo.path}
          {#if app.view === "changes"}
            <WorkingCopy />
          {:else if app.view === "history"}
            <History />
          {:else if app.view === "review"}
            <BranchesReview />
          {:else if app.view === "reflog"}
            <ReflogView />
          {:else if app.view === "compare"}
            <CompareView />
          {:else if app.view === "browse"}
            <BrowseView />
          {:else}
            <StashView />
          {/if}
        {/key}
        </div>
      </main>
    </div>
    {#if app.fileHistoryPath}
      <FileHistory path={app.fileHistoryPath} onclose={() => (app.fileHistoryPath = null)} />
    {/if}
  {:else}
    <div class="home"><RepoManager /></div>
  {/if}
</div>

{#if app.prefs}<Preferences />{/if}
{#if app.switcher && app.repo}<BranchSwitcher />{/if}
{#if app.quick}<QuickActions />{/if}
<Menu />
<Dialog />
<Toasts />
<DragGhost />

<style>
  .window { display: flex; flex-direction: column; height: 100%; }
  .home { flex: 1; min-height: 0; }
  .shell {
    display: grid; flex: 1; min-height: 0;
    grid-template-columns: var(--side-w) 1px 1fr;
    grid-template-rows: auto 1fr;
    grid-template-areas: "top top top" "side resize main";
  }
  .shell.noside { grid-template-columns: 0 0 1fr; }
  .top { grid-area: top; }
  aside { grid-area: side; min-height: 0; overflow: hidden; }
  .resize { grid-area: resize; background: var(--line); cursor: col-resize; position: relative; }
  .resize::after { content: ""; position: absolute; inset: 0 -3px; }
  main { grid-area: main; min-width: 0; min-height: 0; overflow: hidden; display: flex; flex-direction: column; }
  .view { flex: 1; min-height: 0; position: relative; }
</style>
