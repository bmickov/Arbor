<script lang="ts">
  import { api } from "../lib/api";
  import { draggable, dropTarget } from "../lib/drag.svelte";
  import { library } from "../lib/library.svelte";
  import { app } from "../lib/state.svelte";
  import { menu } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";

  let insertAt = $state<number | null>(null);

  // Status for the dot on each tab: live for the active tab, from the library otherwise.
  function info(path: string, active: boolean) {
    if (active && app.status) {
      const s = app.status;
      return { changes: s.staged.length + s.unstaged.length + s.conflicted.length, conflicts: s.conflicted.length, behind: s.behind };
    }
    const s = library.summaries[path];
    return s ? { changes: s.changes, conflicts: s.conflicts, behind: s.behind } : null;
  }

  function tabMenu(e: MouseEvent, i: number) {
    const t = app.tabs[i];
    menu.fromEvent(e, [
      { label: "Close tab", action: () => app.closeTab(i) },
      { label: "Close other tabs", disabled: app.tabs.length < 2, action: () => {
        const keep = app.tabs[i].path;
        for (let k = app.tabs.length - 1; k >= 0; k--) if (app.tabs[k].path !== keep) app.closeTab(k);
      } },
      { separator: true },
      { label: "Show in file manager", action: () => api.openExternal(t.path) },
      { label: "Open terminal here", action: () => api.openTerminal(t.path) },
      { label: "Copy path", action: () => navigator.clipboard.writeText(t.path) },
    ], t.path.replace(/^\/home\/[^/]+/, "~"));
  }
</script>

<div class="tabs" role="tablist" aria-label="Open repositories">
  <button class="home" class:on={app.active === -1} role="tab" aria-selected={app.active === -1}
    title="All repositories (Ctrl+T)" onclick={() => app.switchTo(-1)}>
    <Icon name="grid" size={14} /><span>Repositories</span>
  </button>

  <div class="strip">
    {#each app.tabs as t, i (t.path)}
      {@const st = info(t.path, i === app.active)}
      <div class="tab" class:on={i === app.active} role="tab" aria-selected={i === app.active} tabindex="0"
        class:ins-before={insertAt === i} class:ins-after={insertAt === i + 1 && i === app.tabs.length - 1}
        title={t.path}
        onclick={() => app.switchTo(i)}
        onkeydown={(e) => e.key === "Enter" && app.switchTo(i)}
        onauxclick={(e) => { if (e.button === 1) { e.preventDefault(); app.closeTab(i); } }}
        oncontextmenu={(e) => tabMenu(e, i)}
        use:draggable={() => ({ payload: { kind: "tab", index: i }, label: t.name })}
        use:dropTarget={{
          accepts: (p) => p.kind === "tab" && p.index !== i,
          hover: (_p, _f) => {},
          leave: () => (insertAt = null),
          drop: (p) => { if (p.kind === "tab") app.moveTab(p.index, insertAt ?? i); insertAt = null; },
        }}
        onpointermove={(e) => {
          if (!document.body.classList.contains("dragging")) return;
          const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
          insertAt = e.clientX < r.left + r.width / 2 ? i : i + 1;
        }}>
        {#if st?.conflicts}
          <span class="dot conflict" title="Conflicts"></span>
        {:else if st?.changes}
          <span class="dot dirty" title="{st.changes} uncommitted {st.changes === 1 ? 'change' : 'changes'}"></span>
        {/if}
        <span class="name">{t.name}</span>
        {#if st?.behind}<span class="behind" title="{st.behind} to pull">↓{st.behind}</span>{/if}
        <button class="x" aria-label="Close {t.name}" title="Close (Ctrl+W)" onclick={(e) => { e.stopPropagation(); app.closeTab(i); }}><Icon name="x" size={12} /></button>
      </div>
    {/each}
  </div>
  <button class="plus" title="Open another repository" aria-label="Open another repository" onclick={() => app.switchTo(-1)}><Icon name="plus" size={14} /></button>
</div>

<style>
  .tabs {
    display: flex; align-items: flex-end; gap: 2px; height: 38px; padding: 0 8px 0 6px;
    background: var(--surface-3); border-bottom: 1px solid var(--line);
  }
  .home, .tab {
    height: 31px; display: flex; align-items: center; gap: 7px; padding: 0 10px;
    border-radius: 8px 8px 0 0; color: var(--ink-2); font-weight: 600; flex: none;
    border: 1px solid transparent; border-bottom: 0; margin-bottom: -1px;
  }
  .home:hover, .tab:hover { background: color-mix(in srgb, var(--bg) 55%, transparent); color: var(--ink); }
  .home.on, .tab.on { background: var(--bg); color: var(--ink); border-color: var(--line); }
  .strip { display: flex; gap: 2px; min-width: 0; overflow-x: auto; scrollbar-width: none; align-items: flex-end; height: 100%; }
  .strip::-webkit-scrollbar { display: none; }
  .tab { position: relative; max-width: 220px; min-width: 90px; padding-right: 4px; cursor: default; }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dot { width: 7px; height: 7px; border-radius: 50%; flex: none; }
  .dot.dirty { background: var(--lane-1); }
  .dot.conflict { background: var(--danger); }
  .behind { font-size: 11px; color: var(--ink-3); font-variant-numeric: tabular-nums; }
  .x { display: grid; place-items: center; width: 20px; height: 20px; border-radius: 5px; color: var(--ink-3); opacity: 0; flex: none; }
  .tab:hover .x, .tab.on .x { opacity: 1; }
  .x:hover { background: var(--surface-3); color: var(--ink); }
  .plus { display: grid; place-items: center; width: 28px; height: 28px; margin: 0 0 4px 2px; border-radius: 6px; color: var(--ink-2); flex: none; }
  .plus:hover { background: var(--bg); color: var(--ink); }
  .tab.ins-before::before, .tab.ins-after::after {
    content: ""; position: absolute; top: 5px; bottom: 5px; width: 3px; border-radius: 2px; background: var(--accent);
  }
  .tab.ins-before::before { left: -3px; }
  .tab.ins-after::after { right: -3px; }
  .tab:global(.drop-over) { box-shadow: none !important; }
</style>
