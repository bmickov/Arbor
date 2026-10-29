<script lang="ts">
  import { menu } from "../lib/ui.svelte";

  let el = $state<HTMLDivElement>();
  let pos = $state({ x: 0, y: 0 });
  let active = $state(-1);

  $effect(() => {
    if (!menu.items.length || !el) return;
    // Keep the menu on screen.
    const r = el.getBoundingClientRect();
    pos = {
      x: Math.min(menu.x, window.innerWidth - r.width - 6),
      y: menu.y + r.height > window.innerHeight - 6 ? Math.max(6, menu.y - r.height) : menu.y,
    };
    active = -1;
    el.focus();
  });

  function choose(i: number) {
    const it = menu.items[i];
    if (!it || it.disabled || it.separator) return;
    menu.close();
    it.action?.();
  }

  function key(e: KeyboardEvent) {
    const n = menu.items.length;
    if (e.key === "Escape") return menu.close();
    if (e.key === "Enter") return choose(active);
    const dir = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
    if (!dir) return;
    e.preventDefault();
    for (let k = 1; k <= n; k++) {
      const i = (active + dir * k + n * 2) % n;
      if (!menu.items[i].separator && !menu.items[i].disabled) {
        active = i;
        break;
      }
    }
  }
</script>

{#if menu.items.length}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div class="scrim" onpointerdown={() => menu.close()} oncontextmenu={(e) => { e.preventDefault(); menu.close(); }}></div>
  <div class="menu" role="menu" tabindex="-1" bind:this={el} style:left="{pos.x}px" style:top="{pos.y}px" onkeydown={key}>
    {#if menu.title}<div class="title">{menu.title}</div>{/if}
    {#each menu.items as it, i}
      {#if it.separator}
        {#if i > 0 && i < menu.items.length - 1 && !menu.items[i - 1].separator}<div class="sep"></div>{/if}
      {:else}
        <button role="menuitem" class:danger={it.danger} class:active={i === active} disabled={it.disabled} onclick={() => choose(i)} onpointerenter={() => (active = i)}>
          <span>{it.label}</span>
          {#if it.hint}<span class="hint">{it.hint}</span>{/if}
        </button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .scrim { position: fixed; inset: 0; z-index: 90; }
  .menu {
    position: fixed; z-index: 91; min-width: 220px; max-width: 360px;
    padding: 5px; border-radius: 8px; background: var(--surface);
    border: 1px solid var(--line); box-shadow: var(--shadow-pop); outline: none;
  }
  .title {
    padding: 4px 8px 6px; color: var(--ink-3); font-size: calc(var(--fs) * 12);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 340px;
  }
  button {
    display: flex; width: 100%; justify-content: space-between; gap: 16px; align-items: center;
    padding: 5px 8px; border-radius: 5px; text-align: left;
  }
  button.active:not(:disabled) { background: var(--accent); color: var(--accent-ink); }
  button.danger { color: var(--danger); }
  button.danger.active:not(:disabled) { background: var(--danger); color: #fff; }
  .hint { color: var(--ink-3); font-size: calc(var(--fs) * 12); }
  button.active .hint { color: inherit; opacity: 0.8; }
  .sep { height: 1px; background: var(--line-soft); margin: 4px 2px; }
</style>
