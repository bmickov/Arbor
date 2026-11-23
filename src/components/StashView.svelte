<script lang="ts">
  import { keyText } from "../lib/platform";
  import * as act from "../lib/actions";
  import { draggable } from "../lib/drag.svelte";
  import { relTime, fullDate } from "../lib/format";
  import { app } from "../lib/state.svelte";
  import { menu } from "../lib/ui.svelte";
  import CommitDetail from "./CommitDetail.svelte";
  import Icon from "./Icon.svelte";
  import Splitter from "./Splitter.svelte";
  import { SvelteSet } from "svelte/reactivity";

  const sel = $derived(app.stashes.find((s) => s.index === app.selectedStash) ?? null);

  // Ctrl+click / Shift+click picks several stashes (by commit id, since
  // indexes shift when stashes are deleted). With fewer than two picked, the
  // list behaves as a plain single selection.
  const picked = new SvelteSet<string>();
  let anchor: number | null = null;
  const pickedList = $derived(app.stashes.filter((s) => picked.has(s.oid)));
  const multi = $derived(pickedList.length > 1);

  function click(e: MouseEvent, index: number, oid: string) {
    if (e.ctrlKey || e.metaKey) {
      if (!picked.size && sel) picked.add(sel.oid);
      picked.has(oid) ? picked.delete(oid) : picked.add(oid);
      anchor = index;
    } else if (e.shiftKey && anchor !== null) {
      picked.clear();
      for (const s of app.stashes) if (s.index >= Math.min(anchor, index) && s.index <= Math.max(anchor, index)) picked.add(s.oid);
    } else {
      picked.clear();
      anchor = index;
    }
    app.selectedStash = index;
  }

  async function deleteSelected() {
    const indexes = multi ? pickedList.map((s) => s.index) : sel ? [sel.index] : [];
    if (await act.dropStashes(indexes)) picked.clear();
  }

  function listKey(e: KeyboardEvent) {
    if (e.key === "Delete" && (multi || sel)) {
      e.preventDefault();
      deleteSelected();
    } else if (e.key === "Escape" && picked.size) picked.clear();
    else if (e.key === "a" && (e.ctrlKey || e.metaKey) && app.stashes.length) {
      e.preventDefault();
      for (const s of app.stashes) picked.add(s.oid);
    }
  }

  // Forget picks for stashes that are gone.
  $effect(() => {
    const all = new Set(app.stashes.map((s) => s.oid));
    for (const o of [...picked]) if (!all.has(o)) picked.delete(o);
  });

  $effect(() => {
    if (app.selectedStash === null && app.stashes.length) app.selectedStash = app.stashes[0].index;
  });

  function stashMenu(e: MouseEvent, index: number, oid: string) {
    if (multi && picked.has(oid)) {
      menu.fromEvent(e, [
        { label: `Delete ${pickedList.length} stashes…`, danger: true, action: deleteSelected },
        { label: "Clear selection", action: () => picked.clear() },
      ], `${pickedList.length} stashes`);
      return;
    }
    picked.clear();
    app.selectedStash = index;
    menu.fromEvent(e, [
      { label: "Apply and remove", action: () => act.applyStash(index, true) },
      { label: "Apply, keep in list", action: () => act.applyStash(index, false) },
      { label: "Go back to this point…", action: () => act.goBackToStash(oid, app.stashes.find((s) => s.oid === oid)?.message ?? "this stash") },
      { label: "New branch from stash…", action: () => act.stashToBranch(index) },
      { separator: true },
      { label: "Delete…", danger: true, action: () => act.dropStash(index) },
    ]);
  }
</script>

