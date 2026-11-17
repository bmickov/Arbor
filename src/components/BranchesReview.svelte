<script lang="ts">
  // Branches Review: every local branch with whether it's merged into a base
  // branch (including squash and rebase merges), how far it is from it, its
  // upstream, and how long since anyone touched it. Pick and delete in bulk.
  import { api, type Review, type ReviewRow } from "../lib/api";
  import * as act from "../lib/actions";
  import { fullDate, relTime } from "../lib/format";
  import { app } from "../lib/state.svelte";
  import { toasts } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";
  import { SvelteSet } from "svelte/reactivity";

  type Filter = "all" | "merged" | "stale" | "gone" | "unmerged";
  let review = $state<Review | null>(null);
  let base = $state<string | null>(null);
  let filter = $state<Filter>("all");
  let staleDays = $state(90);
  let loading = $state(false);
  const picked = new SvelteSet<string>();

  async function load() {
    loading = true;
    try {
      review = await api.branchesReview(app.path, base);
      base = review.base;
    } catch (e) {
      toasts.push("error", String(e));
    } finally {
      loading = false;
    }
  }

  // Reload when the repository changes (commits, merges, deletions).
  $effect(() => {
    void app.version;
    void app.path;
    load();
  });

  const now = Date.now() / 1000;
  const isStale = (r: ReviewRow) => now - r.time > staleDays * 86400;
  const deletable = (r: ReviewRow) => !r.isHead && !r.isBase;

  const rows = $derived(
    (review?.rows ?? []).filter((r) =>
      filter === "all" ? true
      : filter === "merged" ? r.merged !== "unmerged" && !r.isBase
      : filter === "stale" ? isStale(r)
      : filter === "gone" ? r.upstreamGone
      : r.merged === "unmerged",
    ),
  );
  const counts = $derived.by(() => {
    const all = review?.rows ?? [];
    return {
      all: all.length,
      merged: all.filter((r) => r.merged !== "unmerged" && !r.isBase).length,
      stale: all.filter(isStale).length,
      gone: all.filter((r) => r.upstreamGone).length,
      unmerged: all.filter((r) => r.merged === "unmerged").length,
    };
  });

  // Forget picks that are no longer shown.
  $effect(() => {
    const shown = new Set(rows.map((r) => r.name));
    for (const n of [...picked]) if (!shown.has(n)) picked.delete(n);
  });

  const allPicked = $derived(rows.filter(deletable).length > 0 && rows.filter(deletable).every((r) => picked.has(r.name)));
  function toggleAll() {
    if (allPicked) picked.clear();
    else for (const r of rows) if (deletable(r)) picked.add(r.name);
  }

  async function deletePicked() {
    if ((await act.deleteBranches([...picked])).length) {
      picked.clear();
      load();
    }
  }

  const baseChoices = $derived([
    ...(app.refs?.local ?? []).map((b) => b.name),
    ...(app.refs?.remotes ?? []).flatMap((r) => r.branches.map((b) => b.fullName)),
  ]);

  const FILTERS: { id: Filter; label: string }[] = [
    { id: "all", label: "All" },
    { id: "merged", label: "Merged" },
    { id: "unmerged", label: "Not merged" },
    { id: "stale", label: "Stale" },
    { id: "gone", label: "Upstream gone" },
  ];
</script>

