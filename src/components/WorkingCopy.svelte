<script lang="ts">
  import { api, type FileDiff, type Selection } from "../lib/api";
  import { basename } from "../lib/format";
  import { app } from "../lib/state.svelte";
  import { menu, toasts } from "../lib/ui.svelte";
  import Composer from "./Composer.svelte";
  import DiffView from "./DiffView.svelte";
  import FileList from "./FileList.svelte";
  import Icon from "./Icon.svelte";

  const st = $derived(app.status);
  let selStaged = $state<Set<string>>(new Set());
  let selUnstaged = $state<Set<string>>(new Set());
  let diff = $state<FileDiff | null>(null);
  let loading = $state(false);

  // Keep the selection pointing at something sensible after every refresh.
  $effect(() => {
    const s = st;
    if (!s) return;
    const cur = app.selectedFile;
    const inList = (area: string, p: string) =>
      (area === "staged" ? s.staged : area === "unstaged" ? s.unstaged : s.conflicted).some((f) => f.path === p);
    if (cur && inList(cur.area, cur.path)) return;
    if (cur) {
      for (const area of ["unstaged", "staged", "conflicted"] as const) {
        if (inList(area, cur.path)) {
          app.selectedFile = { path: cur.path, area };
          return;
        }
      }
    }
    const first = s.conflicted[0] ? { path: s.conflicted[0].path, area: "conflicted" as const }
      : s.unstaged[0] ? { path: s.unstaged[0].path, area: "unstaged" as const }
      : s.staged[0] ? { path: s.staged[0].path, area: "staged" as const } : null;
    app.selectedFile = first;
  });

  $effect(() => {
    const sel = app.selectedFile;
    void app.version;
    const settings = app.diffSettings;
    if (!sel) {
      diff = null;
      return;
    }
    if (sel.area === "staged") selStaged = selStaged.has(sel.path) ? selStaged : new Set([sel.path]);
    else if (sel.area === "unstaged") selUnstaged = selUnstaged.has(sel.path) ? selUnstaged : new Set([sel.path]);
    const entry = st?.staged.find((f) => f.path === sel.path);
    loading = true;
    const req = sel.area === "staged"
      ? api.diffStaged(app.path, sel.path, entry?.oldPath ?? null, settings)
      : api.diffUnstaged(app.path, sel.path, settings);
    req.then((d) => {
      if (app.selectedFile?.path === sel.path && app.selectedFile.area === sel.area) diff = d;
    }).catch((e) => toasts.push("error", String(e))).finally(() => (loading = false));
  });

  function select(area: "staged" | "unstaged", path: string, e: MouseEvent | KeyboardEvent) {
    const list = area === "staged" ? st!.staged : st!.unstaged;
    let set = new Set(area === "staged" ? selStaged : selUnstaged);
    if ((e as MouseEvent).ctrlKey || (e as MouseEvent).metaKey) {
      set.has(path) ? set.delete(path) : set.add(path);
    } else if (e.shiftKey && app.selectedFile?.area === area) {
      const a = list.findIndex((f) => f.path === app.selectedFile!.path);
      const b = list.findIndex((f) => f.path === path);
      const [s, t] = a < b ? [a, b] : [b, a];
      set = new Set(list.slice(s, t + 1).map((f) => f.path));
    } else {
      set = new Set([path]);
    }
    if (area === "staged") {
      selStaged = set;
      selUnstaged = new Set();
    } else {
      selUnstaged = set;
      selStaged = new Set();
    }
    app.selectedFile = { path, area };
  }

  const undoAction = { label: "Undo", run: () => app.undo() };

  function withRenames(paths: string[]): string[] {
    const out = new Set(paths);
    for (const f of st?.staged ?? []) if (out.has(f.path) && f.oldPath) out.add(f.oldPath);
    return [...out];
  }

  const stage = (paths: string[]) => app.act("Staging", () => api.stageFiles(app.path, paths));
  const unstage = (paths: string[]) => app.act("Unstaging", () => api.unstageFiles(app.path, withRenames(paths)));
  async function discard(paths: string[]) {
    await app.act("Discarding", () => api.discardFiles(app.path, paths));
    toasts.push("info", `Discarded changes to ${paths.length === 1 ? basename(paths[0]) : `${paths.length} files`}.`, undoAction);
  }

  async function lineAction(a: "stage" | "unstage" | "discard", sel: Selection[]) {
    const f = app.selectedFile;
    if (!f) return;
    const s = app.diffSettings;
    if (a === "stage") await app.act("Staging", () => api.stageLines(app.path, f.path, sel, s));
    if (a === "unstage") {
      const old = st?.staged.find((x) => x.path === f.path)?.oldPath ?? null;
      await app.act("Unstaging", () => api.unstageLines(app.path, f.path, old, sel, s));
    }
    if (a === "discard") {
      await app.act("Discarding", () => api.discardLines(app.path, f.path, sel, s));
      toasts.push("info", "Discarded the selected lines.", undoAction);
    }
  }

  function fileMenu(e: MouseEvent, area: "staged" | "unstaged" | "conflicted", path: string) {
    const set = area === "staged" ? selStaged : area === "unstaged" ? selUnstaged : new Set<string>();
    const paths = set.has(path) ? [...set] : [path];
    if (!set.has(path) && area !== "conflicted") select(area, path, e);
    const one = paths.length === 1;
    const label = one ? basename(path) : `${paths.length} files`;
    const full = `${app.path}/${path}`;
    const untracked = st?.unstaged.find((f) => f.path === path)?.status === "untracked";
    menu.fromEvent(e, [
      area === "staged"
        ? { label: `Unstage ${label}`, action: () => unstage(paths) }
        : { label: `Stage ${label}`, action: () => stage(paths) },
      ...(area === "unstaged" ? [{ label: `Discard changes to ${label}`, danger: true, action: () => discard(paths) }] : []),
      { separator: true },
      { label: "Open file", disabled: !one, action: () => api.openExternal(full) },
      { label: "Show in folder", disabled: !one, action: () => api.openExternal(full.replace(/\/[^/]+$/, "")) },
      { label: "File history & blame", disabled: !one || untracked, action: () => (app.fileHistoryPath = path) },
      { separator: true },
      { label: "Copy path", action: () => navigator.clipboard.writeText(paths.join("\n")) },
    ], label);
  }

  const entry = $derived(
    app.selectedFile
      ? (app.selectedFile.area === "staged" ? st?.staged : app.selectedFile.area === "unstaged" ? st?.unstaged : st?.conflicted)?.find((f) => f.path === app.selectedFile!.path)
      : undefined,
  );
  const merging = $derived(st?.state === "merge" || st?.state === "cherrypick" || st?.state === "revert");
  const stateLabel = $derived(st?.state === "merge" ? "Merge" : st?.state === "cherrypick" ? "Cherry-pick" : st?.state === "revert" ? "Revert" : st?.state === "rebase" ? "Rebase" : "");

  async function abortMerge() {
    await app.act("Aborting", () => api.abortMerge(app.path), { success: `${stateLabel} aborted.` });
  }
