<script lang="ts">
  import { api } from "./lib/api";
  import { app } from "./lib/state.svelte";
  import { dialog, menu } from "./lib/ui.svelte";
  import Dialog from "./components/Dialog.svelte";
  import DragGhost from "./components/DragGhost.svelte";
  import FileHistory from "./components/FileHistory.svelte";
  import History from "./components/History.svelte";
  import Menu from "./components/Menu.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import StashView from "./components/StashView.svelte";
  import Toasts from "./components/Toasts.svelte";
  import Toolbar from "./components/Toolbar.svelte";
  import Welcome from "./components/Welcome.svelte";
  import WorkingCopy from "./components/WorkingCopy.svelte";

  let sideW = $state(240);

  // Open the folder given on the command line, or else the last repository.
  $effect(() => {
    api.startupPath().catch(() => null).then((p) => {
      let last: string | null = null;
      try {
        last = localStorage.getItem("arbor.last");
      } catch {}
      const target = p ?? last;
      if (target && !app.repo) app.open(target);
    });
  });
  $effect(() => {
    try {
      if (app.repo) localStorage.setItem("arbor.last", app.repo.path);
    } catch {}
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

  function key(e: KeyboardEvent) {
    if (!app.repo || dialog.spec) return;
    const t = e.target as HTMLElement;
    const typing = t.closest("input, textarea, select, [contenteditable]");
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === "z" && !e.shiftKey && !typing) {
      e.preventDefault();
      app.undo();
    } else if (mod && e.key.toLowerCase() === "r") {
      e.preventDefault();
      app.refresh(true);
    } else if (mod && ["1", "2", "3"].includes(e.key)) {
      e.preventDefault();
      app.view = (["changes", "history", "stashes"] as const)[Number(e.key) - 1];
      if (app.view === "history" && !app.log.length) app.loadLog();
    }
  }

  function resizeSide(e: PointerEvent) {
    const move = (ev: PointerEvent) => (sideW = Math.min(420, Math.max(180, ev.clientX)));
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", () => window.removeEventListener("pointermove", move), { once: true });
  }
</script>

<svelte:window onkeydown={key} />
<!-- The UI has its own menus; suppress the browser's. -->
<svelte:document oncontextmenu={(e) => { if (!(e.target as HTMLElement).closest("input, textarea")) e.preventDefault(); }} />

{#if app.repo}
  <div class="shell" style:--side-w="{sideW}px">
    <div class="top"><Toolbar /></div>
    <aside><Sidebar /></aside>
    <div class="resize" role="separator" aria-orientation="vertical" onpointerdown={resizeSide}></div>
    <main>
      {#if app.view === "changes"}
        <WorkingCopy />
      {:else if app.view === "history"}
        <History />
      {:else}
        <StashView />
      {/if}
    </main>
  </div>
  {#if app.fileHistoryPath}
    <FileHistory path={app.fileHistoryPath} onclose={() => (app.fileHistoryPath = null)} />
  {/if}
{:else}
  <Welcome />
{/if}

<Menu />
<Dialog />
<Toasts />
<DragGhost />

<style>
  .shell {
    display: grid; height: 100%;
    grid-template-columns: var(--side-w) 1px 1fr;
    grid-template-rows: auto 1fr;
    grid-template-areas: "top top top" "side resize main";
  }
  .top { grid-area: top; }
  aside { grid-area: side; min-height: 0; overflow: hidden; }
  .resize { grid-area: resize; background: var(--line); cursor: col-resize; position: relative; }
  .resize::after { content: ""; position: absolute; inset: 0 -3px; }
  main { grid-area: main; min-width: 0; min-height: 0; overflow: hidden; }
</style>
