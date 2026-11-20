<script lang="ts" module>
  import type { Verification as V } from "../lib/api";
  const verifications = new Map<string, V | null>();
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import * as act from "../lib/actions";
  import { api, type CommitInfo, type FileDiff, type Verification } from "../lib/api";
  import { authorHue, basename, dirname, fullDate, initials, STATUS_GLYPH } from "../lib/format";
  import { app } from "../lib/state.svelte";
  import { menu, toasts } from "../lib/ui.svelte";
  import DiffView from "./DiffView.svelte";
  import Icon from "./Icon.svelte";
  import Splitter from "./Splitter.svelte";
  import Linked from "./Linked.svelte";
  import { treeRows } from "../lib/filetree";
  import { SvelteSet } from "svelte/reactivity";
  import { nearEnd, Progressive } from "../lib/progressive.svelte";

  let { oid, header }: { oid: string; header?: Snippet } = $props();

  let info = $state<CommitInfo | null>(null);
  let file = $state<string | null>(null);
  let diff = $state.raw<FileDiff | null>(null);
  // Which file the shown diff belongs to, so a slow (huge) diff shows as loading.
  let diffKey = $state("");
  let error = $state<string | null>(null);
  // The commit's signature, checked once per commit (it can't change).
  let verified = $state<{ oid: string; v: Verification | null } | null>(null);
  $effect(() => {
    const id = oid;
    const repo = app.path;
    const hit = verifications.get(`${repo}|${id}`);
    if (hit !== undefined) return void (verified = { oid: id, v: hit });
    verified = null;
    // Checking a signature runs gpg or ssh-keygen: not for every commit
    // passed on the way with the arrow keys.
    const t = setTimeout(() => {
      api.verifyCommit(repo, id).then((v) => {
        verifications.set(`${repo}|${id}`, v);
        if (oid === id) verified = { oid: id, v };
      }).catch(() => {});
    }, 150);
    return () => clearTimeout(t);
  });
  const sigLabel = (v: Verification) =>
    v.status === "good" ? "Verified" : v.status === "bad" ? "Bad signature" : v.status === "error" ? "Signed" : "Signed";
  const sigTitle = (v: Verification) =>
    [`${v.format === "ssh" ? "SSH" : v.format === "x509" ? "X.509" : "GPG"} signature`, v.signer && `by ${v.signer}`, v.key && `key ${v.key}`].filter(Boolean).join(" ") + `\n${v.detail}`;

  // A commit never changes, only the branch and tag labels on it do: load
  // it again when the refs change, not on every refresh (editing a file
  // refreshes too, and a huge commit takes a while to read).
  const refsKey = $derived(app.refs ? JSON.stringify(app.refs) : "");
  let shownOid = "";
  $effect(() => {
    const id = oid;
    const repo = app.path;
    void refsKey;
    const load = () =>
      api.commitInfo(repo, id).then((i) => {
        if (oid !== id) return;
        info = i;
        shownOid = id;
        error = null;
        if (!file || !i.files.some((f) => f.path === file)) file = i.files[0]?.path ?? null;
      }).catch((e) => oid === id && (error = String(e)));
    // Moving through the list with the arrow keys: load only where it stops.
    if (!shownOid || shownOid === id) return void load();
    const t = setTimeout(load, 60);
    return () => clearTimeout(t);
  });

  $effect(() => {
    const f = file;
    const id = oid;
    if (!f) {
      diff = null;
      return;
    }
    const s = app.diffSettingsFor(`${app.path}|${id}|${f}`);
    api.commitFileDiff(app.path, id, f, s).then((d) => {
      if (file === f && oid === id) {
        diff = d;
        diffKey = `${id}:${f}`;
      }
    }).catch((e) => {
      if (file === f && oid === id) diffKey = `${id}:${f}`;
      toasts.push("error", String(e));
    });
  });

  function fileMenu(e: MouseEvent, path: string) {
    const f = info?.files.find((x) => x.path === path);
    const short = info?.short ?? oid.slice(0, 7);
    // Stashes are commits too, but "restore" there means applying the stash.
    const isStash = !!info && info.parents.length === 3;
    const link = isStash ? null : act.hostLink(oid, f?.status === "deleted" ? undefined : path);
    menu.fromEvent(e, [
      { label: "File history & blame", action: () => (app.fileHistoryPath = path) },
      { label: "Open current version", action: () => api.openExternal(`${app.path}/${path}`) },
      { label: "Open in diff tool", action: () => act.openDiffTool(path, oid) },
      { separator: true },
      { label: `Restore the version from ${short}`, hint: "into your working copy", disabled: isStash || f?.status === "deleted", action: () => act.restoreFile(oid, path, `as of ${short}`) },
      { label: "Restore the version before this commit", disabled: isStash || !info?.parents.length || f?.status === "added", action: () => act.restoreFile(`${oid}^`, f?.oldPath ?? path, `as it was before ${short}`) },
      { separator: true },
      { label: "Copy path", action: () => navigator.clipboard.writeText(path) },
      ...(link && f?.status !== "deleted" ? [{ label: link.label, hint: link.pushed ? undefined : "not pushed yet", disabled: !link.pushed, action: link.open }] : []),
    ], basename(path));
  }

  const closedDirs = new SvelteSet<string>();
  const fileRows = $derived(
    !info ? [] : app.settings.fileTree ? treeRows(info.files, (f) => f.path, closedDirs) : info.files.map((f) => ({ kind: "file" as const, item: f, name: basename(f.path), depth: 0 })),
  );
  // A commit can touch tens of thousands of files: show them a slice at a time.
  const prog = new Progressive(300);
  $effect(() => {
    void oid;
    prog.reset();
  });
  const totals = $derived(info ? info.files.reduce((a, f) => [a[0] + f.additions, a[1] + f.deletions], [0, 0]) : [0, 0]);
  const sameCommitter = $derived(info && info.author.email === info.committer.email && info.author.name === info.committer.name);
