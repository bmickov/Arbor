<script lang="ts">
  import { api } from "../lib/api";
  import * as act from "../lib/actions";
  import { relTime } from "../lib/format";
  import { app } from "../lib/state.svelte";
  import Icon from "./Icon.svelte";

  interface Item {
    kind: "local" | "remote" | "create";
    name: string;
    hint: string;
    current?: boolean;
    score: number;
  }

  let query = $state("");
  let active = $state(0);
  let input = $state<HTMLInputElement>();
  let list = $state<HTMLDivElement>();

  $effect(() => {
    input?.focus();
  });

  /** Higher is better; -1 means no match. Substrings beat scattered letters. */
  function score(name: string, q: string): number {
    if (!q) return 0;
    const n = name.toLowerCase();
    const i = n.indexOf(q);
    if (i >= 0) return 1000 - i - (n.length - q.length) * 0.1 + (i === 0 || "/-_".includes(n[i - 1]) ? 50 : 0);
    let pos = 0;
    let gaps = 0;
    for (const ch of q) {
      const at = n.indexOf(ch, pos);
      if (at < 0) return -1;
      gaps += at - pos;
      pos = at + 1;
    }
    return 500 - gaps;
  }

  const items = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const locals = app.refs?.local ?? [];
    const localNames = new Set(locals.map((b) => b.name));
    const tracked = new Set(locals.map((b) => b.upstream).filter(Boolean));
    const out: Item[] = [];
    for (const b of locals) {
      const s = score(b.name, q);
      if (s >= 0) out.push({ kind: "local", name: b.name, hint: b.isHead ? "current" : relTime(b.time), current: b.isHead, score: s });
    }
    for (const r of app.refs?.remotes ?? []) {
      for (const b of r.branches) {
        // Remote branches a local branch tracks are covered above.
        if (tracked.has(b.fullName)) continue;
        const s = score(b.fullName, q);
        if (s >= 0) out.push({ kind: "remote", name: b.fullName, hint: "remote", score: s - 1 });
      }
    }
    if (q) out.sort((a, b) => b.score - a.score);
    else out.sort((a, b) => Number(b.current) - Number(a.current) || (a.kind === b.kind ? 0 : a.kind === "local" ? -1 : 1));
    const raw = query.trim();
    if (raw && !localNames.has(raw) && !/\s|\.\.|[~^:?*\[\\]/.test(raw)) {
      out.push({ kind: "create", name: raw, hint: "new branch", score: -2 });
    }
    return out.slice(0, 200);
  });

  $effect(() => {
    void query;
    active = 0;
  });

  // What Enter does to the highlighted row.
  const enterLabel = $derived.by(() => {
    const it = items[active];
    return !it ? "to switch" : it.kind === "create" ? "to create it" : it.kind === "remote" ? "to check it out" : it.current ? "to stay" : "to switch";
  });

  function close() {
    app.switcher = false;
  }

  async function choose(it: Item | undefined) {
    if (!it) return;
    close();
    if (it.kind === "local") {
      if (!it.current) await act.checkout(it.name);
    } else if (it.kind === "remote") {
      await act.checkoutRemote(it.name);
    } else {
      await app.act("Creating branch", () => api.createBranch(app.path, it.name, null, true), { success: `Created and switched to ${it.name}.` });
    }
  }

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = items.length;
      if (!n) return;
      active = (active + (e.key === "ArrowDown" ? 1 : -1) + n) % n;
      list?.querySelectorAll<HTMLElement>(".item")[active]?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter") {
      e.preventDefault();
      choose(items[active]);
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="backdrop" onclick={(e) => e.target === e.currentTarget && close()}>
  <div class="palette" role="dialog" aria-modal="true" aria-label="Switch branch">
    <div class="field">
      <Icon name="branch" size={15} />
      <input bind:this={input} bind:value={query} onkeydown={key} placeholder="Switch to branch, or type a new name" spellcheck="false"
        role="combobox" aria-expanded="true" aria-controls="switcher-list" aria-activedescendant="sw-{active}" />
    </div>
    <div class="list" id="switcher-list" role="listbox" bind:this={list}>
      {#each items as it, i (it.kind + it.name)}
        <button class="item" id="sw-{i}" class:on={i === active} role="option" aria-selected={i === active}
          onclick={() => choose(it)} onpointermove={() => (active = i)}>
          {#if it.kind === "create"}
            <Icon name="plus" size={14} /><span class="name">Create branch <b>{it.name}</b></span>
          {:else}
            <Icon name={it.kind === "remote" ? "remote" : "branch"} size={14} />
            <span class="name" class:cur={it.current}>{it.name}</span>
          {/if}
          <span class="hint">{it.hint}</span>
        </button>
      {:else}
        <div class="none">{query.trim() && /\s/.test(query.trim()) ? "No matching branches. A new branch name can't contain spaces." : "No matching branches."}</div>
      {/each}
    </div>
    <div class="foot"><span><span class="kbd">↑</span> <span class="kbd">↓</span> to choose</span><span><span class="kbd">Enter</span> {enterLabel}</span><span><span class="kbd">Esc</span> to close</span></div>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 95; background: var(--backdrop); display: flex; justify-content: center; align-items: flex-start; padding-top: 12vh; }
  .palette {
    width: min(560px, calc(100vw - 32px)); background: var(--surface); border: 1px solid var(--line);
    border-radius: 12px; box-shadow: var(--shadow-pop); overflow: hidden; display: flex; flex-direction: column;
    animation: drop 0.12s ease-out;
  }
  @keyframes drop { from { translate: 0 -6px; opacity: 0; } }
  .field { display: flex; align-items: center; gap: 10px; padding: 0 14px; border-bottom: 1px solid var(--line); color: var(--ink-3); }
  .field input { border: 0; background: transparent; box-shadow: none; padding: 13px 0; font-size: calc(var(--fs) * 15); }
  .list { max-height: 50vh; overflow-y: auto; padding: 6px; }
  .item { display: flex; align-items: center; gap: 10px; width: 100%; padding: 7px 10px; border-radius: 7px; text-align: left; color: var(--ink-2); }
  .item.on { background: var(--sel); color: var(--ink); }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--ink); }
  .name.cur { font-weight: 700; }
  .hint { font-size: calc(var(--fs) * 12); color: var(--ink-3); flex: none; }
  .none { padding: 14px; color: var(--ink-3); text-align: center; }
  .foot { display: flex; gap: 16px; padding: 7px 14px; border-top: 1px solid var(--line-soft); font-size: calc(var(--fs) * 11); color: var(--ink-3); }
</style>
