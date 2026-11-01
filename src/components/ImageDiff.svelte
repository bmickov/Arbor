<script lang="ts">
  // Compare two versions of an image: side by side, a swipe between them,
  // one faded over the other (onion skin), or their pixel difference.
  import { app } from "../lib/state.svelte";

  let { before, after }: { before: string | null; after: string | null } = $props();

  type Mode = "side" | "swipe" | "onion" | "difference";
  const both = $derived(!!before && !!after);
  let mode = $state<Mode>((app.settings.imageMode as Mode) || "side");
  const shown = $derived<Mode>(both ? mode : "side");
  let split = $state(50);
  let fade = $state(50);
  let sizeA = $state<[number, number] | null>(null);
  let sizeB = $state<[number, number] | null>(null);
  let stage = $state<HTMLDivElement>();

  function pick(m: Mode) {
    mode = m;
    app.settings.imageMode = m;
    app.saveSettings();
  }

  function drag(e: PointerEvent) {
    if (!stage) return;
    const move = (ev: PointerEvent) => {
      const r = stage!.getBoundingClientRect();
      split = Math.max(0, Math.min(100, ((ev.clientX - r.left) / r.width) * 100));
    };
    move(e);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", () => window.removeEventListener("pointermove", move), { once: true });
  }

  const nat = (e: Event): [number, number] => {
    const i = e.currentTarget as HTMLImageElement;
    return [i.naturalWidth, i.naturalHeight];
  };
  const dims = (s: [number, number] | null) => (s ? `${s[0]} × ${s[1]}` : "");
  const sizeNote = $derived(sizeA && sizeB && (sizeA[0] !== sizeB[0] || sizeA[1] !== sizeB[1]) ? `${dims(sizeA)} → ${dims(sizeB)}` : dims(sizeB ?? sizeA));
</script>

<div class="imgdiff">
  <div class="bar">
    {#if both}
      <div class="seg" role="radiogroup" aria-label="Comparison">
        {#each [["side", "Side by side"], ["swipe", "Swipe"], ["onion", "Onion skin"], ["difference", "Difference"]] as [id, label]}
          <button role="radio" aria-checked={shown === id} class:on={shown === id} onclick={() => pick(id as Mode)}>{label}</button>
        {/each}
      </div>
    {/if}
    {#if shown === "onion"}
      <label class="slider">Before <input type="range" min="0" max="100" bind:value={fade} aria-label="Fade" /> After</label>
    {/if}
    <span class="sp"></span>
    <span class="faint dims">{sizeNote}</span>
  </div>

  {#if shown === "side"}
    <div class="side">
      <figure><figcaption>Before</figcaption>{#if before}<img src={before} alt="Before" onload={(e) => (sizeA = nat(e))} />{:else}<span class="faint none">None (new image)</span>{/if}</figure>
      <figure><figcaption>After</figcaption>{#if after}<img src={after} alt="After" onload={(e) => (sizeB = nat(e))} />{:else}<span class="faint none">Deleted</span>{/if}</figure>
    </div>
  {:else}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="stack" bind:this={stage} onpointerdown={shown === "swipe" ? drag : undefined} class:swipe={shown === "swipe"}>
      <img class="base" src={before} alt="Before" onload={(e) => (sizeA = nat(e))} />
      <img class="top" src={after} alt="After" onload={(e) => (sizeB = nat(e))}
        style:clip-path={shown === "swipe" ? `inset(0 0 0 ${split}%)` : null}
        style:opacity={shown === "onion" ? fade / 100 : null}
        style:mix-blend-mode={shown === "difference" ? "difference" : null} />
      {#if shown === "swipe"}
        <div class="handle" style:left="{split}%"><span>◀ ▶</span></div>
        <span class="tag l">Before</span><span class="tag r">After</span>
      {/if}
    </div>
    {#if shown === "difference"}<p class="faint hint">Identical pixels are black; anything brighter changed.</p>{/if}
  {/if}
</div>

<style>
  .imgdiff { padding: 12px 16px 20px; display: flex; flex-direction: column; gap: 12px; }
  .bar { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; font-family: var(--font-ui); font-size: calc(var(--fs) * 12); }
  .seg { display: flex; background: var(--surface-3); border-radius: 7px; padding: 2px; }
  .seg button { padding: 3px 10px; border-radius: 5px; font-weight: 600; color: var(--ink-2); }
  .seg button.on { background: var(--surface); color: var(--ink); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.1); }
  .slider { display: flex; align-items: center; gap: 6px; color: var(--ink-3); }
  .sp { flex: 1; }
  .side { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
  figure { margin: 0; display: flex; flex-direction: column; gap: 6px; align-items: flex-start; min-width: 0; }
  figcaption { font-family: var(--font-ui); font-size: calc(var(--fs) * 11); font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: 0.04em; }
  img { max-width: 100%; border: 1px solid var(--line); background: repeating-conic-gradient(var(--surface-3) 0 25%, var(--surface) 0 50%) 0 0 / 16px 16px; }
  .none { padding: 20px 0; }
  .stack { position: relative; display: inline-grid; align-self: flex-start; max-width: 100%; user-select: none; }
  .stack img { grid-area: 1 / 1; }
  .stack .top { background: none; border-color: transparent; }
  .stack.swipe { cursor: ew-resize; }
  .handle { position: absolute; top: 0; bottom: 0; width: 2px; margin-left: -1px; background: var(--accent); pointer-events: none; }
  .handle span { position: absolute; top: 50%; left: 50%; translate: -50% -50%; background: var(--accent); color: var(--accent-ink); font-size: 9px; padding: 3px 4px; border-radius: 4px; white-space: nowrap; }
  .tag { position: absolute; top: 6px; font-family: var(--font-ui); font-size: calc(var(--fs) * 10.5); font-weight: 700; background: var(--surface); padding: 1px 6px; border-radius: 4px; opacity: 0.85; }
  .tag.l { left: 6px; }
  .tag.r { right: 6px; }
  .hint { margin: 0; font-family: var(--font-ui); font-size: calc(var(--fs) * 12); }
</style>
