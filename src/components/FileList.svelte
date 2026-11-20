<script lang="ts">
  import type { FileEntry } from "../lib/api";
  import { draggable, dropTarget, type Payload } from "../lib/drag.svelte";
  import { basename, dirname, STATUS_GLYPH } from "../lib/format";
  import { treeRows, under } from "../lib/filetree";
  import { app } from "../lib/state.svelte";
  import Icon from "./Icon.svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { tick } from "svelte";
  import { nearEnd, Progressive } from "../lib/progressive.svelte";

  let {
    files,
    area,
    selected,
    onselect,
    ontoggle,
    oncontext,
    ondropfiles,
  }: {
    files: FileEntry[];
    area: "staged" | "unstaged";
    selected: Set<string>;
    onselect: (path: string, e: MouseEvent | KeyboardEvent) => void;
    ontoggle: (paths: string[]) => void;
    oncontext: (e: MouseEvent, path: string) => void;
    ondropfiles: (paths: string[]) => void;
  } = $props();

  function dragData(f: FileEntry) {
    const paths = selected.has(f.path) ? files.filter((x) => selected.has(x.path)).map((x) => x.path) : [f.path];
    return {
      payload: { kind: "files", paths, from: area } as Payload,
      label: paths.length === 1 ? basename(paths[0]) : `${paths.length} files`,
    };
  }

  // Flat list, or a folder tree (Preferences / the list's own toggle).
  const collapsed = new SvelteSet<string>();
  const tree = $derived(app.settings.fileTree);
  const rows = $derived(tree ? treeRows(files, (f) => f.path, collapsed) : files.map((f) => ({ kind: "file" as const, item: f, name: basename(f.path), depth: 0 })));
  const visible = $derived(rows.filter((r) => r.kind === "file").map((r) => (r as { item: FileEntry }).item));
  // Thousands of changed files: render them a slice at a time.
  const prog = new Progressive(300);
  const shown = $derived(rows.length > prog.limit ? rows.slice(0, prog.limit) : rows);
  let list: HTMLElement;

  // Coming back to this view (another tab, another view): bring the selected
  // file into sight, once, rather than leave it scrolled off below.
  let revealed = false;
  $effect(() => {
    const sel = app.selectedFile;
    if (revealed || !list || !rows.length) return;
    revealed = true;
    if (sel?.area !== area) return;
    const i = rows.findIndex((r) => r.kind === "file" && r.item.path === sel.path);
    if (i < 0) return;
    prog.reveal(i);
    tick().then(() => list?.querySelector<HTMLElement>(`.file[data-path="${CSS.escape(sel.path)}"]`)?.scrollIntoView({ block: "nearest" }));
  });

  function key(e: KeyboardEvent, f: FileEntry) {
    const i = visible.indexOf(f);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const step = e.key === "ArrowDown" ? 1 : -1;
      const next = visible[i + step];
      if (next) {
        onselect(next.path, e);
        prog.reveal(rows.findIndex((r) => r.kind === "file" && r.item === next));
        tick().then(() => list?.querySelector<HTMLElement>(`.file[data-path="${CSS.escape(next.path)}"]`)?.focus());
      }
    } else if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      ontoggle(selected.has(f.path) ? [...selected] : [f.path]);
    }
  }
</script>

