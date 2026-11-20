<script lang="ts">
  import { nearEnd, Progressive } from "../lib/progressive.svelte";
  // Compare two branches (or tags, or remote branches): the commits each has
  // that the other doesn't, and the files the target would bring to the base.
  import { api, type Comparison, type FileDiff, type MergePrediction } from "../lib/api";
  import * as act from "../lib/actions";
  import { basename, dirname, relTime, STATUS_GLYPH } from "../lib/format";
  import { app } from "../lib/state.svelte";
  import CommitDetail from "./CommitDetail.svelte";
  import DiffView from "./DiffView.svelte";
  import Icon from "./Icon.svelte";
  import Splitter from "./Splitter.svelte";

  let cmp = $state<Comparison | null>(null);
  let error = $state<string | null>(null);
  let tab = $state<"commits" | "files">("commits");
  let pick = $state<{ kind: "commit"; oid: string } | { kind: "file"; path: string } | null>(null);
  let diff = $state.raw<FileDiff | null>(null);
  let diffKey = $state("");

  const base = $derived(app.compare?.base ?? "");
  const target = $derived(app.compare?.target ?? "");

  // Would merging target into base conflict? Worked out without touching anything.
  let prediction = $state.raw<MergePrediction | null>(null);
  $effect(() => {
    const b = base, t = target;
    void app.refsVersion;
    prediction = null;
    if (!b || !t || b === t) return;
    api.predictMerge(app.path, t, b).then((p) => {
      if (base === b && target === t) prediction = p;
    }).catch(() => {});
  });

  $effect(() => {
    const b = base, t = target;
    void app.refsVersion;
    cmp = null;
    error = null;
    if (!b || !t) return;
    api.compareBranches(app.path, b, t)
      .then((c) => {
        if (app.compare?.base !== b || app.compare?.target !== t) return;
        cmp = c;
        if (!pick || (pick.kind === "commit" && !c.ahead.concat(c.behind).some((x) => x.oid === (pick as { oid: string }).oid))) {
          pick = c.ahead[0] ? { kind: "commit", oid: c.ahead[0].oid } : c.files[0] ? { kind: "file", path: c.files[0].path } : null;
          if (!c.ahead.length && c.files.length) tab = "files";
        }
      })
      .catch((e) => (error = String(e)));
  });

  // Load the selected file's diff.
  $effect(() => {
    const p = pick;
    if (p?.kind !== "file" || !base || !target) return;
    const key = `${app.path}|cmp:${base}..${target}|${p.path}`;
    api.compareFileDiff(app.path, base, target, p.path, app.diffSettingsFor(key))
      .then((d) => { diff = d; diffKey = key; })
      .catch(() => { diff = null; diffKey = key; });
  });

  function set(which: "base" | "target", v: string) {
    app.compare = { base: which === "base" ? v : base, target: which === "target" ? v : target };
    pick = null;
  }
  function swap() {
    app.compare = { base: target, target: base };
    pick = null;
  }

  const refs = $derived({
    local: (app.refs?.local ?? []).map((b) => b.name),
    remote: (app.refs?.remotes ?? []).flatMap((r) => r.branches.map((b) => b.fullName)),
    tags: (app.refs?.tags ?? []).map((t) => t.name),
  });
  const canMerge = $derived(!!cmp && base === app.headBranch && cmp.ahead.length > 0);
  const filesProg = new Progressive(300);
  $effect(() => {
    void cmp;
    filesProg.reset();
  });
</script>

