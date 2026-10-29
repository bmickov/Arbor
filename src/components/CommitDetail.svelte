<script lang="ts">
  import type { Snippet } from "svelte";
  import { api, type CommitInfo, type FileDiff } from "../lib/api";
  import { authorHue, basename, dirname, fullDate, initials, STATUS_GLYPH } from "../lib/format";
  import { app } from "../lib/state.svelte";
  import { menu, toasts } from "../lib/ui.svelte";
  import DiffView from "./DiffView.svelte";

  let { oid, header }: { oid: string; header?: Snippet } = $props();

  let info = $state<CommitInfo | null>(null);
  let file = $state<string | null>(null);
  let diff = $state<FileDiff | null>(null);
  let error = $state<string | null>(null);

  $effect(() => {
    const id = oid;
    void app.version;
    api.commitInfo(app.path, id).then((i) => {
      if (oid !== id) return;
      info = i;
      error = null;
      if (!file || !i.files.some((f) => f.path === file)) file = i.files[0]?.path ?? null;
    }).catch((e) => (error = String(e)));
  });

  $effect(() => {
    const f = file;
    const id = oid;
    const s = app.diffSettings;
    if (!f) {
      diff = null;
      return;
    }
    api.commitFileDiff(app.path, id, f, s).then((d) => {
      if (file === f && oid === id) diff = d;
    }).catch((e) => toasts.push("error", String(e)));
  });

  function fileMenu(e: MouseEvent, path: string) {
    menu.fromEvent(e, [
      { label: "File history & blame", action: () => (app.fileHistoryPath = path) },
      { label: "Open current version", action: () => api.openExternal(`${app.path}/${path}`) },
      { label: "Copy path", action: () => navigator.clipboard.writeText(path) },
    ], basename(path));
  }

  const totals = $derived(info ? info.files.reduce((a, f) => [a[0] + f.additions, a[1] + f.deletions], [0, 0]) : [0, 0]);
  const sameCommitter = $derived(info && info.author.email === info.committer.email && info.author.name === info.committer.name);
</script>

<div class="cd">
  {#if error}
    <div class="empty"><div><strong>Couldn't load this commit</strong>{error}</div></div>
  {:else if info}
    <div class="head">
      {#if header}{@render header()}{/if}
      <h2 class="selectable">{info.summary}</h2>
      {#if info.body}<pre class="body selectable">{info.body}</pre>{/if}
      <div class="meta">
        <span class="avatar" style:--h={authorHue(info.author.email)}>{initials(info.author.name)}</span>
        <div class="who">
          <span><b>{info.author.name}</b> <span class="faint selectable">{info.author.email}</span></span>
          <span class="faint" title="Authored">{fullDate(info.author.time)}{#if !sameCommitter} · committed by {info.committer.name}{/if}</span>
        </div>
        <button class="sha mono" title="Copy commit ID" onclick={() => navigator.clipboard.writeText(info!.oid).then(() => toasts.push("info", "Copied commit ID."))}>{info.short}</button>
      </div>
      {#if info.parents.length}
        <div class="parents faint">
          {info.parents.length > 1 ? "Parents" : "Parent"}
          {#each info.parents as p}
            <button class="mono plink" onclick={() => (app.selectedCommit = p)}>{p.slice(0, 7)}</button>
          {/each}
        </div>
      {/if}
    </div>

    <div class="files">
      <div class="fsum faint">{info.files.length} file{info.files.length === 1 ? "" : "s"} changed · <span class="plus">+{totals[0]}</span> <span class="minus">−{totals[1]}</span></div>
      {#each info.files as f (f.path)}
        <button class="file" class:sel={file === f.path} onclick={() => (file = f.path)} oncontextmenu={(e) => fileMenu(e, f.path)} title={f.oldPath ? `${f.oldPath} → ${f.path}` : f.path}>
          <span class="st {f.status}">{STATUS_GLYPH[f.status]}</span>
          <span class="name"><b>{basename(f.path)}</b> <span class="dir">{dirname(f.path)}</span></span>
          {#if !f.binary}<span class="counts mono"><span class="plus">+{f.additions}</span> <span class="minus">−{f.deletions}</span></span>{/if}
        </button>
      {/each}
    </div>

    <div class="diff">
      <DiffView {diff} mode="readonly" split={app.settings.splitDiff} image={{ old: info.parents[0] ?? null, new: info.oid }} />
    </div>
  {/if}
</div>

<style>
  .cd { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .head { padding: 14px 16px 10px; border-bottom: 1px solid var(--line); display: flex; flex-direction: column; gap: 8px; }
  h2 { margin: 0; font-size: 15px; font-weight: 700; line-height: 1.35; }
  .body {
    margin: 0; font-family: var(--font-ui); white-space: pre-wrap; color: var(--ink-2);
    max-height: 140px; overflow: auto;
  }
  .meta { display: flex; align-items: center; gap: 10px; }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; font-size: 12px; }
  .who > span { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .avatar {
    width: 30px; height: 30px; border-radius: 50%; flex: none; display: grid; place-items: center;
    font-size: 11px; font-weight: 700; background: hsl(var(--h) 45% 88%); color: hsl(var(--h) 45% 28%);
  }
  @media (prefers-color-scheme: dark) { .avatar { background: hsl(var(--h) 30% 26%); color: hsl(var(--h) 55% 82%); } }
  .sha { padding: 3px 7px; border-radius: 5px; border: 1px solid var(--line); color: var(--ink-2); }
  .sha:hover { background: var(--surface-2); }
  .parents { font-size: 12px; display: flex; gap: 6px; align-items: center; }
  .plink { color: var(--accent); }
  .plink:hover { text-decoration: underline; }

  .files { max-height: 34%; overflow-y: auto; padding: 4px 6px 6px; border-bottom: 1px solid var(--line); flex: none; }
  .fsum { font-size: 12px; padding: 4px 8px 4px; }
  .file { display: flex; align-items: center; gap: 8px; width: 100%; height: 26px; padding: 0 8px; border-radius: 5px; text-align: left; }
  .file:hover { background: var(--surface-2); }
  .file.sel { background: var(--sel); }
  .st {
    width: 16px; height: 16px; border-radius: 4px; flex: none; display: grid; place-items: center;
    font-size: 10px; font-weight: 700; font-family: var(--font-mono); color: var(--ink-2); background: var(--surface-3);
  }
  .st.added, .st.untracked { color: var(--add-ink); background: var(--add-bg-strong); }
  .st.deleted { color: var(--del-ink); background: var(--del-bg-strong); }
  .st.modified { color: var(--lane-1); background: color-mix(in srgb, var(--lane-1) 18%, transparent); }
  .st.renamed { color: var(--lane-3); background: color-mix(in srgb, var(--lane-3) 18%, transparent); }
  .name { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .name b { font-weight: 600; }
  .dir { color: var(--ink-3); font-size: 12px; }
  .counts { font-size: 11px; white-space: nowrap; }
  .plus { color: var(--add-ink); }
  .minus { color: var(--del-ink); }
  .diff { flex: 1; min-height: 0; }
</style>
