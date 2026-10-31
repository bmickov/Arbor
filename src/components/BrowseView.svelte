<script lang="ts">
  // Browse the repository's files as they were at any commit, branch or tag,
  // view any file, and export the whole tree as a ZIP.
  import { api, type FileContent, type TreeEntry } from "../lib/api";
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
      .then((e) => (entries = e))
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

  async function show(path: string) {
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
    </div>
    <Splitter id="browse.tree" min={180} keep={320} />
    <div class="view">
      {#if !selected}
        <div class="empty"><div><strong>Pick a file</strong>See it exactly as it was at {label}.</div></div>
      {:else if image}
        <div class="img"><img src={image} alt={selected} /></div>
      {:else if content?.text != null}
        <div class="fhead mono">{selected}</div>
        <div class="text"><TextView text={content.text} path={selected} /></div>
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
  .view { min-width: 0; min-height: 0; display: flex; flex-direction: column; }
  .fhead { padding: 6px 12px; border-bottom: 1px solid var(--line-soft); color: var(--ink-2); font-size: calc(var(--cs) * 12); background: var(--surface-2); }
  .text { flex: 1; min-height: 0; }
  .img { padding: 20px; overflow: auto; }
  .img img { max-width: 100%; border: 1px solid var(--line); background: repeating-conic-gradient(var(--surface-3) 0 25%, var(--surface) 0 50%) 0 0 / 16px 16px; }
  .spinner { width: 12px; height: 12px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { rotate: 1turn; } }
</style>
