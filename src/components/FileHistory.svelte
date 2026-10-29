<script lang="ts">
  import { api, type Blame, type FileDiff, type FileHistoryEntry } from "../lib/api";
  import { authorHue, basename, fullDate, initials, relTime } from "../lib/format";
  import { app } from "../lib/state.svelte";
  import { toasts } from "../lib/ui.svelte";
  import DiffView from "./DiffView.svelte";
  import Icon from "./Icon.svelte";

  let { path, onclose }: { path: string; onclose: () => void } = $props();

  let tab = $state<"history" | "blame">("history");
  let entries = $state<FileHistoryEntry[]>([]);
  let sel = $state<FileHistoryEntry | null>(null);
  let diff = $state<FileDiff | null>(null);
  let blame = $state<Blame | null>(null);
  let loading = $state(true);

  $effect(() => {
    const p = path;
    loading = true;
    api.fileHistory(app.path, p).then((e) => {
      entries = e;
      sel = e[0] ?? null;
    }).catch((e) => toasts.push("error", String(e))).finally(() => (loading = false));
  });

  $effect(() => {
    const s = sel;
    if (!s) return;
    api.commitFileDiff(app.path, s.commit.oid, s.path, app.diffSettings).then((d) => (diff = d)).catch(() => (diff = null));
  });

  $effect(() => {
    if (tab !== "blame" || blame) return;
    api.blame(app.path, path).then((b) => (blame = b)).catch((e) => {
      toasts.push("error", String(e));
      tab = "history";
    });
  });

  // One gutter label per blame hunk, on its first line.
  const gutter = $derived.by(() => {
    if (!blame) return [];
    const out = new Array(blame.lines.length).fill(null);
    for (const h of blame.hunks) out[h.start - 1] = h;
    return out;
  });
  const hunkIndex = $derived.by(() => {
    if (!blame) return [];
    const out = new Array(blame.lines.length).fill(0);
    blame.hunks.forEach((h, k) => {
      for (let i = h.start - 1; i < h.start - 1 + h.count; i++) out[i] = k;
    });
    return out;
  });

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onkeydown={key} onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="drawer" role="dialog" aria-modal="true" aria-label="History of {path}">
    <header>
      <Icon name="file" />
      <div class="title"><b>{basename(path)}</b> <span class="faint">{path}</span></div>
      <div class="tabs" role="tablist">
        <button role="tab" aria-selected={tab === "history"} class:on={tab === "history"} onclick={() => (tab = "history")}>History</button>
        <button role="tab" aria-selected={tab === "blame"} class:on={tab === "blame"} onclick={() => (tab = "blame")}>Blame</button>
      </div>
      <button class="close" onclick={onclose} aria-label="Close"><Icon name="x" /></button>
    </header>

    {#if tab === "history"}
      <div class="body">
        <div class="list">
          {#each entries as e (e.commit.oid)}
            <button class="entry" class:sel={sel?.commit.oid === e.commit.oid} onclick={() => (sel = e)}>
              <span class="sum">{e.commit.summary}</span>
              <span class="meta faint">{e.commit.author} · {relTime(e.commit.time)} · <span class="mono">{e.commit.short}</span></span>
              {#if e.path !== path}<span class="renamed">as {e.path}</span>{/if}
            </button>
          {:else}
            <div class="empty"><div>{loading ? "Loading…" : "No commits change this file."}</div></div>
          {/each}
        </div>
        <div class="diff">
          <DiffView {diff} mode="readonly" split={app.settings.splitDiff} image={sel ? { old: sel.commit.parents[0] ?? null, new: sel.commit.oid } : null} />
        </div>
      </div>
    {:else if blame}
      {#if blame.binary}
        <div class="empty"><div><strong>Binary file</strong>There are no lines to blame.</div></div>
      {:else}
        <div class="blame mono">
          {#each blame.lines as line, i}
            {@const h = gutter[i]}
            <div class="bl" class:start={!!h} class:odd={hunkIndex[i] % 2 === 1}>
              <span class="who">
                {#if h}
                  <button class="hc" title="{h.summary}\n{h.author}, {fullDate(h.time)}" onclick={() => { app.selectedCommit = h.oid; app.view = "history"; onclose(); if (!app.log.length) app.loadLog(); }}>
                    <span class="av" style:--h={authorHue(h.author)}>{initials(h.author)}</span>
                    <span class="hs">{h.summary}</span>
                    <span class="age">{relTime(h.time)}</span>
                  </button>
                {/if}
              </span>
              <span class="no">{i + 1}</span>
              <span class="code">{line}</span>
            </div>
          {/each}
        </div>
      {/if}
    {:else}
      <div class="empty"><div>Loading blame…</div></div>
    {/if}
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 70; background: var(--backdrop); display: flex; justify-content: flex-end; }
  .drawer {
    width: min(1100px, 92vw); height: 100%; background: var(--surface); box-shadow: var(--shadow-pop);
    display: flex; flex-direction: column; animation: in 0.16s ease-out;
  }
  @keyframes in { from { translate: 24px 0; opacity: 0; } }
  header { display: flex; align-items: center; gap: 10px; padding: 10px 12px 10px 16px; border-bottom: 1px solid var(--line); }
  .title { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .tabs { display: flex; background: var(--surface-3); border-radius: 7px; padding: 2px; }
  .tabs button { padding: 4px 12px; border-radius: 5px; font-weight: 600; color: var(--ink-2); }
  .tabs button.on { background: var(--surface); color: var(--ink); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08); }
  .close { display: grid; place-items: center; width: 28px; height: 28px; border-radius: 6px; color: var(--ink-2); }
  .close:hover { background: var(--surface-3); }
  .body { flex: 1; min-height: 0; display: grid; grid-template-columns: 320px 1fr; }
  .list { overflow-y: auto; border-right: 1px solid var(--line); padding: 6px; }
  .entry { display: flex; flex-direction: column; gap: 2px; width: 100%; text-align: left; padding: 7px 10px; border-radius: 6px; }
  .entry:hover { background: var(--surface-2); }
  .entry.sel { background: var(--sel); }
  .sum { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .meta { font-size: calc(var(--fs) * 12); }
  .renamed { font-size: calc(var(--fs) * 11); color: var(--lane-3); }
  .diff { min-width: 0; min-height: 0; }

  .blame { flex: 1; overflow: auto; font-size: calc(var(--cs) * 12); line-height: 1.65; }
  .bl { display: grid; grid-template-columns: 300px 46px 1fr; white-space: pre; }
  .bl.odd { background: var(--surface-2); }
  .bl.start { border-top: 1px solid var(--line-soft); }
  .who { overflow: hidden; border-right: 1px solid var(--line-soft); font-family: var(--font-ui); }
  .hc { display: flex; align-items: center; gap: 6px; width: 100%; padding: 0 8px; height: calc(var(--cs) * 19.8); text-align: left; }
  .hc:hover .hs { color: var(--accent); text-decoration: underline; }
  .av {
    width: 16px; height: 16px; border-radius: 50%; flex: none; display: grid; place-items: center; font-size: calc(var(--fs) * 8); font-weight: 700;
    background: hsl(var(--h) var(--av-bg-s) var(--av-bg-l)); color: hsl(var(--h) var(--av-ink-s) var(--av-ink-l));
  }
  .hs { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; font-size: calc(var(--fs) * 12); }
  .age { color: var(--ink-3); font-size: calc(var(--fs) * 11); }
  .no { color: var(--ink-3); text-align: right; padding-right: 10px; font-size: calc(var(--cs) * 11); }
  .code { user-select: text; padding-right: 20px; tab-size: 4; }
</style>
