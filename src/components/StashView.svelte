<script lang="ts">
  import * as act from "../lib/actions";
  import { draggable } from "../lib/drag.svelte";
  import { relTime, fullDate } from "../lib/format";
  import { app } from "../lib/state.svelte";
  import { menu } from "../lib/ui.svelte";
  import CommitDetail from "./CommitDetail.svelte";
  import Icon from "./Icon.svelte";

  const sel = $derived(app.stashes.find((s) => s.index === app.selectedStash) ?? null);

  $effect(() => {
    if (app.selectedStash === null && app.stashes.length) app.selectedStash = app.stashes[0].index;
  });

  function stashMenu(e: MouseEvent, index: number) {
    app.selectedStash = index;
    menu.fromEvent(e, [
      { label: "Apply and remove", action: () => act.applyStash(index, true) },
      { label: "Apply, keep in list", action: () => act.applyStash(index, false) },
      { label: "New branch from stash…", action: () => act.stashToBranch(index) },
      { separator: true },
      { label: "Delete", danger: true, action: () => act.dropStash(index) },
    ]);
  }
</script>

<div class="stashes">
  <div class="list">
    {#each app.stashes as s (s.oid)}
      <button class="item" class:sel={s.index === app.selectedStash}
        onclick={() => (app.selectedStash = s.index)}
        oncontextmenu={(e) => stashMenu(e, s.index)}
        use:draggable={() => ({ payload: { kind: "stash", index: s.index, message: s.message }, label: `Stash: ${s.message}` })}
        title="Drag onto Changes to apply">
        <span class="msg">{s.message}</span>
        <span class="meta faint">{s.branch ? `on ${s.branch} · ` : ""}<span title={fullDate(s.time)}>{relTime(s.time)}</span>{s.hasUntracked ? " · new files" : ""}</span>
      </button>
    {:else}
      <div class="empty"><div><strong>No stashes</strong>Stash puts your changes aside so you can switch tasks with a clean working copy.</div></div>
    {/each}
  </div>
  <div class="detail">
    {#if sel}
      <CommitDetail oid={sel.oid}>
        {#snippet header()}
          <div class="actions">
            <button class="btn primary small" onclick={() => act.applyStash(sel.index, true)}><Icon name="pull" size={13} /> Apply and remove</button>
            <button class="btn small" onclick={() => act.applyStash(sel.index, false)}>Apply, keep</button>
            <button class="btn small" onclick={() => act.stashToBranch(sel.index)}><Icon name="branch" size={13} /> New branch</button>
            <span class="sp"></span>
            <button class="btn small danger" onclick={() => act.dropStash(sel.index)}><Icon name="discard" size={13} /> Delete</button>
          </div>
        {/snippet}
      </CommitDetail>
    {:else if app.stashes.length}
      <div class="empty"><div><strong>Select a stash</strong></div></div>
    {/if}
  </div>
</div>

<style>
  .stashes { display: grid; grid-template-columns: minmax(260px, 340px) 1fr; height: 100%; min-height: 0; background: var(--surface); }
  .list { overflow-y: auto; border-right: 1px solid var(--line); padding: 6px; }
  .item { display: flex; flex-direction: column; gap: 2px; width: 100%; padding: 8px 10px; border-radius: 6px; text-align: left; }
  .item:hover { background: var(--surface-2); }
  .item.sel { background: var(--sel); }
  .msg { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { font-size: 12px; }
  .detail { min-width: 0; min-height: 0; }
  .actions { display: flex; gap: 6px; flex-wrap: wrap; margin-bottom: 4px; }
  .sp { flex: 1; }
</style>
