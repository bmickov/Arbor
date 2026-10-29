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
  }: {
    diff: FileDiff | null;
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
  $effect(() => {
    const sig = diff ? `${diff.path}|${diff.additions}|${diff.deletions}|${diff.hunks.map((h) => h.header + h.lines.length).join(";")}` : "";
    if (sig === lastSig) return;
    lastSig = sig;
    selected = new Set();
    anchor = null;
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

<div class="wrap">
<div class="diff mono" class:interactive={mode !== "readonly"}>
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
  {:else}
    {#each diff.hunks as hunk, h}
      <div class="hunk-head">
        <span class="range">{hunk.header.replace(/^(@@[^@]*@@).*/, "$1")}</span>
        <span class="ctx">{hunk.header.replace(/^@@[^@]*@@\s?/, "")}</span>
        {#if mode === "unstaged"}
          <button class="hbtn" onclick={() => fire("discard", hunkSel(h))} title="Discard this hunk"><Icon name="discard" size={13} /> Discard</button>
          <button class="hbtn strong" onclick={() => fire("stage", hunkSel(h))}><Icon name="plus" size={13} /> Stage hunk</button>
        {:else if mode === "staged"}
          <button class="hbtn strong" onclick={() => fire("unstage", hunkSel(h))}><Icon name="minus" size={13} /> Unstage hunk</button>
        {/if}
      </div>
      {#if split && mode === "readonly"}
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
          {#each hunk.lines as ln, l}
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
          {/each}
        </div>
      {/if}
    {/each}
    <div class="tail"></div>
  {/if}
</div>

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
  .diff { position: relative; height: 100%; overflow: auto; background: var(--surface); font-size: 12px; line-height: 19px; }
  .hunk-head {
    position: sticky; top: 0; z-index: 1; display: flex; align-items: center; gap: 10px;
    padding: 4px 8px 4px 12px; background: var(--surface-2); border-block: 1px solid var(--line-soft);
    font-family: var(--font-ui); font-size: 12px; color: var(--ink-3);
  }
  .hunk-head:first-child { border-top: 0; }
  .range { font-family: var(--font-mono); font-size: 11px; }
  .ctx { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font-mono); font-size: 11px; color: var(--ink-2); }
  .hbtn {
    display: inline-flex; align-items: center; gap: 4px; height: 22px; padding: 0 8px;
    border-radius: 5px; font-weight: 600; color: var(--ink-2); border: 1px solid transparent;
  }
  .hbtn:hover { background: var(--surface); border-color: var(--line); color: var(--ink); }
  .hbtn.strong { color: var(--accent); }

  .lines { min-width: max-content; }
  .line { display: grid; grid-template-columns: 44px 44px 18px 1fr; white-space: pre; }
  .interactive .line.add, .interactive .line.del { cursor: pointer; }
  .no { color: var(--ink-3); text-align: right; padding-right: 8px; font-size: 11px; opacity: 0.85; }
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
  table.split td.no { width: 44px; }
  table.split td.code { padding: 0 10px 0 6px; border-right: 1px solid var(--line-soft); }
  table.split td.add { background: var(--add-bg); }
  table.split td.del { background: var(--del-bg); }
  table.split td.blank { background: repeating-linear-gradient(135deg, transparent 0 5px, var(--line-soft) 5px 6px); }

  .images { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; padding: 16px; font-family: var(--font-ui); }
  figure { margin: 0; display: flex; flex-direction: column; gap: 6px; align-items: center; }
  figcaption { color: var(--ink-3); font-size: 12px; font-weight: 600; }
  figure img {
    max-width: 100%; max-height: 60vh; border: 1px solid var(--line);
    background: repeating-conic-gradient(var(--surface-3) 0 25%, var(--surface) 0 50%) 0 0 / 16px 16px;
  }
  .tail { height: 60px; }
  .empty { font-family: var(--font-ui); }

  .selbar {
    position: absolute; bottom: 16px; left: 50%; translate: -50% 0; width: max-content; display: flex; align-items: center; gap: 8px;
    padding: 6px 6px 6px 14px; border-radius: 10px; background: var(--surface); border: 1px solid var(--line);
    box-shadow: var(--shadow-pop); font-family: var(--font-ui); font-weight: 600; z-index: 2; font-size: 12px;
    animation: pop 0.12s ease-out;
  }
  @keyframes pop { from { translate: -50% 6px; opacity: 0; } }
</style>