</script>

<div class="cd">
  {#if error}
    <div class="empty"><div><strong>Couldn't load this commit</strong>{error}</div></div>
  {:else if info}
    {@const topH = app.layout.sizes["commit.top"]}
    <div class="top" class:sized={topH != null} style:height={topH != null ? `${topH}px` : null}>
    <div class="head">
      {#if header}{@render header()}{/if}
      <h2 class="selectable"><Linked text={info.summary} /></h2>
      {#if info.body}<pre class="body selectable"><Linked text={info.body} /></pre>{/if}
      <div class="meta">
        <span class="avatar" style:--h={authorHue(info.author.email)}>{initials(info.author.name)}</span>
        <div class="who">
          <span><b>{info.author.name}</b> <span class="faint selectable">{info.author.email}</span></span>
          <span class="faint" title="Authored">{fullDate(info.author.time)}{#if !sameCommitter} · committed by {info.committer.name}{/if}</span>
        </div>
        {#if verified?.oid === info.oid && verified.v}
          {@const v = verified.v}
          <span class="sig {v.status}" title={sigTitle(v)}><Icon name={v.status === "bad" ? "x" : v.status === "good" ? "check" : "commit"} size={12} /> {sigLabel(v)}</span>
        {/if}
        <button class="sha mono" title="Copy commit ID" onclick={() => navigator.clipboard.writeText(info!.oid).then(() => toasts.push("info", "Copied commit ID."))}>{info.short}</button>
      </div>
      {#if info.parents.length}
        <div class="parents faint">
          {info.parents.length > 1 ? "Parents" : "Parent"}
          {#each info.parents as p}
            <button class="mono plink" onclick={() => app.reveal(p)}>{p.slice(0, 7)}</button>
          {/each}
        </div>
      {/if}
    </div>

    <div class="files">
      <div class="fsum faint">
        <span>{info.files.length} file{info.files.length === 1 ? "" : "s"} changed · <span class="plus">+{totals[0]}</span> <span class="minus">−{totals[1]}</span></span>
        <button class="viewtoggle" onclick={() => { app.settings.fileTree = !app.settings.fileTree; app.saveSettings(); }}
          title={app.settings.fileTree ? "Show as a list" : "Show as a folder tree"} aria-label={app.settings.fileTree ? "Show as a list" : "Show as a folder tree"}>
          <Icon name={app.settings.fileTree ? "file" : "folder"} size={12} />
        </button>
      </div>
      {#each fileRows.length > prog.limit ? fileRows.slice(0, prog.limit) : fileRows as row (row.kind === "dir" ? `d:${row.path}` : row.item.path)}
        {#if row.kind === "dir"}
          <button class="file dirrow" style:padding-left="{8 + row.depth * 14}px" onclick={() => (closedDirs.has(row.path) ? closedDirs.delete(row.path) : closedDirs.add(row.path))}>
            <span class="chev" class:open={row.open}><Icon name="chevron" size={11} /></span>
            <Icon name="folder" size={13} />
            <span class="name">{row.name}</span>
            <span class="faint">{row.count}</span>
          </button>
        {:else}
          {@const f = row.item}
          <button class="file" class:sel={file === f.path} style:padding-left={app.settings.fileTree ? `${22 + row.depth * 14}px` : null}
            onclick={() => (file = f.path)} oncontextmenu={(e) => fileMenu(e, f.path)} title={f.oldPath ? `${f.oldPath} → ${f.path}` : f.path}>
            <span class="st {f.status}">{STATUS_GLYPH[f.status]}</span>
            <span class="name"><b>{row.name}</b>{#if !app.settings.fileTree}<span class="dir">{dirname(f.path)}</span>{/if}</span>
            {#if !f.binary}<span class="counts mono"><span class="plus">+{f.additions}</span> <span class="minus">−{f.deletions}</span></span>{/if}
          </button>
        {/if}
      {/each}
      {#if fileRows.length > prog.limit}<div class="more faint" use:nearEnd={prog.more}>{fileRows.length - prog.limit} more…</div>{/if}
    </div>
    </div>
    <Splitter id="commit.top" axis="y" min={70} keep={140} />

    <div class="diff">
      <DiffView {diff} onshowall={() => app.largeDiffs.add(`${app.path}|${oid}|${file}`)} loading={!!file && diffKey !== `${oid}:${file}`} mode="readonly" split={app.settings.splitDiff} image={{ old: info.parents[0] ?? null, new: info.oid }} />
    </div>
  {/if}
</div>

<style>
  .cd { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .head { padding: 14px 16px 10px; border-bottom: 1px solid var(--line-soft); display: flex; flex-direction: column; gap: 8px; }
  h2 { margin: 0; font-size: calc(var(--fs) * 15); font-weight: 700; line-height: 1.35; }
  .body {
    margin: 0; font-family: var(--font-ui); white-space: pre-wrap; color: var(--ink-2);
    max-height: 140px; overflow: auto;
  }
  .meta { display: flex; align-items: center; gap: 10px; }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; font-size: calc(var(--fs) * 12); }
  .who > span { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .avatar {
    width: 30px; height: 30px; border-radius: 50%; flex: none; display: grid; place-items: center;
    font-size: calc(var(--fs) * 11); font-weight: 700; background: hsl(var(--h) var(--av-bg-s) var(--av-bg-l)); color: hsl(var(--h) var(--av-ink-s) var(--av-ink-l));
  }
  .sha { padding: 3px 7px; border-radius: 5px; border: 1px solid var(--line); color: var(--ink-2); }
  .sha:hover { background: var(--surface-2); }
  .sig { display: inline-flex; align-items: center; gap: 3px; padding: 2px 7px; border-radius: 5px; font-size: calc(var(--fs) * 11.5); font-weight: 600; border: 1px solid currentColor; color: var(--ink-3); white-space: nowrap; cursor: default; }
  .sig.good { color: var(--add-ink); background: var(--add-bg); border-color: transparent; }
  .sig.bad { color: var(--danger); background: var(--danger-soft); border-color: transparent; }
  .parents { font-size: calc(var(--fs) * 12); display: flex; gap: 6px; align-items: center; }
  .plink { color: var(--accent); }
  .plink:hover { text-decoration: underline; }

  .top { display: flex; flex-direction: column; flex: none; min-height: 0; max-height: 60%; }
  .top.sized { max-height: none; overflow-y: auto; }
  .head { flex: none; }
  .files { max-height: 34vh; overflow-y: auto; padding: 4px 6px 6px; flex: none; }
  .top.sized .files { max-height: none; flex: 1 0 60px; }
  .more { padding: 6px 8px; font-size: calc(var(--fs) * 12); }
  .fsum { font-size: calc(var(--fs) * 12); padding: 4px 8px 4px; display: flex; align-items: center; justify-content: space-between; }
  .viewtoggle { display: grid; place-items: center; width: 20px; height: 20px; border-radius: 4px; color: var(--ink-3); }
  .viewtoggle:hover { background: var(--surface-3); color: var(--ink); }
  .dirrow { color: var(--ink-2); gap: 6px; }
  .chev { display: inline-grid; transition: rotate 0.1s; color: var(--ink-3); }
  .chev.open { rotate: 90deg; }
  .file { display: flex; align-items: center; gap: 8px; width: 100%; height: 26px; padding: 0 8px; border-radius: 5px; text-align: left; }
  .file:hover { background: var(--surface-2); }
  .file.sel { background: var(--sel); }
  .st {
    width: 16px; height: 16px; border-radius: 4px; flex: none; display: grid; place-items: center;
    font-size: calc(var(--fs) * 10); font-weight: 700; font-family: var(--font-mono); color: var(--ink-2); background: var(--surface-3);
  }
  .st.added, .st.untracked { color: var(--add-ink); background: var(--add-bg-strong); }
  .st.deleted { color: var(--del-ink); background: var(--del-bg-strong); }
  .st.modified { color: var(--lane-1); background: color-mix(in srgb, var(--lane-1) 18%, transparent); }
  .st.renamed { color: var(--lane-3); background: color-mix(in srgb, var(--lane-3) 18%, transparent); }
  .name { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .name b { font-weight: 600; }
  .dir { margin-left: 0.4em; color: var(--ink-3); font-size: calc(var(--fs) * 12); }
  .counts { font-size: calc(var(--cs) * 11); white-space: nowrap; }
  .plus { color: var(--add-ink); }
  .minus { color: var(--del-ink); }
  .diff { flex: 1; min-height: 0; }
</style>