{#snippet refSelect(value: string, which: "base" | "target", label: string)}
  <select {value} aria-label={label} onchange={(e) => set(which, (e.currentTarget as HTMLSelectElement).value)}>
    {#if !value}<option value="" disabled>Choose a branch…</option>{/if}
    <optgroup label="Branches">{#each refs.local as r}<option value={r}>{r}</option>{/each}</optgroup>
    {#if refs.remote.length}<optgroup label="Remote branches">{#each refs.remote as r}<option value={r}>{r}</option>{/each}</optgroup>{/if}
    {#if refs.tags.length}<optgroup label="Tags">{#each refs.tags as r}<option value={r}>{r}</option>{/each}</optgroup>{/if}
  </select>
{/snippet}

<div class="compare">
  <header>
    <div class="pick">
      <span class="lbl">Compare</span>
      {@render refSelect(target, "target", "Branch to compare")}
      <span class="lbl">with</span>
      {@render refSelect(base, "base", "Base branch")}
      <button class="btn small ghost" onclick={swap} title="Swap" aria-label="Swap the two branches"><Icon name="sync" size={14} /></button>
    </div>
    {#if cmp}
      <p class="summary">
        <b>{target}</b> has {cmp.aheadTotal} commit{cmp.aheadTotal === 1 ? "" : "s"} that <b>{base}</b> doesn't{cmp.behindTotal ? `, and is missing ${cmp.behindTotal}` : ""}.
        {cmp.files.length} file{cmp.files.length === 1 ? "" : "s"} would change.
        {#if canMerge}<button class="btn small primary" onclick={() => act.mergeInto(target, base)}><Icon name="merge" size={13} /> Merge {target} into {base}</button>{/if}
      </p>
      {#if prediction && (prediction.result === "clean" || prediction.result === "conflicts" || prediction.result === "fast-forward")}
        <p class="predict {prediction.result}" title={prediction.conflicts.join("\n")}>
          {#if prediction.result === "conflicts"}
            <Icon name="warning" size={13} /> Merging would conflict in {prediction.conflicts.length} file{prediction.conflicts.length === 1 ? "" : "s"}:
            <span class="mono">{prediction.conflicts.slice(0, 4).join(", ")}{prediction.conflicts.length > 4 ? ", …" : ""}</span>
          {:else if prediction.result === "fast-forward"}
            <Icon name="check" size={13} /> {base} can simply move forward to {target}: no merge commit, no conflicts.
          {:else}
            <Icon name="check" size={13} /> Merges cleanly{prediction.bothChanged ? ` (both changed ${prediction.bothChanged} file${prediction.bothChanged === 1 ? "" : "s"}, in different places)` : ""}.
          {/if}
        </p>
      {/if}
    {:else if error}
      <p class="summary err">{error.replace(/^\w+:/, "")}</p>
    {:else if !target}
      <p class="summary faint">Choose a branch to compare with {base}.</p>
    {/if}
  </header>

  <div class="body" style:--side-w="{app.size('compare.list', 340)}px">
    <div class="side">
      <div class="tabs" role="tablist">
        <button role="tab" aria-selected={tab === "commits"} class:on={tab === "commits"} onclick={() => (tab = "commits")}>Commits <span class="n">{cmp?.aheadTotal ?? ""}</span></button>
        <button role="tab" aria-selected={tab === "files"} class:on={tab === "files"} onclick={() => (tab = "files")}>Files <span class="n">{cmp?.files.length ?? ""}</span></button>
      </div>
      <div class="list">
        {#if cmp && tab === "commits"}
          <div class="group">Only on {target}</div>
          {#each cmp.ahead as c (c.oid)}
            <button class="item" class:sel={pick?.kind === "commit" && pick.oid === c.oid} onclick={() => (pick = { kind: "commit", oid: c.oid })}>
              <span class="sum">{c.summary}</span><span class="meta faint">{c.short} · {c.author} · {relTime(c.time)}</span>
            </button>
          {:else}
            <div class="none faint">Nothing: {base} already has all of {target}'s commits.</div>
          {/each}
          {#if cmp.aheadTotal > cmp.ahead.length}<div class="none faint">…and {cmp.aheadTotal - cmp.ahead.length} more</div>{/if}
          {#if cmp.behind.length}
            <div class="group">Only on {base}</div>
            {#each cmp.behind as c (c.oid)}
              <button class="item other" class:sel={pick?.kind === "commit" && pick.oid === c.oid} onclick={() => (pick = { kind: "commit", oid: c.oid })}>
                <span class="sum">{c.summary}</span><span class="meta faint">{c.short} · {c.author} · {relTime(c.time)}</span>
              </button>
            {/each}
            {#if cmp.behindTotal > cmp.behind.length}<div class="none faint">…and {cmp.behindTotal - cmp.behind.length} more</div>{/if}
          {/if}
        {:else if cmp}
          {#each cmp.files.length > filesProg.limit ? cmp.files.slice(0, filesProg.limit) : cmp.files as f (f.path)}
            <button class="item file" class:sel={pick?.kind === "file" && pick.path === f.path} onclick={() => (pick = { kind: "file", path: f.path })}>
              <span class="st st-{f.status}">{STATUS_GLYPH[f.status] ?? "M"}</span>
              <span class="fname">{basename(f.path)} <span class="faint">{dirname(f.path)}</span></span>
              <span class="mono counts"><span class="add">+{f.additions}</span> <span class="del">−{f.deletions}</span></span>
            </button>
          {:else}
            <div class="none faint">No file changes.</div>
          {/each}
          {#if cmp.files.length > filesProg.limit}<div class="none faint" use:nearEnd={filesProg.more}>{cmp.files.length - filesProg.limit} more…</div>{/if}
        {/if}
      </div>
    </div>
    <Splitter id="compare.list" min={200} keep={320} />
    <div class="detail">
      {#if pick?.kind === "commit"}
        {#key pick.oid}<CommitDetail oid={pick.oid} />{/key}
      {:else if pick?.kind === "file"}
        <DiffView {diff} loading={diffKey !== `${app.path}|cmp:${base}..${target}|${pick.path}`} mode="readonly" split={app.settings.splitDiff}
          onshowall={() => pick?.kind === "file" && app.largeDiffs.add(`${app.path}|cmp:${base}..${target}|${pick.path}`)} />
      {/if}
    </div>
  </div>
</div>

<style>
  .compare { height: 100%; display: flex; flex-direction: column; min-height: 0; background: var(--surface); }
  header { padding: 12px 16px; border-bottom: 1px solid var(--line); display: flex; flex-direction: column; gap: 8px; }
  .pick { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .pick select { width: auto; max-width: 280px; padding: 4px 8px; font-weight: 600; }
  .lbl { color: var(--ink-3); }
  .summary { margin: 0; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; color: var(--ink-2); }
  .summary.err { color: var(--danger); }
  .body { flex: 1; min-height: 0; display: grid; grid-template-columns: var(--side-w) auto minmax(0, 1fr); }
  .side { display: flex; flex-direction: column; min-height: 0; min-width: 0; }
  .tabs { display: flex; gap: 2px; padding: 8px 8px 4px; }
  .tabs button { padding: 4px 10px; border-radius: 6px; font-weight: 600; color: var(--ink-2); }
  .tabs button.on { background: var(--sel); color: var(--ink); }
  .n { color: var(--ink-3); font-weight: 500; }
  .list { flex: 1; overflow-y: auto; padding: 0 6px 10px; }
  .group { padding: 10px 10px 4px; font-size: calc(var(--fs) * 11); font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: 0.04em; }
  .item { display: flex; flex-direction: column; gap: 1px; width: 100%; padding: 6px 10px; border-radius: 6px; text-align: left; }
  .item:hover { background: var(--surface-2); }
  .item.sel { background: var(--sel); }
  .item.other .sum { color: var(--ink-2); font-weight: 500; }
  .item.file { flex-direction: row; align-items: center; gap: 8px; }
  .sum { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { font-size: calc(var(--fs) * 11.5); }
  .fname { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .counts { font-size: calc(var(--cs) * 11); flex: none; }
  .add { color: var(--add-ink); }
  .del { color: var(--del-ink); }
  .st { width: 16px; text-align: center; font-family: var(--font-mono); font-size: calc(var(--cs) * 11); font-weight: 700; color: var(--ink-3); }
  .st-added { color: var(--add-ink); }
  .st-deleted { color: var(--del-ink); }
  .none { padding: 8px 10px; font-size: calc(var(--fs) * 12); }
  .detail { min-width: 0; min-height: 0; }
  .predict { margin: 4px 0 0; display: flex; align-items: center; gap: 6px; font-size: calc(var(--fs) * 12.5); color: var(--add-ink); flex-wrap: wrap; }
  .predict.conflicts { color: var(--warn); }
  .predict .mono { color: var(--ink-2); font-size: calc(var(--cs) * 12); }
</style>
