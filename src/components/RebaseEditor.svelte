<script lang="ts">
  import { api, type CommitRow, type RebaseAction } from "../lib/api";
  import * as act from "../lib/actions";
  import { draggable, dropTarget } from "../lib/drag.svelte";
  import { app } from "../lib/state.svelte";
  import { dialog } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";

  let { from, onclose }: { from: CommitRow; onclose: () => void } = $props();

  interface Item { c: CommitRow; action: RebaseAction; message: string; full: string }

  const all = act.rewritable();
  const start = all.findIndex((c) => c.oid === from.oid);
  // Newest first, like the history list.
  let items = $state<Item[]>(all.slice(Math.max(0, start)).reverse().map((c) => ({ c, action: "pick", message: c.summary, full: "" })));
  let target = $state<{ i: number; below: boolean } | null>(null);
  let busy = $state(false);

  // Load full messages so rewording keeps the description.
  $effect(() => {
    for (const it of items) {
      if (it.full) continue;
      api.commitInfo(app.path, it.c.oid).then((info) => {
        it.full = info.body ? `${info.summary}\n\n${info.body}` : info.summary;
        if (it.action !== "reword") it.message = it.full;
      });
    }
  });

  const ACTIONS: { id: RebaseAction; label: string; hint: string }[] = [
    { id: "pick", label: "Keep", hint: "Keep the commit as it is" },
    { id: "reword", label: "Edit message", hint: "Keep the changes, change the message" },
    { id: "squash", label: "Squash", hint: "Combine with the commit below, keeping both messages" },
    { id: "fixup", label: "Fixup", hint: "Combine with the commit below, dropping this message" },
    { id: "drop", label: "Remove", hint: "Remove the commit and its changes" },
  ];

  function move(fromI: number, toI: number, below: boolean) {
    const list = [...items];
    const [it] = list.splice(fromI, 1);
    let at = toI + (below ? 1 : 0);
    if (fromI < at) at--;
    list.splice(at, 0, it);
    items = list;
  }

  const bottomInvalid = $derived.by(() => {
    const kept = items.filter((x) => x.action !== "drop");
    const last = kept[kept.length - 1];
    return last && (last.action === "squash" || last.action === "fixup");
  });
  const changed = $derived(items.some((x, i) => x.action !== "pick" || x.c.oid !== all.slice(Math.max(0, start)).reverse()[i]?.oid));

  async function apply() {
    if (bottomInvalid || !changed) return;
    const oldest = all[Math.max(0, start)];
    const base = oldest.parents[0] ?? null;
    const steps = [...items].reverse().map((x) => ({
      oid: x.c.oid,
      action: x.action,
      message: x.action === "reword" ? x.message : null,
    }));
    const touched = items.map((x) => x.c.oid);
    const pushed = act.pushedSet();
    if (touched.some((o) => pushed.has(o)) && !(await dialog.confirm("Rewrite pushed commits?", "Some of these commits are already on the remote. Afterwards you'll need to force push.", "Rewrite anyway", true))) return;
    busy = true;
    const r = await app.act("Rewriting history", () => api.interactiveRebase(app.path, base, steps, `Rewrite ${items.length} commits`), {
      success: (r) => (r.result === "unchanged" ? "Nothing changed." : "History rewritten. Undo reverts it."),
    });
    busy = false;
    if (r) onclose();
  }

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onkeydown={key}>
  <div class="panel" role="dialog" aria-modal="true" aria-labelledby="rb-title">
    <header>
      <h2 id="rb-title">Rewrite history</h2>
      <p class="muted">Drag commits to reorder them, and choose what happens to each. Newest is at the top. Nothing changes until you apply, and Undo reverts it.</p>
    </header>
    <ol>
      {#each items as it, i (it.c.oid)}
        <li class:dropped={it.action === "drop"} class:combine={it.action === "squash" || it.action === "fixup"}
          class:line-above={target?.i === i && !target.below} class:line-below={target?.i === i && target.below}
          use:draggable={() => ({ payload: { kind: "commit", oid: it.c.oid, summary: it.c.summary }, label: it.c.summary })}
          use:dropTarget={{
            accepts: (p) => p.kind === "commit" && p.oid !== it.c.oid && items.some((x) => x.c.oid === p.oid),
            hover: (_p, f) => (target = { i, below: f > 0.5 }),
            leave: () => { if (target?.i === i) target = null; },
            drop: (p) => {
              if (p.kind !== "commit") return;
              const fromI = items.findIndex((x) => x.c.oid === p.oid);
              move(fromI, i, target?.below ?? false);
              target = null;
            },
          }}>
          <span class="grip"><Icon name="grip" size={14} /></span>
          <select bind:value={it.action} data-nodrag aria-label="Action for {it.c.short}">
            {#each ACTIONS as a}<option value={a.id} title={a.hint}>{a.label}</option>{/each}
          </select>
          {#if it.action === "reword"}
            <textarea rows="3" bind:value={it.message} data-nodrag aria-label="New message"></textarea>
          {:else}
            <span class="summary">
              {#if it.action === "squash" || it.action === "fixup"}<Icon name="merge" size={13} />{/if}
              {it.c.summary}
            </span>
          {/if}
          <span class="sha mono faint">{it.c.short}</span>
        </li>
      {/each}
    </ol>
    {#if bottomInvalid}<p class="error">The oldest commit can't be squashed — there's nothing below it in this list to combine with.</p>{/if}
    <footer>
      <button class="btn" onclick={onclose}>Cancel</button>
      <button class="btn primary" disabled={busy || bottomInvalid || !changed} onclick={apply}>Apply changes</button>
    </footer>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 80; background: var(--backdrop); display: grid; place-items: center; padding: 24px; }
  .panel {
    width: min(720px, 100%); max-height: 85vh; display: flex; flex-direction: column; background: var(--surface);
    border: 1px solid var(--line); border-radius: 12px; box-shadow: var(--shadow-pop);
  }
  header { padding: 18px 20px 8px; }
  h2 { margin: 0 0 4px; font-size: calc(var(--fs) * 16); }
  header p { margin: 0; font-size: calc(var(--fs) * 12); }
  ol { list-style: none; margin: 0; padding: 8px 12px; overflow-y: auto; flex: 1; }
  li {
    display: flex; align-items: center; gap: 10px; padding: 6px 8px; border-radius: 7px; border: 1px solid var(--line-soft);
    margin-bottom: 4px; background: var(--surface); cursor: grab;
  }
  li.dropped .summary { text-decoration: line-through; color: var(--ink-3); }
  li.combine { margin-left: 22px; border-left: 3px solid var(--lane-3); }
  li.line-above { box-shadow: 0 -2px 0 var(--accent); }
  li.line-below { box-shadow: 0 2px 0 var(--accent); }
  .grip { color: var(--ink-3); display: grid; }
  select { width: 130px; flex: none; padding: 4px 6px; }
  .summary { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; display: flex; gap: 6px; align-items: center; }
  textarea { flex: 1; font-size: calc(var(--fs) * 12); }
  .sha { flex: none; }
  .error { margin: 0 20px; color: var(--danger); font-size: calc(var(--fs) * 12); }
  footer { display: flex; justify-content: flex-end; gap: 8px; padding: 12px 16px; border-top: 1px solid var(--line); }
</style>
