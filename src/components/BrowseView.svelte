<script lang="ts">
  // Browse the repository's files as they were at any commit, branch or tag,
  // view any file, and export the whole tree as a ZIP.
  import { api, type FileContent, type GrepResult, type TreeEntry } from "../lib/api";
  import { markRanges } from "../lib/worddiff";
  import * as act from "../lib/actions";
  import { IMAGE_EXT } from "../lib/format";
  import { app } from "../lib/state.svelte";
  import { toasts } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";
  import Splitter from "./Splitter.svelte";
  import TextView from "./TextView.svelte";
  import { SvelteSet } from "svelte/reactivity";

  interface Node { name: string; path: string; dir: boolean; children: Node[]; entry?: TreeEntry }

  let entries = $state.raw<TreeEntry[]>([]);
  let loading = $state(false);
  let filter = $state("");
  let selected = $state<string | null>(null);
  let content = $state.raw<FileContent | null>(null);
  let image = $state<string | null>(null);
  const open = new SvelteSet<string>([""]);

  const rev = $derived(app.browse?.rev ?? "HEAD");
  const label = $derived(app.browse?.label ?? "HEAD");

  $effect(() => {
    const r = rev;
    loading = true;
    selected = null;
    api.treeAt(app.path, r)
      .then((e) => {
        entries = e;
        // Start with the README, as a project's page does.
        const readme = e.find((x) => x.kind === "file" && !x.path.includes("/") && /^readme(\.(md|markdown|txt|rst))?$/i.test(x.path));
        if (readme && rev === r && !selected) show(readme.path);
      })
      .catch((e) => { entries = []; toasts.push("error", String(e)); })
      .finally(() => (loading = false));
  });

  // Build the folder tree once per listing.
  const root = $derived.by(() => {
    const top: Node = { name: "", path: "", dir: true, children: [] };
    const dirs = new Map<string, Node>([["", top]]);
    const dirFor = (path: string): Node => {
      let d = dirs.get(path);
      if (d) return d;
      const cut = path.lastIndexOf("/");
      const parent = dirFor(cut < 0 ? "" : path.slice(0, cut));
      d = { name: path.slice(cut + 1), path, dir: true, children: [] };
      parent.children.push(d);
      dirs.set(path, d);
      return d;
    };
    for (const e of entries) {
      const cut = e.path.lastIndexOf("/");
      dirFor(cut < 0 ? "" : e.path.slice(0, cut)).children.push({ name: e.path.slice(cut + 1), path: e.path, dir: false, children: [], entry: e });
    }
    const sort = (n: Node) => {
      n.children.sort((a, b) => Number(b.dir) - Number(a.dir) || a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" }));
      n.children.forEach((c) => c.dir && sort(c));
    };
    sort(top);
    return top;
  });

  // Visible rows: the open folders' contents, or matching files while filtering.
  const rows = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (q) return entries.filter((e) => e.path.toLowerCase().includes(q)).slice(0, 2000).map((e) => ({ node: { name: e.path, path: e.path, dir: false, children: [], entry: e } as Node, depth: 0 }));
    const out: { node: Node; depth: number }[] = [];
    const walk = (n: Node, depth: number) => {
      for (const c of n.children) {
        out.push({ node: c, depth });
        if (c.dir && open.has(c.path)) walk(c, depth + 1);
      }
    };
    walk(root, 0);
    return out;
  });

  // Find in files: search the contents of every file at this revision.
  let mode = $state<"files" | "search">("files");
  let query = $state("");
  let caseSensitive = $state(false);
  let useRegex = $state(false);
  let found = $state.raw<GrepResult | null>(null);
  let searching = $state(false);
  let searchError = $state<string | null>(null);
  let hitLine = $state(0);
  let seq = 0;
  $effect(() => {
    const q = query, r = rev, c = caseSensitive, x = useRegex;
    if (mode !== "search" || !q.trim()) {
      found = null;
      searchError = null;
      return;
    }
    const my = ++seq;
    searching = true;
    const t = setTimeout(() => {
      api.grepTree(app.path, r, q, c, x)
        .then((g) => { if (my === seq) { found = g; searchError = null; } })
        .catch((e) => { if (my === seq) { found = null; searchError = String(e); } })
        .finally(() => { if (my === seq) searching = false; });
    }, 250);
    return () => clearTimeout(t);
  });
  const groups = $derived.by(() => {
    const m = new Map<string, GrepResult["matches"]>();
    for (const x of found?.matches ?? []) m.set(x.path, [...(m.get(x.path) ?? []), x]);
    return [...m];
  });

  /** The matching line, starting a little before the first match so it's in view. */
  function snippet(m: GrepResult["matches"][number]): string {
    const chars = [...m.text];
    const lead = chars.findIndex((c) => !/\s/.test(c));
    const from = Math.max(lead < 0 ? 0 : lead, (m.ranges[0]?.[0] ?? 0) - 12);
    const cut = from > Math.max(lead, 0);
    const text = (cut ? "…" : "") + chars.slice(from).join("");
    const shift = from - (cut ? 1 : 0);
    return markRanges(null, text, m.ranges.map(([a, b]) => [a - shift, b - shift] as [number, number]).filter(([a]) => a >= 0));
  }

  async function show(path: string, line = 0) {
    hitLine = line;
    if (selected === path && content) return;
    selected = path;
    content = null;
    image = null;
    try {
      const c = await api.fileContent(app.path, rev, path);
      if (selected !== path) return;
      content = c;
      if (c.binary && IMAGE_EXT.test(path)) {
        const bytes = await api.fileAt(app.path, rev, path);
        if (bytes && selected === path) {
          const type = path.toLowerCase().endsWith(".svg") ? "image/svg+xml" : "";
          image = URL.createObjectURL(new Blob([new Uint8Array(bytes)], type ? { type } : undefined));
        }
      }
    } catch (e) {
      toasts.push("error", String(e));
    }
  }

  const size = (n: number) => (n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(1)} MB`);
  const total = $derived(entries.reduce((s, e) => s + e.size, 0));
  const isCommit = $derived(/^[0-9a-f]{7,40}$/.test(rev));
</script>

<div class="browse">
  <header>
    <div class="title">
      <Icon name="folder" size={15} />
      <h2>Files at <span class="rev">{label}</span></h2>
      <span class="faint">{entries.length.toLocaleString()} files · {size(total)}</span>
      {#if loading}<span class="spinner"></span>{/if}
    </div>
    <div class="tools">
      <button class="btn small" onclick={() => (isCommit ? act.selectCommit(rev) : act.showHistory(rev))}><Icon name="history" size={13} /> Show in history</button>
      <button class="btn small" onclick={() => act.exportZip(rev, label)}><Icon name="fetch" size={13} /> Export as ZIP…</button>
    </div>
  </header>
  <div class="body" style:--side-w="{app.size('browse.tree', 300)}px">
    <div class="side">
      <div class="modes" role="tablist">
        <button role="tab" aria-selected={mode === "files"} class:on={mode === "files"} onclick={() => (mode = "files")}><Icon name="folder" size={12} /> Files</button>
        <button role="tab" aria-selected={mode === "search"} class:on={mode === "search"} onclick={() => (mode = "search")}><Icon name="search" size={12} /> Find in files</button>
      </div>
      {#if mode === "search"}
        <div class="filter">
          <Icon name="search" size={13} />
          <!-- svelte-ignore a11y_autofocus -->
          <input placeholder="Text in any file at {label}" bind:value={query} spellcheck="false" aria-label="Find in files" autofocus />
          <button class="opt" class:on={caseSensitive} onclick={() => (caseSensitive = !caseSensitive)} title="Match case" aria-pressed={caseSensitive}>Aa</button>
          <button class="opt mono" class:on={useRegex} onclick={() => (useRegex = !useRegex)} title="Regular expression" aria-pressed={useRegex}>.*</button>
        </div>
        <div class="tree results">
          {#if searchError}<div class="none err">{searchError}</div>
          {:else if searching && !found}<div class="none faint">Searching…</div>
          {:else if found}
            <div class="rsum faint">{found.matches.length.toLocaleString()}{found.more ? "+" : ""} match{found.matches.length === 1 ? "" : "es"} in {found.files} file{found.files === 1 ? "" : "s"}{searching ? " · updating…" : ""}</div>
            {#each groups as [path, ms] (path)}
              <div class="rfile" title={path}><Icon name="file" size={12} /> <b>{path.split("/").pop()}</b> <span class="faint">{path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : ""}</span> <span class="n">{ms.length}</span></div>
              {#each ms as m (m.line)}
                <button class="hit" class:sel={selected === path && hitLine === m.line} onclick={() => show(path, m.line)}>
                  <span class="no">{m.line}</span><span class="t mono">{@html snippet(m)}</span>
                </button>
              {/each}
            {:else}
              <div class="none faint">Nothing found.</div>
            {/each}
          {:else}
            <div class="none faint">Searches every text file as it was at {label}.</div>
          {/if}
        </div>
      {:else}
      <div class="filter">
        <Icon name="search" size={13} />
        <input placeholder="Filter files" bind:value={filter} spellcheck="false" aria-label="Filter files" />
      </div>
      <div class="tree" role="tree">
        {#each rows as { node, depth } (node.path)}
          <button class="node" class:sel={selected === node.path} style:padding-left="{10 + depth * 14}px" role="treeitem"
            aria-selected={selected === node.path} aria-expanded={node.dir ? open.has(node.path) : undefined}
            onclick={() => (node.dir ? (open.has(node.path) ? open.delete(node.path) : open.add(node.path)) : show(node.path))}>
            {#if node.dir}
              <span class="chev" class:on={open.has(node.path)}><Icon name="chevron" size={11} /></span>
              <Icon name="folder" size={13} />
            {:else}
              <span class="chev"></span>
              <Icon name={node.entry?.kind === "submodule" ? "remote" : "file"} size={13} />
            {/if}
            <span class="nm">{node.name}</span>
            {#if node.entry?.kind === "file"}<span class="sz faint">{size(node.entry.size)}</span>{/if}
          </button>
        {:else}
          <div class="none faint">{loading ? "Loading…" : filter ? "No files match." : "No files."}</div>
        {/each}
      </div>
      {/if}
    </div>
    <Splitter id="browse.tree" min={180} keep={320} />
    <div class="view">
      {#if !selected}
        <div class="empty"><div><strong>Pick a file</strong>See it exactly as it was at {label}.</div></div>
      {:else if image}
        <div class="img"><img src={image} alt={selected} /></div>
      {:else if content?.text != null}
        <div class="fhead mono">{selected}</div>
        <div class="text"><TextView text={content.text} path={selected} line={hitLine} /></div>
      {:else if content}
        <div class="empty"><div><strong>{content.binary ? "Binary file" : "Too large to show"}</strong>{size(content.size)}. Export as ZIP to get it.</div></div>
      {:else}
        <div class="empty"><div><span class="spinner"></span> Loading {selected}…</div></div>
      {/if}
    </div>
  </div>
</div>

<style>
  .browse { height: 100%; display: flex; flex-direction: column; min-height: 0; background: var(--surface); }
  header { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 10px 16px; border-bottom: 1px solid var(--line); flex-wrap: wrap; }
  .title { display: flex; align-items: center; gap: 8px; color: var(--ink-3); }
  h2 { margin: 0; font-size: calc(var(--fs) * 15); color: var(--ink); }
  .rev { font-family: var(--font-mono); font-size: calc(var(--cs) * 13); }
  .tools { display: flex; gap: 6px; }
  .body { flex: 1; min-height: 0; display: grid; grid-template-columns: var(--side-w) auto minmax(0, 1fr); }
  .side { display: flex; flex-direction: column; min-height: 0; min-width: 0; }
  .filter { display: flex; align-items: center; gap: 6px; margin: 8px; padding: 0 8px; border: 1px solid var(--line); border-radius: var(--radius); color: var(--ink-3); background: var(--input-bg); }
  .filter input { border: 0; background: transparent; box-shadow: none; padding: 5px 0; }
  .tree { flex: 1; overflow: auto; padding: 0 4px 10px; }
  .node { display: flex; align-items: center; gap: 5px; width: 100%; height: 26px; padding-right: 8px; border-radius: 5px; text-align: left; color: var(--ink-3); }
  .node:hover { background: var(--surface-2); }
  .node.sel { background: var(--sel); }
  .chev { width: 12px; display: inline-grid; flex: none; transition: rotate 0.1s; }
  .chev.on { rotate: 90deg; }
  .nm { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--ink); }
  .sz { font-size: calc(var(--fs) * 11); flex: none; }
  .none { padding: 12px; }
  .none.err { color: var(--danger); }
  .modes { display: flex; gap: 2px; margin: 8px 8px 0; background: var(--surface-3); border-radius: 7px; padding: 2px; }
  .modes button { flex: 1; display: flex; align-items: center; justify-content: center; gap: 5px; padding: 3px 6px; border-radius: 5px; font-weight: 600; font-size: calc(var(--fs) * 12); color: var(--ink-2); }
  .modes button.on { background: var(--surface); color: var(--ink); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.1); }
  .opt { flex: none; padding: 1px 5px; border-radius: 4px; font-size: calc(var(--fs) * 11); font-weight: 700; color: var(--ink-3); }
  .opt.on { background: var(--accent); color: var(--accent-ink); }
  .rsum { padding: 4px 8px 6px; font-size: calc(var(--fs) * 11.5); }
  .rfile { display: flex; align-items: center; gap: 4px; padding: 6px 6px 2px; font-size: calc(var(--fs) * 12); min-width: 0; white-space: nowrap; overflow: hidden; }
  .rfile .n { margin-left: auto; color: var(--ink-3); font-size: calc(var(--fs) * 11); }
  .hit { display: flex; gap: 8px; width: 100%; text-align: left; padding: 2px 6px 2px 10px; border-radius: 4px; min-width: 0; }
  .hit:hover { background: var(--surface-2); }
  .hit.sel { background: var(--sel); }
  .hit .no { flex: none; width: 3.5ch; text-align: right; color: var(--ink-3); font-size: calc(var(--cs) * 11); }
  .hit .t { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-size: calc(var(--cs) * 11.5); color: var(--ink-2); }
  .hit .t :global(mark.wd) { background: var(--warn-soft); color: var(--ink); border-radius: 2px; box-shadow: 0 0 0 1px var(--warn); }
  .view { min-width: 0; min-height: 0; display: flex; flex-direction: column; }
  .fhead { padding: 6px 12px; border-bottom: 1px solid var(--line-soft); color: var(--ink-2); font-size: calc(var(--cs) * 12); background: var(--surface-2); }
  .text { flex: 1; min-height: 0; }
  .img { padding: 20px; overflow: auto; }
  .img img { max-width: 100%; border: 1px solid var(--line); background: repeating-conic-gradient(var(--surface-3) 0 25%, var(--surface) 0 50%) 0 0 / 16px 16px; }
  .spinner { width: 12px; height: 12px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { rotate: 1turn; } }
</style>
