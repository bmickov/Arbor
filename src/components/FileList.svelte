<script lang="ts">
  import type { FileEntry } from "../lib/api";
  import { draggable, dropTarget, type Payload } from "../lib/drag.svelte";
  import { basename, dirname, STATUS_GLYPH } from "../lib/format";
  import Icon from "./Icon.svelte";

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

  function key(e: KeyboardEvent, i: number) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = files[i + (e.key === "ArrowDown" ? 1 : -1)];
      if (next) {
        onselect(next.path, e);
        (e.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLElement>(".file")[i + (e.key === "ArrowDown" ? 1 : -1)]?.focus();
      }
    } else if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      ontoggle(selected.has(files[i].path) ? [...selected] : [files[i].path]);
    }
  }
</script>

<div class="files"
  use:dropTarget={{ accepts: (p) => p.kind === "files" && p.from !== area, drop: (p) => p.kind === "files" && ondropfiles(p.paths) }}>
  {#each files as f, i (f.path)}
    <div class="file" class:sel={selected.has(f.path)} role="option" aria-selected={selected.has(f.path)} tabindex="0"
      title={f.oldPath ? `${f.oldPath} → ${f.path}` : f.path}
      onclick={(e) => onselect(f.path, e)}
      onkeydown={(e) => key(e, i)}
      oncontextmenu={(e) => oncontext(e, f.path)}
      use:draggable={() => dragData(f)}>
      <span class="st {f.status}">{STATUS_GLYPH[f.status]}</span>
      <span class="name"><b>{basename(f.path)}</b> <span class="dir">{dirname(f.path)}</span></span>
      <button class="toggle" title={area === "staged" ? "Unstage" : "Stage"} aria-label={area === "staged" ? `Unstage ${f.path}` : `Stage ${f.path}`}
        onclick={(e) => { e.stopPropagation(); ontoggle(selected.has(f.path) ? [...selected] : [f.path]); }}>
        <Icon name={area === "staged" ? "minus" : "plus"} size={13} />
      </button>
    </div>
  {:else}
    <div class="none faint">{area === "staged" ? "Drag files here, or press + on a change, to stage it" : "Nothing to stage"}</div>
  {/each}
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
    font-size: 10px; font-weight: 700; font-family: var(--font-mono);
    color: var(--ink-2); background: var(--surface-3);
  }
  .st.added, .st.untracked { color: var(--add-ink); background: var(--add-bg-strong); }
  .st.deleted { color: var(--del-ink); background: var(--del-bg-strong); }
  .st.modified { color: var(--lane-1); background: color-mix(in srgb, var(--lane-1) 18%, transparent); }
  .st.renamed { color: var(--lane-3); background: color-mix(in srgb, var(--lane-3) 18%, transparent); }
  .name { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .name b { font-weight: 600; }
  .dir { color: var(--ink-3); font-size: 12px; }
  .toggle {
    display: grid; place-items: center; width: 22px; height: 22px; border-radius: 5px; flex: none;
    color: var(--ink-2); opacity: 0;
  }
  .file:hover .toggle, .file.sel .toggle, .file:focus-within .toggle { opacity: 1; }
  .toggle:hover { background: var(--accent); color: var(--accent-ink); }
  .none { padding: 6px 8px; font-size: 12px; }
</style>
