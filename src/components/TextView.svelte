<script lang="ts">
  // A read-only text viewer with line numbers. Only the lines in view are in
  // the DOM, so files with hundreds of thousands of lines stay fast.
  import { Highlighter, languageFor } from "../lib/highlight";
  import { app } from "../lib/state.svelte";

  let { text, path = "", line = 0 }: { text: string; path?: string; /** 1-based line to show and highlight. */ line?: number } = $props();

  const lines = $derived(text.replace(/\n$/, "").split("\n"));
  const lineH = $derived(Math.round(app.settings.codeSize * 1.6));
  const digits = $derived(String(lines.length).length);
  // Widest line, for the horizontal scroll width (tabs count as 4 columns).
  const widest = $derived(lines.reduce((m, l) => Math.max(m, l.length + (l.includes("\t") ? l.split("\t").length * 3 : 0)), 0));

  let scroller = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let viewH = $state(600);
  const OVERSCAN = 30;
  const first = $derived(Math.max(0, Math.floor(scrollTop / lineH) - OVERSCAN));
  const last = $derived(Math.min(lines.length, Math.ceil((scrollTop + viewH) / lineH) + OVERSCAN));

  // Colour by language, a chunk of lines at a time as they come into view.
  const hl = $derived.by(() => {
    const lang = app.settings.syntax ? languageFor(path) : null;
    const ls = lines;
    return lang ? new Highlighter(lang, (i) => ls[i], ls.length) : null;
  });

  // A new file starts at the top, or at the line asked for.
  $effect(() => {
    void path;
    const l = line;
    if (!scroller) return;
    const top = l > 0 ? Math.max(0, (l - 1) * lineH - scroller.clientHeight / 3) : 0;
    scroller.scrollTop = top;
    scrollTop = top;
  });
</script>

<div class="tv mono" bind:this={scroller} bind:clientHeight={viewH} onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
  style="--lh: {lineH}px; --nw: calc({digits}ch + 18px)">
  <div class="space" style="height: {lines.length * lineH + 40}px; min-width: calc({widest}ch + {digits}ch + 50px)">
    <div class="win" style="transform: translateY({first * lineH}px)">
      {#each lines.slice(first, last) as l, k (first + k)}
        {@const html = hl?.get(first + k) ?? null}
        <div class="ln" class:hit={first + k + 1 === line}><span class="no">{first + k + 1}</span><span class="code">{#if html != null}{@html html}{:else}{l}{/if}</span></div>
      {/each}
    </div>
  </div>
</div>

<style>
  .tv { height: 100%; overflow: auto; background: var(--surface); font-size: calc(var(--cs) * 12); }
  .space { position: relative; }
  .win { position: absolute; top: 0; left: 0; right: 0; }
  .ln { display: grid; grid-template-columns: var(--nw) 1fr; height: var(--lh); line-height: var(--lh); white-space: pre; }
  .no { color: var(--ink-3); text-align: right; padding-right: 12px; font-size: calc(var(--cs) * 11); user-select: none; opacity: 0.85; }
  .code { tab-size: 4; user-select: text; padding-right: 24px; }
  .ln.hit { background: var(--warn-soft); box-shadow: inset 3px 0 0 var(--warn); }
</style>