<div class="files" bind:this={list}
  use:dropTarget={{ accepts: (p) => p.kind === "files" && p.from !== area, drop: (p) => p.kind === "files" && ondropfiles(p.paths) }}>
  {#each shown as row (row.kind === "dir" ? `d:${row.path}` : row.item.path)}
    {#if row.kind === "dir"}
      <div class="folder" style:padding-left="{8 + row.depth * 14}px" role="treeitem" aria-expanded={row.open} aria-selected="false" tabindex="-1"
        onclick={() => (collapsed.has(row.path) ? collapsed.delete(row.path) : collapsed.add(row.path))}
        onkeydown={(e) => e.key === "Enter" && (collapsed.has(row.path) ? collapsed.delete(row.path) : collapsed.add(row.path))}>
        <span class="chev" class:open={row.open}><Icon name="chevron" size={11} /></span>
        <Icon name="folder" size={13} />
        <span class="name">{row.name}</span>
        <span class="cnt">{row.count}</span>
        <button class="toggle" title={area === "staged" ? "Unstage folder" : "Stage folder"} aria-label="{area === 'staged' ? 'Unstage' : 'Stage'} {row.path}"
          onclick={(e) => { e.stopPropagation(); ontoggle(under(files, (f) => f.path, row.path)); }}>
          <Icon name={area === "staged" ? "minus" : "plus"} size={13} />
        </button>
      </div>
    {:else}
    {@const f = row.item}
    <div class="file" class:sel={selected.has(f.path)} data-path={f.path} role="option" aria-selected={selected.has(f.path)} tabindex="0"
      style:padding-left={tree ? `${8 + row.depth * 14 + 14}px` : null}
      title={f.oldPath ? `${f.oldPath} → ${f.path}` : f.path}
      onclick={(e) => onselect(f.path, e)}
      onkeydown={(e) => key(e, f)}
      oncontextmenu={(e) => oncontext(e, f.path)}
      use:draggable={() => dragData(f)}>
      <span class="st {f.status}">{STATUS_GLYPH[f.status]}</span>
      <span class="name"><b>{row.name}</b>{#if !tree}<span class="dir">{dirname(f.path)}</span>{/if}</span>
      <button class="toggle" title={area === "staged" ? "Unstage" : "Stage"} aria-label={area === "staged" ? `Unstage ${f.path}` : `Stage ${f.path}`}
        onclick={(e) => { e.stopPropagation(); ontoggle(selected.has(f.path) ? [...selected] : [f.path]); }}>
        <Icon name={area === "staged" ? "minus" : "plus"} size={13} />
      </button>
    </div>
    {/if}
  {:else}
    <div class="none faint">{area === "staged" ? "Drag files here, or press + on a change, to stage it" : "Nothing to stage"}</div>
  {/each}
  {#if shown.length < rows.length}
    <div class="more faint" use:nearEnd={prog.more}>{rows.length - shown.length} more…</div>
  {/if}
</div>

<style>
  .files { padding: 2px 6px 6px; min-height: 36px; border-radius: 6px; }
  .file {
    display: flex; align-items: center; gap: 8px; height: 26px; padding: 0 4px 0 8px;
    border-radius: 5px; outline-offset: -2px;
  }
  .file:hover { background: var(--surface-2); }
  .file.sel { background: var(--sel); }
  .file:focus-visible { outline: 2px solid var(--focus); }
  .st {
    width: 16px; height: 16px; border-radius: 4px; flex: none; display: grid; place-items: center;
    font-size: calc(var(--fs) * 10); font-weight: 700; font-family: var(--font-mono);
    color: var(--ink-2); background: var(--surface-3);
  }
  .st.added, .st.untracked { color: var(--add-ink); background: var(--add-bg-strong); }
  .st.deleted { color: var(--del-ink); background: var(--del-bg-strong); }
  .st.modified { color: var(--lane-1); background: color-mix(in srgb, var(--lane-1) 18%, transparent); }
  .st.renamed { color: var(--lane-3); background: color-mix(in srgb, var(--lane-3) 18%, transparent); }
  .name { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .name b { font-weight: 600; }
  .dir { margin-left: 0.4em; color: var(--ink-3); font-size: calc(var(--fs) * 12); }
  .toggle {
    display: grid; place-items: center; width: 22px; height: 22px; border-radius: 5px; flex: none;
    color: var(--ink-2); opacity: 0;
  }
  .file:hover .toggle, .file.sel .toggle, .file:focus-within .toggle { opacity: 1; }
  .toggle:hover { background: var(--accent); color: var(--accent-ink); }
  .none, .more { padding: 6px 8px; font-size: calc(var(--fs) * 12); }
  .folder { display: flex; align-items: center; gap: 6px; height: 24px; padding-right: 4px; border-radius: 5px; color: var(--ink-2); cursor: default; }
  .folder:hover { background: var(--surface-2); }
  .folder:hover .toggle { opacity: 1; }
  .folder .name { font-size: calc(var(--fs) * 12.5); }
  .chev { display: inline-grid; transition: rotate 0.1s; color: var(--ink-3); }
  .chev.open { rotate: 90deg; }
  .cnt { font-size: calc(var(--fs) * 11); color: var(--ink-3); }
</style>
