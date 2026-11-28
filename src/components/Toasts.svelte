<script lang="ts">
  import { toasts } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";
</script>

<div class="toasts" aria-live="polite">
  {#each toasts.list as t (t.id)}
    <div class="toast {t.kind}" role="status" onpointerenter={() => toasts.hold(t.id)} onpointerleave={() => toasts.release(t.id)}>
      <span class="text">{t.text}</span>
      {#if t.action}
        <button class="btn small" onclick={() => { t.action!.run(); toasts.dismiss(t.id); }}>{t.action.label}</button>
      {/if}
      <button class="close" aria-label="Dismiss" onclick={() => toasts.dismiss(t.id)}><Icon name="x" size={14} /></button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed; right: 16px; bottom: 16px; z-index: 120;
    display: flex; flex-direction: column; gap: 8px; align-items: flex-end; pointer-events: none;
  }
  .toast {
    pointer-events: auto; display: flex; align-items: flex-start; gap: 10px;
    max-width: 460px; padding: 10px 10px 10px 14px; border-radius: 8px;
    background: var(--ink); color: var(--surface); box-shadow: var(--shadow-pop);
    animation: slide 0.16s ease-out;
  }
  @keyframes slide { from { transform: translateY(8px); opacity: 0; } }
  .toast.error { background: var(--danger); color: #fff; }
  .text { white-space: pre-wrap; user-select: text; padding-top: 1px; line-height: 1.45; }
  .close { opacity: 0.7; display: grid; place-items: center; padding: 3px; border-radius: 4px; }
  .close:hover { opacity: 1; background: rgba(255, 255, 255, 0.15); }
  .btn.small { background: transparent; color: inherit; border-color: currentColor; }
</style>