<div class="review">
  <header>
    <div class="title">
      <h2>Branches Review</h2>
      <label class="base">
        compared with
        <select value={base ?? ""} onchange={(e) => { base = (e.currentTarget as HTMLSelectElement).value; load(); }} aria-label="Base branch">
          {#each baseChoices as b}<option value={b}>{b}</option>{/each}
        </select>
      </label>
      {#if loading}<span class="spinner" aria-label="Loading"></span>{/if}
    </div>
    <div class="tools">
      <div class="seg" role="radiogroup" aria-label="Show">
        {#each FILTERS as f}
          <button role="radio" aria-checked={filter === f.id} class:on={filter === f.id} onclick={() => (filter = f.id)}>
            {f.label} <span class="n">{counts[f.id]}</span>
          </button>
        {/each}
      </div>
      <label class="stale">
        Stale after
        <select bind:value={staleDays} aria-label="Stale after">
          {#each [30, 60, 90, 180, 365] as d}<option value={d}>{d} days</option>{/each}
        </select>
      </label>
    </div>
  </header>

  <div class="table" role="table" aria-label="Branches">
    <div class="row head" role="row">
      <span role="columnheader"><input type="checkbox" checked={allPicked} onchange={toggleAll} aria-label="Select all" /></span>
      <span role="columnheader">Branch</span>
      <span role="columnheader">Merged into {review?.base ?? "base"}</span>
      <span role="columnheader" title="Commits only on this branch / only on the base">vs {review?.base ?? "base"}</span>
      <span role="columnheader">Upstream</span>
      <span role="columnheader">Last commit</span>
    </div>
    {#each rows as r (r.name)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="row" role="row" tabindex="-1" class:picked={picked.has(r.name)} class:stale={isStale(r)}
        oncontextmenu={(e) => act.branchMenu(e, r.name)} ondblclick={() => act.showHistory(r.name)}>
        <span role="cell">
          {#if deletable(r)}
            <input type="checkbox" checked={picked.has(r.name)} aria-label="Select {r.name}"
              onchange={() => (picked.has(r.name) ? picked.delete(r.name) : picked.add(r.name))} />
          {/if}
        </span>
        <span role="cell" class="name">
          <Icon name="branch" size={13} />
          <button class="link" onclick={() => act.showHistory(r.name)} title="Show history">{r.name}</button>
          {#if r.isHead}<span class="tag cur">current</span>{/if}
          {#if r.isBase}<span class="tag">base</span>{/if}
        </span>
        <span role="cell">
          {#if r.isBase}<span class="faint">—</span>
          {:else if r.merged === "merged"}<span class="ok"><Icon name="check" size={13} /> Merged</span>
          {:else if r.merged === "squashed"}<span class="ok" title="Its commits aren't in the base, but all its changes are (squash or rebase merge)"><Icon name="check" size={13} /> Changes merged</span>
          {:else}<span class="no">{r.ahead} commit{r.ahead === 1 ? "" : "s"} not merged</span>{/if}
        </span>
        <span role="cell" class="ab">
          {#if !r.isBase}<span title="Only on {r.name}">↑{r.ahead}</span> <span title="Only on {review?.base}">↓{r.behind}</span>{/if}
        </span>
        <span role="cell" class="up">
          {#if r.upstreamGone}<span class="gone" title="{r.upstream} was deleted on the remote">gone</span>
          {:else if r.upstream}<span class="faint">{r.upstream}</span>{#if r.toPush} <span title="To push">↑{r.toPush}</span>{/if}{#if r.toPull} <span title="To pull">↓{r.toPull}</span>{/if}
          {:else}<span class="faint">not pushed</span>{/if}
        </span>
        <span role="cell" class="last" title="{r.summary}\n{r.author}, {fullDate(r.time)}">
          <span class="when" class:old={isStale(r)}>{relTime(r.time)}</span>
          <span class="faint">{r.author}</span>
        </span>
      </div>
    {:else}
      <div class="none faint">{!review ? "" : review.rows.length ? "No branches match." : "No branches yet. Make a first commit to start one."}</div>
    {/each}
  </div>

  {#if picked.size}
    <div class="bar">
      <span>{picked.size} selected</span>
      <button class="btn small danger" onclick={deletePicked}><Icon name="discard" size={13} /> Delete…</button>
      <button class="btn small ghost" onclick={() => picked.clear()}>Clear</button>
    </div>
  {:else if counts.merged > 1}
    <div class="bar hint">
      <span class="faint">Tip: show <b>Merged</b>, select all, and delete branches you no longer need. Undo brings them back.</span>
    </div>
  {/if}
</div>

<style>
  .review { height: 100%; display: flex; flex-direction: column; min-height: 0; background: var(--surface); }
  header { padding: 14px 18px 10px; border-bottom: 1px solid var(--line); display: flex; flex-direction: column; gap: 10px; }
  .title { display: flex; align-items: center; gap: 12px; }
  h2 { margin: 0; font-size: calc(var(--fs) * 17); }
  .base, .stale { display: flex; align-items: center; gap: 6px; color: var(--ink-3); font-size: calc(var(--fs) * 12); }
  .base select, .stale select { width: auto; padding: 3px 6px; }
  .tools { display: flex; align-items: center; gap: 14px; flex-wrap: wrap; }
  .seg { display: flex; background: var(--surface-3); border-radius: 7px; padding: 2px; }
  .seg button { padding: 3px 10px; border-radius: 5px; font-weight: 600; color: var(--ink-2); font-size: calc(var(--fs) * 12); }
  .seg button.on { background: var(--surface); color: var(--ink); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08); }
  .seg .n { color: var(--ink-3); font-weight: 500; margin-left: 2px; }
  .table { flex: 1; overflow: auto; padding: 0 10px 10px; }
  .row {
    display: grid; grid-template-columns: 28px minmax(180px, 1.6fr) minmax(150px, 1fr) 90px minmax(140px, 1fr) minmax(150px, 1fr);
    gap: 10px; align-items: center; padding: 6px 8px; border-radius: 6px; font-size: calc(var(--fs) * 12.5);
  }
  .row:not(.head):hover { background: var(--surface-2); }
  .row.picked { background: var(--sel); }
  .row.head { position: sticky; top: 0; z-index: 1; background: var(--surface); color: var(--ink-3); font-weight: 700; font-size: calc(var(--fs) * 11.5); border-bottom: 1px solid var(--line-soft); border-radius: 0; padding-top: 10px; }
  .name { display: flex; align-items: center; gap: 6px; min-width: 0; color: var(--ink-3); }
  .link { color: var(--ink); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .link:hover { text-decoration: underline; }
  .tag { font-size: calc(var(--fs) * 10); font-weight: 700; border: 1px solid var(--line); border-radius: 4px; padding: 0 4px; color: var(--ink-3); flex: none; }
  .tag.cur { color: var(--accent); border-color: currentColor; }
  .ok { color: var(--add-ink); display: inline-flex; align-items: center; gap: 4px; font-weight: 600; }
  .no { color: var(--ink-2); }
  .ab { font-variant-numeric: tabular-nums; color: var(--ink-2); }
  .up { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .gone { color: var(--warn); border: 1px solid currentColor; border-radius: 4px; padding: 0 4px; font-size: calc(var(--fs) * 11); font-weight: 600; }
  .last { display: flex; gap: 8px; min-width: 0; white-space: nowrap; }
  .when.old { color: var(--warn); }
  .none { padding: 20px; text-align: center; }
  .bar { display: flex; align-items: center; gap: 8px; padding: 8px 18px; border-top: 1px solid var(--line); font-weight: 600; font-size: calc(var(--fs) * 12.5); }
  .bar.hint { font-weight: 400; }
  .spinner { width: 12px; height: 12px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { rotate: 1turn; } }
</style>
