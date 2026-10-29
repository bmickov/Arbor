<script lang="ts">
  import { drag } from "../lib/drag.svelte";
</script>

{#if drag.payload}
  <div class="ghost" class:ok={drag.overAccepts} style:left="{drag.x + 14}px" style:top="{drag.y + 10}px">
    {drag.label}
  </div>
{/if}

<style>
  .ghost {
    position: fixed; z-index: 200; pointer-events: none; max-width: 320px;
    padding: 4px 10px; border-radius: 12px; font-weight: 600; font-size: calc(var(--fs) * 12);
    background: var(--ink); color: var(--surface); box-shadow: var(--shadow-pop);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; opacity: 0.85;
  }
  .ghost.ok { background: var(--accent); color: var(--accent-ink); opacity: 1; }
</style>