<div class="stashes" style:--list-w="{app.size('stash.list', 300)}px">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="list" onkeydown={listKey}>
    {#each app.stashes as s (s.oid)}
      <button class="item" class:sel={multi ? picked.has(s.oid) : s.index === app.selectedStash}
        onclick={(e) => click(e, s.index, s.oid)}
        oncontextmenu={(e) => stashMenu(e, s.index, s.oid)}
        use:draggable={() => ({ payload: { kind: "stash", index: s.index, message: s.message }, label: `Stash: ${s.message}` })}
        title={keyText("Drag onto Changes to apply · Ctrl+click or Shift+click to select several")}>
        <span class="msg">{s.message}</span>
        <span class="meta faint">{s.branch ? `on ${s.branch} · ` : ""}<span title={fullDate(s.time)}>{relTime(s.time)}</span>{s.hasUntracked ? " · new files" : ""}</span>
      </button>
    {:else}
      <div class="empty"><div><strong>No stashes</strong>Stash puts your changes aside so you can switch tasks with a clean working copy.</div></div>
    {/each}
  </div>
  <Splitter id="stash.list" min={180} keep={320} />
  <div class="detail">
    {#if multi}
      <div class="empty">
        <div class="multi">
          <span class="micon"><Icon name="stash" size={22} /></span>
          <strong>{pickedList.length} stashes selected</strong>
          <ul>
            {#each pickedList.slice(0, 6) as s (s.oid)}<li>{s.message}</li>{/each}
            {#if pickedList.length > 6}<li class="faint">and {pickedList.length - 6} more</li>{/if}
          </ul>
          <div class="mbtns">
            <button class="btn danger" onclick={deleteSelected}><Icon name="discard" size={13} /> Delete {pickedList.length} stashes…</button>
            <button class="btn" onclick={() => picked.clear()}>Clear selection</button>
          </div>
          <small>{keyText("Ctrl+click to add or remove one, Shift+click for a range, Ctrl+A for all, Delete to delete.")}</small>
        </div>
      </div>
    {:else if sel}
      <CommitDetail oid={sel.oid} stash>
        {#snippet header()}
          <div class="actions">
            <button class="btn primary small" onclick={() => act.applyStash(sel.index, true)}><Icon name="pull" size={13} /> Apply and remove</button>
            <button class="btn small" onclick={() => act.applyStash(sel.index, false)}>Apply, keep</button>
            <button class="btn small" onclick={() => act.stashToBranch(sel.index)}><Icon name="branch" size={13} /> New branch</button>
            <span class="sp"></span>
            <button class="btn small danger" onclick={() => act.dropStash(sel.index)}><Icon name="discard" size={13} /> Delete…</button>
          </div>
        {/snippet}
      </CommitDetail>
    {:else if app.stashes.length}
      <div class="empty"><div><strong>Select a stash</strong></div></div>
    {/if}
  </div>
</div>

<style>
  .stashes { display: grid; grid-template-columns: var(--list-w) auto minmax(0, 1fr); height: 100%; min-height: 0; background: var(--surface); }
  .list { overflow-y: auto; padding: 6px; min-width: 0; }
  .item { display: flex; flex-direction: column; gap: 2px; width: 100%; padding: 8px 10px; border-radius: 6px; text-align: left; }
  .item:hover { background: var(--surface-2); }
  .item.sel { background: var(--sel); }
  .msg { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { font-size: calc(var(--fs) * 12); }
  .detail { min-width: 0; min-height: 0; }
  .actions { display: flex; gap: 6px; flex-wrap: wrap; margin-bottom: 4px; }
  .sp { flex: 1; }
  .multi { display: flex; flex-direction: column; align-items: center; gap: 8px; max-width: 420px; }
  .multi ul { list-style: none; margin: 0; padding: 0; color: var(--ink-2); }
  .multi li { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 380px; }
  .multi small { color: var(--ink-3); font-size: calc(var(--fs) * 11.5); }
  .mbtns { display: flex; gap: 8px; margin-top: 4px; }
  .micon { display: grid; place-items: center; width: 44px; height: 44px; border-radius: 12px; background: var(--surface-2); border: 1px solid var(--line-soft); color: var(--ink-2); }
</style>