</script>

<div class="wc">
  <div class="left">
    {#if st && st.state !== "clean"}
      <div class="banner">
        <Icon name="warning" />
        <div>
          {#if st.state === "rebase"}
            <b>A rebase is in progress</b>
            <span>It was started outside Arbor. Finish it with <span class="mono">git rebase --continue</span> or <span class="mono">--abort</span>.</span>
          {:else}
            <b>{stateLabel} in progress</b>
            <span>{st.conflicted.length ? `Resolve ${st.conflicted.length} conflicted file${st.conflicted.length === 1 ? "" : "s"}, then commit.` : "All conflicts are resolved. Commit to finish."}</span>
          {/if}
        </div>
        {#if merging}<button class="btn small" onclick={abortMerge}>Abort</button>{/if}
      </div>
    {/if}

    <div class="scroll">
      {#if st?.conflicted.length}
        <section>
          <header><h3 class="conf">Conflicts</h3><span class="n">{st.conflicted.length}</span></header>
          <div class="files">
            {#each st.conflicted as f (f.path)}
              <div class="cfile" class:sel={app.selectedFile?.path === f.path && app.selectedFile.area === "conflicted"}
                role="option" aria-selected="false" tabindex="0"
                onclick={() => (app.selectedFile = { path: f.path, area: "conflicted" })}
                onkeydown={(e) => e.key === "Enter" && (app.selectedFile = { path: f.path, area: "conflicted" })}
                oncontextmenu={(e) => fileMenu(e, "conflicted", f.path)}>
                <span class="name" title={f.path}><b>{basename(f.path)}</b></span>
                <button class="btn small" title="Keep the version from your branch" onclick={() => app.act("Resolving", () => api.resolveConflict(app.path, f.path, "ours"))}>Mine</button>
                <button class="btn small" title="Keep the version being merged in" onclick={() => app.act("Resolving", () => api.resolveConflict(app.path, f.path, "theirs"))}>Theirs</button>
                <button class="btn small primary" title="Stage the file as it is now in your editor" onclick={() => stage([f.path])}>Resolved</button>
              </div>
            {/each}
          </div>
        </section>
      {/if}

      <section>
        <header>
          <h3>Unstaged</h3><span class="n">{st?.unstaged.length ?? 0}</span>
          <span class="sp"></span>
          {#if st?.unstaged.length}<button class="link" onclick={() => app.act("Staging", () => api.stageAll(app.path))}>Stage all</button>{/if}
        </header>
        <FileList files={st?.unstaged ?? []} area="unstaged" selected={app.selectedFile?.area === "unstaged" ? selUnstaged : new Set()}
          onselect={(p, e) => select("unstaged", p, e)} ontoggle={stage} oncontext={(e, p) => fileMenu(e, "unstaged", p)} ondropfiles={unstage} />
      </section>

      <section>
        <header>
          <h3>Staged</h3><span class="n">{st?.staged.length ?? 0}</span>
          <span class="sp"></span>
          {#if st?.staged.length}<button class="link" onclick={() => app.act("Unstaging", () => api.unstageAll(app.path))}>Unstage all</button>{/if}
        </header>
        <FileList files={st?.staged ?? []} area="staged" selected={app.selectedFile?.area === "staged" ? selStaged : new Set()}
          onselect={(p, e) => select("staged", p, e)} ontoggle={unstage} oncontext={(e, p) => fileMenu(e, "staged", p)} ondropfiles={stage} />
      </section>
    </div>

    <Composer />
  </div>

  <div class="right">
    {#if app.selectedFile && entry}
      <div class="fhead">
        <div class="ftitle">
          <span class="path" title={entry.path}>
            {#if entry.oldPath}<span class="faint">{entry.oldPath} →</span>{/if}
            <bdi>{entry.path}</bdi>
          </span>
          {#if diff && !diff.binary}
            <span class="stats"><span class="plus">+{diff.additions}</span> <span class="minus">−{diff.deletions}</span></span>
          {/if}
          {#if loading}<span class="faint">…</span>{/if}
        </div>
        <div class="factions">
          {#if app.selectedFile.area === "unstaged"}
            <button class="btn small" onclick={() => discard([entry.path])}><Icon name="discard" size={13} /> Discard file</button>
            <button class="btn small primary" onclick={() => stage([entry.path])}><Icon name="plus" size={13} /> Stage file</button>
          {:else if app.selectedFile.area === "staged"}
            <button class="btn small" onclick={() => unstage([entry.path])}><Icon name="minus" size={13} /> Unstage file</button>
          {:else}
            <button class="btn small" onclick={() => api.openExternal(`${app.path}/${entry.path}`)}>Open in editor</button>
          {/if}
        </div>
      </div>
      {#if app.selectedFile.area === "conflicted"}
        <div class="empty"><div><strong>This file has conflicts</strong>Edit it to remove the conflict markers, then choose “Resolved”. Or keep one side entirely with “Mine” or “Theirs”.</div></div>
      {:else}
        <div class="dv">
          <DiffView {diff} mode={app.selectedFile.area} onaction={lineAction}
            image={{ old: app.selectedFile.area === "staged" || st?.head ? "HEAD" : null, new: null }} />
        </div>
      {/if}
    {:else}
      <div class="empty">
        <div>
          <strong>{st?.staged.length || st?.unstaged.length ? "Select a file" : "Working copy is clean"}</strong>
          {st?.staged.length || st?.unstaged.length ? "Its changes appear here. Select lines to stage just those." : "Changes you make to files appear here."}
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .wc { display: grid; grid-template-columns: minmax(280px, 340px) 1fr; height: 100%; min-height: 0; }
  .left { display: flex; flex-direction: column; min-height: 0; background: var(--surface); border-right: 1px solid var(--line); }
  .scroll { flex: 1; overflow-y: auto; padding-bottom: 8px; }
  .banner {
    display: flex; gap: 10px; align-items: flex-start; padding: 10px 12px; margin: 8px;
    border-radius: 8px; background: var(--warn-soft); color: var(--ink);
  }
  .banner :global(svg) { color: var(--warn); flex: none; margin-top: 1px; }
  .banner div { flex: 1; display: flex; flex-direction: column; gap: 2px; font-size: 12px; }
  section header { display: flex; align-items: baseline; gap: 6px; padding: 12px 14px 4px; }
  h3 { margin: 0; font-size: 12px; font-weight: 700; color: var(--ink-2); }
  h3.conf { color: var(--danger); }
  .n { font-size: 12px; color: var(--ink-3); }
  .sp { flex: 1; }
  .link { font-size: 12px; color: var(--accent); font-weight: 600; }
  .link:hover { text-decoration: underline; }
  .files { padding: 2px 6px; }
  .cfile { display: flex; align-items: center; gap: 4px; padding: 3px 4px 3px 8px; border-radius: 5px; }
  .cfile.sel { background: var(--sel); }
  .cfile .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .right { display: flex; flex-direction: column; min-width: 0; min-height: 0; background: var(--surface); }
  .fhead {
    display: flex; align-items: center; gap: 10px; padding: 8px 10px 8px 14px; border-bottom: 1px solid var(--line);
    min-height: 44px;
  }
  .ftitle { flex: 1; min-width: 0; display: flex; align-items: baseline; gap: 10px; }
  .path { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; direction: rtl; text-align: left; }
  .stats { font-family: var(--font-mono); font-size: 11px; white-space: nowrap; }
  .plus { color: var(--add-ink); }
  .minus { color: var(--del-ink); }
  .factions { display: flex; gap: 6px; }
  .dv { flex: 1; min-height: 0; }
</style>
