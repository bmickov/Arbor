<script lang="ts">
  import { dialog } from "../lib/ui.svelte";

  let values = $state<Record<string, string | boolean>>({});
  let error = $state<string | null>(null);
  let form = $state<HTMLFormElement>();

  $effect(() => {
    const s = dialog.spec;
    if (!s) return;
    const v: Record<string, string | boolean> = {};
    for (const f of s.fields ?? []) v[f.key] = f.type === "checkbox" ? !!f.checked : (f.value ?? "");
    values = v;
    error = null;
    queueMicrotask(() => {
      const first = form?.querySelector<HTMLElement>("input:not([type=checkbox]), textarea") ?? form?.querySelector<HTMLElement>("button.primary-action");
      first?.focus();
      if (first instanceof HTMLInputElement || first instanceof HTMLTextAreaElement) first.select();
    });
  });

  function submit(e: Event) {
    e.preventDefault();
    const s = dialog.spec!;
    const msg = s.validate?.(values) ?? null;
    if (msg) {
      error = msg;
      return;
    }
    dialog.finish({ button: "confirm", values: { ...values } });
  }

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      dialog.finish({ button: "cancel", values: {} });
    }
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) submit(e);
  }
</script>

{#if dialog.spec}
  {@const s = dialog.spec}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="backdrop" onkeydown={key} role="dialog" aria-modal="true" aria-labelledby="dlg-title" tabindex="-1">
    <form class="dialog" bind:this={form} onsubmit={submit} aria-labelledby="dlg-title">
      <h2 id="dlg-title">{s.title}</h2>
      {#if s.body}<p class="body">{s.body}</p>{/if}
      {#each s.fields ?? [] as f (f.key)}
        {#if f.type === "checkbox"}
          <label class="check"><input type="checkbox" bind:checked={values[f.key] as boolean} /> {f.label}</label>
        {:else}
          <label class="field">
            <span>{f.label}</span>
            {#if f.type === "textarea"}
              <textarea rows="6" bind:value={values[f.key] as string} placeholder={f.placeholder}></textarea>
            {:else}
              <input type={f.type === "password" ? "password" : "text"} bind:value={values[f.key] as string} placeholder={f.placeholder} spellcheck="false" autocomplete="off" />
            {/if}
            {#if f.hint}<small class="faint">{f.hint}</small>{/if}
          </label>
        {/if}
      {/each}
      {#if error}<p class="error">{error}</p>{/if}
      <div class="buttons">
        {#each s.extra ?? [] as x}
          <button type="button" class="btn" class:danger={x.danger} onclick={() => dialog.finish({ button: x.id, values: { ...values } })}>{x.label}</button>
        {/each}
        <span class="spacer"></span>
        {#if s.cancel !== null}
          <button type="button" class="btn" onclick={() => dialog.finish({ button: "cancel", values: {} })}>{s.cancel ?? "Cancel"}</button>
        {/if}
        <button type="submit" class="btn primary-action {s.danger ? 'danger solid' : 'primary'}">{s.confirm}</button>
      </div>
    </form>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed; inset: 0; z-index: 100; display: grid; place-items: start center;
    padding-top: 14vh; background: rgba(10, 18, 22, 0.32);
  }
  .dialog {
    width: min(480px, calc(100vw - 32px)); background: var(--surface); border-radius: 10px;
    border: 1px solid var(--line); box-shadow: var(--shadow-pop); padding: 18px 18px 14px;
    display: flex; flex-direction: column; gap: 12px;
    animation: rise 0.14s ease-out;
  }
  @keyframes rise { from { transform: translateY(6px); opacity: 0; } }
  h2 { margin: 0; font-size: 15px; font-weight: 700; }
  .body { margin: 0; color: var(--ink-2); white-space: pre-wrap; user-select: text; }
  .field { display: flex; flex-direction: column; gap: 5px; }
  .field span { font-size: 12px; color: var(--ink-2); font-weight: 600; }
  .check { display: flex; align-items: center; gap: 8px; }
  .error { margin: 0; color: var(--danger); }
  .buttons { display: flex; gap: 8px; align-items: center; margin-top: 4px; }
  .spacer { flex: 1; }
</style>
