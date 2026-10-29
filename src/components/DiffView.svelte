<script lang="ts">
  import { api, type DiffLine, type FileDiff, type Selection } from "../lib/api";
  import { IMAGE_EXT } from "../lib/format";
  import { app } from "../lib/state.svelte";
  import Icon from "./Icon.svelte";

  type Mode = "unstaged" | "staged" | "readonly";
  type Act = "stage" | "unstage" | "discard";

  let {
    diff,
    mode = "readonly",
    split = false,
    onaction,
    image,
    loading = false,
  }: {
    diff: FileDiff | null;
    /** The requested diff hasn't arrived yet (big diffs take a moment). */
    loading?: boolean;
    mode?: Mode;
    split?: boolean;
    onaction?: (a: Act, sel: Selection[]) => void;
    /** Revisions for image previews: old/new side (null new = working tree). */
    image?: { old: string | null; new: string | null } | null;
  } = $props();

  // Selected changed lines, keyed "hunk:line".
  let selected = $state<Set<string>>(new Set());
  let anchor: { h: number; l: number } | null = null;
  let dragging = false;

  // Reset the selection only when the diff's content changes, not when the
  // same diff is reloaded by a background refresh.
  let lastSig = "";
  let lastPath = "";
  $effect(() => {
    const sig = diff ? `${diff.path}|${diff.additions}|${diff.deletions}|${diff.hunks.map((h) => h.header + h.lines.length).join(";")}` : "";
    if (sig === lastSig) return;
    lastSig = sig;
    selected = new Set();
    anchor = null;
    // A different file starts at the top; a refresh of the same file stays put.
    if (diff?.path !== lastPath && scroller) scroller.scrollTop = 0;
    lastPath = diff?.path ?? "";
  });

  const key = (h: number, l: number) => `${h}:${l}`;
  const isChange = (ln: DiffLine) => ln.origin !== " ";

  function rangeKeys(a: { h: number; l: number }, b: { h: number; l: number }): string[] {
    if (!diff) return [];
    const [s, e] = a.h < b.h || (a.h === b.h && a.l <= b.l) ? [a, b] : [b, a];
    const out: string[] = [];
    for (let h = s.h; h <= e.h; h++) {
      const lines = diff.hunks[h].lines;
      const from = h === s.h ? s.l : 0;
      const to = h === e.h ? e.l : lines.length - 1;
      for (let l = from; l <= to; l++) if (isChange(lines[l])) out.push(key(h, l));
    }
    return out;
  }

  function down(e: MouseEvent, h: number, l: number) {
    if (mode === "readonly" || e.button !== 0) return;
    e.preventDefault();
    const k = key(h, l);
    if (e.shiftKey && anchor) {
      selected = new Set([...selected, ...rangeKeys(anchor, { h, l })]);
    } else if (e.ctrlKey || e.metaKey) {
      const s = new Set(selected);
      s.has(k) ? s.delete(k) : s.add(k);
      selected = s;
      anchor = { h, l };
    } else {
      selected = new Set([k]);
      anchor = { h, l };
      dragging = true;
      window.addEventListener("mouseup", () => (dragging = false), { once: true });
    }
  }

  function enter(h: number, l: number) {
    if (dragging && anchor) selected = new Set(rangeKeys(anchor, { h, l }));
  }

  function selection(): Selection[] {
    const m = new Map<number, number[]>();
    for (const k of selected) {
      const [h, l] = k.split(":").map(Number);
      if (!m.has(h)) m.set(h, []);
      m.get(h)!.push(l);
    }
    return [...m].map(([hunk, lines]) => ({ hunk, lines }));
  }

  function hunkSel(h: number): Selection[] {
    const lines = diff!.hunks[h].lines.map((ln, i) => (isChange(ln) ? i : -1)).filter((i) => i >= 0);
    return [{ hunk: h, lines }];
  }

  function fire(a: Act, sel: Selection[]) {
    onaction?.(a, sel);
    selected = new Set();
  }

  const count = $derived(selected.size);
  const primary: Act = $derived(mode === "staged" ? "unstage" : "stage");

  // Pair removed/added runs for the side-by-side view.
  interface Row { left: DiffLine | null; right: DiffLine | null }
  function pairs(lines: DiffLine[]): Row[] {
    const rows: Row[] = [];
    let i = 0;
    while (i < lines.length) {
      if (lines[i].origin === " ") {
        rows.push({ left: lines[i], right: lines[i] });
        i++;
        continue;
      }
      const dels: DiffLine[] = [];
      const adds: DiffLine[] = [];
      while (i < lines.length && lines[i].origin === "-") dels.push(lines[i++]);
      while (i < lines.length && lines[i].origin === "+") adds.push(lines[i++]);
      for (let k = 0; k < Math.max(dels.length, adds.length); k++) rows.push({ left: dels[k] ?? null, right: adds[k] ?? null });
    }
    return rows;
  }

  // Big diffs are virtualized: every row has a fixed height, and only the
  // rows in (or near) the viewport are in the DOM. Small diffs render in full,
  // which keeps wrapping in the side-by-side view and native text selection.
  const VIRTUAL_MIN = 3000;
  const OVERSCAN = 600; // px rendered above and below the viewport

  const splitMode = $derived(split && mode === "readonly");
  const lineCount = $derived(diff ? diff.hunks.reduce((n, h) => n + h.lines.length, 0) : 0);
  const virtual = $derived(lineCount > VIRTUAL_MIN);
  // Line-number gutters grow with the largest line number (7 digits for a million lines).
  const width = (max: number) => (String(max).length <= 4 ? "44px" : `calc(${String(max).length}ch + 12px)`);
  const gutterOld = $derived(width(diff ? diff.hunks.reduce((m, h) => Math.max(m, h.oldStart + h.oldLines), 0) : 0));
  const gutterNew = $derived(width(diff ? diff.hunks.reduce((m, h) => Math.max(m, h.newStart + h.newLines), 0) : 0));
  const lineH = $derived(Math.round(app.settings.codeSize * 1.6));
  const headH = $derived(Math.max(32, Math.ceil((app.settings.uiSize / 13) * 12 * 1.4) + 10));

  // Flattened rows: hunk index, and line (or side-by-side pair) index, -1 for the hunk header.
  const layout = $derived.by(() => {
    if (!diff || !virtual) return null;
    const pairsOf = splitMode ? diff.hunks.map((h) => pairs(h.lines)) : null;
    let n = 0;
    diff.hunks.forEach((h, i) => (n += 1 + (pairsOf ? pairsOf[i].length : h.lines.length)));
    const rowH = new Int32Array(n), rowL = new Int32Array(n), top = new Float64Array(n + 1);
    let r = 0, y = 0, widest = 0;
    diff.hunks.forEach((h, i) => {
      rowH[r] = i; rowL[r] = -1; top[r++] = y; y += headH;
      const count = pairsOf ? pairsOf[i].length : h.lines.length;
      for (let l = 0; l < count; l++) {
        rowH[r] = i; rowL[r] = l; top[r++] = y; y += lineH;
      }
      if (!pairsOf) for (const ln of h.lines) {
        // Tabs are 4 columns wide (tab-size: 4); good enough for sizing the scroll width.
        let w = ln.content.length;
        if (w > widest || w * 4 > widest) {
          for (let k = ln.content.indexOf("\t"); k >= 0; k = ln.content.indexOf("\t", k + 1)) w += 3;
          if (w > widest) widest = w;
        }
      }
    });
    top[n] = y;
    return { n, rowH, rowL, top, pairsOf, widest };
  });

  let scroller = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let viewH = $state(0);

  // First row whose bottom is below `y`.
  function rowAt(top: Float64Array, n: number, y: number): number {
    let lo = 0, hi = n - 1;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (top[mid + 1] <= y) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  const win = $derived.by(() => {
    if (!layout || !layout.n) return null;
    const { n, top } = layout;
    const first = rowAt(top, n, Math.max(0, scrollTop - OVERSCAN));
    const last = rowAt(top, n, scrollTop + (viewH || 800) + OVERSCAN);
    const rows: number[] = [];
    for (let i = first; i <= last; i++) rows.push(i);
    // The hunk whose rows are at the top of the viewport keeps its header pinned.
    const pinned = layout.rowH[rowAt(top, n, scrollTop)];
    return { rows, y: top[first], pinned };
  });

  // Image previews.
  let images = $state<{ old: string | null; new: string | null } | null>(null);
  $effect(() => {
    const d = diff;
    images = null;
    if (!d || !image || !IMAGE_EXT.test(d.path) || !d.binary) return;
    const mk = (bytes: number[] | null) => {
      if (!bytes) return null;
      const type = d.path.toLowerCase().endsWith(".svg") ? "image/svg+xml" : "";
      return URL.createObjectURL(new Blob([new Uint8Array(bytes)], type ? { type } : undefined));
    };
    const repo = app.path;
    Promise.all([
      d.status === "added" || d.status === "untracked" || !image.old ? Promise.resolve(null) : api.fileAt(repo, image.old, d.oldPath ?? d.path),
      d.status === "deleted" ? Promise.resolve(null) : api.fileAt(repo, image.new, d.path),
    ]).then(([o, n]) => (images = { old: mk(o), new: mk(n) })).catch(() => {});
  });
</script>

{#snippet head(h: number)}
  {@const hunk = diff!.hunks[h]}
  <div class="hunk-head">
    <span class="range">{hunk.header.replace(/^(@@[^@]*@@).*/, "$1")}</span>
    <span class="ctx">{hunk.header.replace(/^@@[^@]*@@\s?/, "")}</span>
    {#if virtual}<span class="count">{hunk.lines.length.toLocaleString()} lines</span>{/if}
    {#if mode === "unstaged"}
      <button class="hbtn" onclick={() => fire("discard", hunkSel(h))} title="Discard this hunk"><Icon name="discard" size={13} /> Discard</button>
      <button class="hbtn strong" onclick={() => fire("stage", hunkSel(h))}><Icon name="plus" size={13} /> Stage hunk</button>
    {:else if mode === "staged"}
      <button class="hbtn strong" onclick={() => fire("unstage", hunkSel(h))}><Icon name="minus" size={13} /> Unstage hunk</button>
    {/if}
  </div>
{/snippet}

{#snippet line(h: number, l: number)}
  {@const ln = diff!.hunks[h].lines[l]}
  {@const sel = selected.has(key(h, l))}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="line {ln.origin === '+' ? 'add' : ln.origin === '-' ? 'del' : ''}" class:sel
    onmousedown={(e) => isChange(ln) && down(e, h, l)}
    onmouseenter={() => isChange(ln) && enter(h, l)}>
    <span class="no">{ln.oldNo ?? ""}</span>
    <span class="no">{ln.newNo ?? ""}</span>
    <span class="sign">{ln.origin === " " ? "" : ln.origin}</span>
    <span class="code">{ln.content}{#if !ln.nl}<span class="eof" title="No newline at end of file">⏎̸</span>{/if}</span>
  </div>
{/snippet}

{#snippet pairRow(row: Row)}
  <div class="prow">
    <span class="no">{row.left?.oldNo ?? ""}</span>
    <span class="code {row.left ? (row.left.origin === '-' ? 'del' : '') : 'blank'}" title={row.left?.content}>{row.left?.content ?? ""}</span>
    <span class="no">{row.right?.newNo ?? ""}</span>
    <span class="code {row.right ? (row.right.origin === '+' ? 'add' : '') : 'blank'}" title={row.right?.content}>{row.right?.content ?? ""}</span>
  </div>
{/snippet}

<div class="wrap">
<div class="diff mono" class:interactive={mode !== "readonly"} class:v={virtual}
  style="--lh: {lineH}px; --hh: {headH}px; --nwo: {gutterOld}; --nwn: {gutterNew}"
  bind:this={scroller} bind:clientHeight={viewH} onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}>
  {#if !diff}
    <div class="empty"><div><strong>No changes</strong>Select a file to see its changes.</div></div>
  {:else if diff.binary}
    {#if images}
      <div class="images">
        <figure><figcaption>Before</figcaption>{#if images.old}<img src={images.old} alt="Before" />{:else}<span class="faint">None</span>{/if}</figure>
        <figure><figcaption>After</figcaption>{#if images.new}<img src={images.new} alt="After" />{:else}<span class="faint">Deleted</span>{/if}</figure>
      </div>
    {:else}
      <div class="empty"><div><strong>Binary file</strong>{diff.status === "added" || diff.status === "untracked" ? "Added" : diff.status === "deleted" ? "Deleted" : "Changed"} — no text diff to show.</div></div>
    {/if}
  {:else if diff.tooLarge}
    <div class="empty"><div><strong>This diff is too large to show</strong>{diff.additions} additions and {diff.deletions} deletions.</div></div>
  {:else if !diff.hunks.length}
    <div class="empty"><div><strong>No content changes</strong>{diff.oldMode !== diff.newMode ? `File mode changed from ${diff.oldMode.toString(8)} to ${diff.newMode.toString(8)}.` : diff.status === "renamed" ? "The file was renamed." : "The file is empty."}</div></div>
  {:else if virtual && layout && win}
    <div class="vpin">{@render head(win.pinned)}</div>
    <div class="vspace" class:vsplit={splitMode}
      style="height: {layout.top[layout.n] + 60}px; {splitMode ? '' : `min-width: calc(${layout.widest}ch + var(--nwo) + var(--nwn) + 42px);`}">
      <div class="vwin" style="transform: translateY({win.y}px)">
        {#each win.rows as i (i)}
          {@const h = layout.rowH[i]}
          {@const l = layout.rowL[i]}
          {#if l < 0}
            {@render head(h)}
          {:else if layout.pairsOf}
            {@render pairRow(layout.pairsOf[h][l])}
          {:else}
            {@render line(h, l)}
          {/if}
        {/each}
      </div>
    </div>
  {:else}
    {#each diff.hunks as hunk, h}
      {@render head(h)}
      {#if splitMode}
        <table class="split">
          <tbody>
            {#each pairs(hunk.lines) as row}
              <tr>
                <td class="no">{row.left?.oldNo ?? ""}</td>
                <td class="code {row.left ? (row.left.origin === '-' ? 'del' : '') : 'blank'}">{row.left?.content ?? ""}</td>
                <td class="no">{row.right?.newNo ?? ""}</td>
                <td class="code {row.right ? (row.right.origin === '+' ? 'add' : '') : 'blank'}">{row.right?.content ?? ""}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else}
        <div class="lines">
          {#each hunk.lines as _, l}
            {@render line(h, l)}
          {/each}
        </div>
      {/if}
    {/each}
    <div class="tail"></div>
  {/if}
</div>

  {#if loading}
    <div class="loading" class:blank={!diff}><span class="spin"></span>Loading changes…</div>
  {/if}

  {#if count && mode !== "readonly"}
    <div class="selbar">
      <span>{count} line{count === 1 ? "" : "s"} selected</span>
      {#if mode === "unstaged"}
        <button class="btn small danger" onclick={() => fire("discard", selection())}>Discard</button>
      {/if}
      <button class="btn small primary" onclick={() => fire(primary, selection())}>{primary === "stage" ? "Stage" : "Unstage"} lines</button>
      <button class="btn small ghost" onclick={() => (selected = new Set())} aria-label="Clear selection"><Icon name="x" size={13} /></button>
    </div>
  {/if}
</div>

<style>
  .wrap { position: relative; height: 100%; }
  .diff { position: relative; height: 100%; overflow: auto; background: var(--surface); font-size: calc(var(--cs) * 12); line-height: 1.6; }
  .hunk-head {
    position: sticky; top: 0; z-index: 1; display: flex; align-items: center; gap: 10px;
    padding: 4px 8px 4px 12px; background: var(--surface-2); border-block: 1px solid var(--line-soft);
    font-family: var(--font-ui); font-size: calc(var(--fs) * 12); color: var(--ink-3);
  }
  .hunk-head:first-child { border-top: 0; }
  .range { font-family: var(--font-mono); font-size: calc(var(--cs) * 11); }
  .ctx { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font-mono); font-size: calc(var(--cs) * 11); color: var(--ink-2); }
  .hbtn {
    display: inline-flex; align-items: center; gap: 4px; height: 22px; padding: 0 8px;
    border-radius: 5px; font-weight: 600; color: var(--ink-2); border: 1px solid transparent;
  }
  .hbtn:hover { background: var(--surface); border-color: var(--line); color: var(--ink); }
  .hbtn.strong { color: var(--accent); }

  .lines { min-width: max-content; }
  .line { display: grid; grid-template-columns: var(--nwo) var(--nwn) 18px 1fr; white-space: pre; }
  .interactive .line.add, .interactive .line.del { cursor: pointer; }
  .no { color: var(--ink-3); text-align: right; padding-right: 8px; font-size: calc(var(--cs) * 11); opacity: 0.85; }
  .sign { text-align: center; color: var(--ink-3); }
  .code { padding-right: 24px; tab-size: 4; user-select: text; }
  .interactive .code { user-select: none; }
  .line.add { background: var(--add-bg); }
  .line.add .sign, .line.add .no { color: var(--add-ink); }
  .line.del { background: var(--del-bg); }
  .line.del .sign, .line.del .no { color: var(--del-ink); }
  .line.sel { box-shadow: inset 3px 0 0 var(--accent); }
  .line.add.sel { background: var(--add-bg-strong); }
  .line.del.sel { background: var(--del-bg-strong); }
  .eof { color: var(--del-ink); margin-left: 4px; }

  table.split { width: 100%; border-collapse: collapse; table-layout: fixed; }
  table.split td { white-space: pre-wrap; word-break: break-all; vertical-align: top; padding: 0; }
  table.split td.no:first-child { width: var(--nwo); }
  table.split td.no { width: var(--nwn); }
  table.split td.code { padding: 0 10px 0 6px; border-right: 1px solid var(--line-soft); }
  table.split td.add { background: var(--add-bg); }
  table.split td.del { background: var(--del-bg); }
  table.split td.blank { background: repeating-linear-gradient(135deg, transparent 0 5px, var(--line-soft) 5px 6px); }

  .images { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; padding: 16px; font-family: var(--font-ui); }
  figure { margin: 0; display: flex; flex-direction: column; gap: 6px; align-items: center; }
  figcaption { color: var(--ink-3); font-size: calc(var(--fs) * 12); font-weight: 600; }
  figure img {
    max-width: 100%; max-height: 60vh; border: 1px solid var(--line);
    background: repeating-conic-gradient(var(--surface-3) 0 25%, var(--surface) 0 50%) 0 0 / 16px 16px;
  }
  .tail { height: 60px; }

  /* Virtualized rendering for big diffs: fixed row heights. */
  .count { font-size: calc(var(--fs) * 11); white-space: nowrap; }
  .vspace { position: relative; }
  .vwin { position: absolute; top: 0; left: 0; right: 0; will-change: transform; }
  .v .hunk-head { height: var(--hh); position: static; border-top: 1px solid var(--line-soft); }
  .v .line, .v .prow { height: var(--lh); line-height: var(--lh); overflow: hidden; }
  .vpin { position: sticky; top: 0; left: 0; height: 0; z-index: 2; }
  .vpin > .hunk-head { position: absolute; top: 0; left: 0; right: 0; }
  .vspace.vsplit { width: 100%; }
  .prow { display: grid; grid-template-columns: var(--nwo) minmax(0, 1fr) var(--nwn) minmax(0, 1fr); white-space: pre; }
  .prow .code { overflow: hidden; text-overflow: ellipsis; padding: 0 10px 0 6px; border-right: 1px solid var(--line-soft); }
  .prow .code.add { background: var(--add-bg); }
  .prow .code.del { background: var(--del-bg); }
  .prow .code.blank { background: repeating-linear-gradient(135deg, transparent 0 5px, var(--line-soft) 5px 6px); }
  .empty { font-family: var(--font-ui); }

  .loading {
    position: absolute; inset: 0; z-index: 3; display: grid; place-content: center; grid-auto-flow: column; gap: 10px; align-items: center;
    background: color-mix(in srgb, var(--surface) 70%, transparent); color: var(--ink-2);
    font-family: var(--font-ui); font-size: calc(var(--fs) * 13);
    /* Quick diffs never flash the overlay. */
    opacity: 0; animation: show 0.1s 0.15s forwards;
  }
  .loading.blank { background: var(--surface); }
  @keyframes show { to { opacity: 1; } }
  @media (prefers-reduced-motion: reduce) { .loading { opacity: 1; } }
  .spin { width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { rotate: 1turn; } }
  .selbar {
    position: absolute; bottom: 16px; left: 50%; translate: -50% 0; width: max-content; display: flex; align-items: center; gap: 8px;
    padding: 6px 6px 6px 14px; border-radius: 10px; background: var(--surface); border: 1px solid var(--line);
    box-shadow: var(--shadow-pop); font-family: var(--font-ui); font-weight: 600; z-index: 2; font-size: calc(var(--fs) * 12);
    animation: pop 0.12s ease-out;
  }
  @keyframes pop { from { translate: -50% 6px; opacity: 0; } }
</style>
