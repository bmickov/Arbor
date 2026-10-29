<script lang="ts">
  import type { CommitRow, RefLabel } from "../lib/api";
  import * as act from "../lib/actions";
  import { draggable, dropTarget, type Payload } from "../lib/drag.svelte";
  import { authorHue, initials, laneVar, relTime, fullDate } from "../lib/format";
  import { app } from "../lib/state.svelte";
  import { menu } from "../lib/ui.svelte";
  import CommitDetail from "./CommitDetail.svelte";
  import GraphCell from "./GraphCell.svelte";
  import Icon from "./Icon.svelte";
  import RebaseEditor from "./RebaseEditor.svelte";

  const ROW = 32;
  const LANE = 16;
  const MAX_LANES = 14;

  let query = $state("");
  let scroller = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let viewH = $state(600);
  let listW = $state(55); // percent
  let rebaseFrom = $state<CommitRow | null>(null);
  let dropLine = $state<{ oid: string; below: boolean } | null>(null);

  const q = $derived(query.trim().toLowerCase());
  const rows = $derived(
    q ? app.log.filter((r) => r.summary.toLowerCase().includes(q) || r.author.toLowerCase().includes(q) || r.oid.startsWith(q) || r.refs.some((l) => l.name.toLowerCase().includes(q))) : app.log,
  );
  const lanes = $derived(Math.min(MAX_LANES, Math.max(1, ...app.log.slice(0, 4000).map((r) => r.graph.width))));
  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - 15));
  const last = $derived(Math.min(rows.length, Math.ceil((scrollTop + viewH) / ROW) + 15));
  const visible = $derived(rows.slice(first, last));
  const rewritableIds = $derived(new Set(act.rewritable().map((c) => c.oid)));
  const pushed = $derived(act.pushedSet());
  const head = $derived(app.status?.head);

  // Keep the selected commit in view when it changes from elsewhere.
  $effect(() => {
    const sel = app.selectedCommit;
    if (!sel || !scroller) return;
    const i = rows.findIndex((r) => r.oid === sel);
    if (i < 0) return;
    const top = i * ROW;
    if (top < scroller.scrollTop || top + ROW > scroller.scrollTop + viewH) scroller.scrollTop = Math.max(0, top - viewH / 3);
  });

  // Select the head commit when nothing is selected.
  $effect(() => {
    if (!app.selectedCommit && rows.length) app.selectedCommit = rows.find((r) => r.oid === head)?.oid ?? rows[0].oid;
  });

  function key(e: KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const i = rows.findIndex((r) => r.oid === app.selectedCommit);
    const n = rows[Math.max(0, Math.min(rows.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))];
    if (n) app.selectedCommit = n.oid;
  }

  function pillKind(l: RefLabel) {
    return l.kind === "head" ? "head" : l.kind === "remote" ? "remote" : l.kind === "tag" ? "tag" : "";
  }

  function pillDrag(l: RefLabel): { payload: Payload; label: string } | null {
    if (l.kind === "head" || l.kind === "local") return { payload: { kind: "branch", name: l.name }, label: l.name };
    if (l.kind === "remote") return { payload: { kind: "remote-branch", fullName: l.name }, label: l.name };
    return null;
  }

  function pillMenu(e: MouseEvent, l: RefLabel, row: CommitRow) {
    if (l.kind === "head" || l.kind === "local") act.branchMenu(e, l.name);
    else if (l.kind === "remote") act.remoteBranchMenu(e, l.name);
    else act.tagMenu(e, l.name, row.oid);
  }

  // Reordering: dropping commit A on commit B.
  function canReorder(p: Payload, target: CommitRow) {
    return !q && p.kind === "commit" && p.oid !== target.oid && rewritableIds.has(p.oid) && rewritableIds.has(target.oid);
  }

  function dropCommit(p: Payload, target: CommitRow) {
    if (p.kind !== "commit") return;
    const below = dropLine?.oid === target.oid ? dropLine.below : false;
    dropLine = null;
    const list = act.rewritable();
    const ti = list.findIndex((c) => c.oid === target.oid);
    // Rows are newest first: "above" a row means newer than it.
    const after = below ? (ti > 0 ? list[ti - 1].oid : null) : target.oid;
    act.moveCommit(p.oid, after === p.oid ? (ti > 1 ? list[ti - 2].oid : null) : after);
  }

  function scopeMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const set = (s: string) => {
      app.scope = s;
      app.loadLog();
    };
    menu.open(r.left, r.bottom + 4, [
      { label: "All branches", hint: app.scope === "all" ? "current" : undefined, action: () => set("all") },
      { label: "Current branch only", hint: app.scope === "head" ? "current" : undefined, action: () => set("head") },
      { separator: true },
      ...(app.refs?.local ?? []).map((b) => ({ label: b.name, hint: app.scope === b.name ? "current" : undefined, action: () => set(b.name) })),
    ], "Show history of");
  }

  const scopeLabel = $derived(app.scope === "all" ? "All branches" : app.scope === "head" ? "Current branch" : app.scope);

  function startResize(e: PointerEvent) {
    const host = (e.currentTarget as HTMLElement).parentElement!;
    const r = host.getBoundingClientRect();
    const move = (ev: PointerEvent) => (listW = Math.min(80, Math.max(30, ((ev.clientX - r.left) / r.width) * 100)));
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", () => window.removeEventListener("pointermove", move), { once: true });
  }
</script>

<div class="history" style:--list-w="{listW}%">
  <div class="list">
    <div class="tools">
      <button class="scope" onclick={scopeMenu}><Icon name="history" size={14} /> {scopeLabel} <Icon name="down" size={12} /></button>
      <div class="search">
        <Icon name="search" size={14} />
        <input placeholder="Search message, author, ID" bind:value={query} spellcheck="false" />
        {#if query}<button class="clear" onclick={() => (query = "")} aria-label="Clear search"><Icon name="x" size={12} /></button>{/if}
      </div>
    </div>

    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="rows" tabindex="0" role="listbox" aria-label="Commits" bind:this={scroller} bind:clientHeight={viewH}
      onscroll={(e) => (scrollTop = (e.currentTarget as HTMLElement).scrollTop)} onkeydown={key}>
      <div class="spacer" style:height="{rows.length * ROW + (app.logMore ? 56 : 12)}px">
        {#each visible as row, k (row.oid)}
          {@const i = first + k}
          {@const canMove = rewritableIds.has(row.oid)}
          <div class="row" class:sel={app.selectedCommit === row.oid} class:headrow={row.oid === head}
            class:line-above={dropLine?.oid === row.oid && !dropLine.below} class:line-below={dropLine?.oid === row.oid && dropLine.below}
            style:top="{i * ROW}px" role="option" aria-selected={app.selectedCommit === row.oid} tabindex="-1"
            onclick={() => (app.selectedCommit = row.oid)}
            onkeydown={() => {}}
            oncontextmenu={(e) => { app.selectedCommit = row.oid; act.commitMenu(e, row, (c) => (rebaseFrom = c)); }}
            use:draggable={() => ({ payload: { kind: "commit", oid: row.oid, summary: row.summary }, label: `${row.short} ${row.summary}` })}
            use:dropTarget={{
              accepts: (p) => canReorder(p, row),
              drop: (p) => dropCommit(p, row),
              hover: (_p, f) => (dropLine = { oid: row.oid, below: f > 0.5 }),
              leave: () => { if (dropLine?.oid === row.oid) dropLine = null; },
            }}>
            {#if !q}
              <GraphCell g={row.graph} h={ROW} lane={LANE} {lanes} head={row.oid === head} merge={row.parents.length > 1} />
            {:else}
              <span class="nograph"></span>
            {/if}
            <div class="msg">
              {#each row.refs as l (l.kind + l.name)}
                {@const d = pillDrag(l)}
                {#if d}
                  <span class="pill {pillKind(l)}" role="button" tabindex="-1" style:--c={laneVar(row.graph.color)} title={l.kind === "remote" ? `${l.name} (remote)` : l.name}
                    oncontextmenu={(e) => { e.stopPropagation(); pillMenu(e, l, row); }}
                    use:draggable={() => d}
                    use:dropTarget={{ accepts: act.acceptsOnBranch(l.name, l.kind === "head"), drop: (p, e) => act.dropOnBranch(p, l.name, e) }}>
                    {#if l.kind === "remote"}<Icon name="remote" size={11} />{/if}{l.name}
                  </span>
                {:else}
                  <span class="pill tag" role="button" tabindex="-1" title={l.name} oncontextmenu={(e) => { e.stopPropagation(); pillMenu(e, l, row); }}>
                    <Icon name="tag" size={10} />{l.name}
                  </span>
                {/if}
              {/each}
              <span class="summary" class:merge={row.parents.length > 1}>{row.summary}</span>
            </div>
            {#if canMove && !q}<span class="grip" title="Drag to reorder" class:pushed={pushed.has(row.oid)}><Icon name="grip" size={12} /></span>{/if}
            <span class="avatar" style:--h={authorHue(row.email)} title="{row.author} <{row.email}>">{initials(row.author)}</span>
            <span class="author">{row.author}</span>
            <span class="date" title={fullDate(row.time)}>{relTime(row.time)}</span>
          </div>
        {/each}
        {#if app.logMore && !q}
          <button class="btn more" style:top="{rows.length * ROW + 12}px" onclick={() => app.loadMoreLog()}>Load more commits</button>
        {/if}
      </div>
      {#if !rows.length}
        <div class="empty"><div>
          {#if app.logLoading}<strong>Loading history…</strong>
          {:else if q}<strong>No matching commits</strong>Searching the {app.log.length} loaded commits.
          {:else}<strong>No commits yet</strong>Stage some changes and commit them.{/if}
        </div></div>
      {/if}
    </div>
  </div>

  <div class="splitter" role="separator" aria-orientation="vertical" onpointerdown={startResize}></div>

  <div class="detail">
    {#if app.selectedCommit}
      <CommitDetail oid={app.selectedCommit} />
    {:else}
      <div class="empty"><div><strong>Select a commit</strong>Its message and changes appear here.</div></div>
    {/if}
  </div>
</div>

{#if rebaseFrom}
  <RebaseEditor from={rebaseFrom} onclose={() => (rebaseFrom = null)} />
{/if}

<style>
  .history { display: grid; grid-template-columns: var(--list-w) 5px 1fr; height: 100%; min-height: 0; background: var(--surface); }
  .list { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .tools { display: flex; gap: 8px; align-items: center; padding: 8px 10px; border-bottom: 1px solid var(--line); }
  .scope {
    display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 9px; border-radius: var(--radius);
    font-weight: 600; color: var(--ink-2); white-space: nowrap; max-width: 45%;
    overflow: hidden; text-overflow: ellipsis;
  }
  .scope:hover { background: var(--surface-3); color: var(--ink); }
  .search {
    flex: 1; display: flex; align-items: center; gap: 6px; padding: 0 8px; height: 28px;
    border: 1px solid var(--line); border-radius: var(--radius); color: var(--ink-3); background: var(--surface);
  }
  .search input { border: 0; padding: 0; height: 26px; background: transparent; box-shadow: none; }
  .clear { color: var(--ink-3); display: grid; }

  .rows { flex: 1; overflow: auto; position: relative; outline: none; }
  .spacer { position: relative; min-width: 100%; }
  .row {
    position: absolute; left: 0; right: 0; height: 32px; display: flex; align-items: center; gap: 8px;
    padding-right: 12px; white-space: nowrap;
  }
  .row:hover { background: var(--surface-2); }
  .row.sel { background: var(--sel); }
  .row.headrow .summary { font-weight: 700; }
  .row:global(.drop-over) { box-shadow: none !important; background: transparent !important; }
  .row.line-above::before, .row.line-below::after {
    content: ""; position: absolute; left: 8px; right: 8px; height: 3px; border-radius: 2px;
    background: var(--accent); z-index: 2; pointer-events: none;
  }
  .row.line-above::before { top: -1px; }
  .row.line-below::after { bottom: -2px; }
  .nograph { width: 10px; }
  .msg { flex: 1; min-width: 0; display: flex; align-items: center; gap: 5px; overflow: hidden; }
  .summary { overflow: hidden; text-overflow: ellipsis; }
  .summary.merge { color: var(--ink-2); }
  .pill { cursor: grab; flex: none; }
  .grip { color: var(--ink-3); opacity: 0; display: grid; cursor: grab; }
  .grip.pushed { color: var(--warn); }
  .row:hover .grip { opacity: 1; }
  .avatar {
    width: 20px; height: 20px; border-radius: 50%; flex: none; display: grid; place-items: center;
    font-size: 9px; font-weight: 700; letter-spacing: 0.02em;
    background: hsl(var(--h) 45% 88%); color: hsl(var(--h) 45% 28%);
  }
  @media (prefers-color-scheme: dark) { .avatar { background: hsl(var(--h) 30% 26%); color: hsl(var(--h) 55% 82%); } }
  .author { width: 110px; overflow: hidden; text-overflow: ellipsis; color: var(--ink-2); font-size: 12px; }
  .date { width: 78px; text-align: right; color: var(--ink-3); font-size: 12px; font-variant-numeric: tabular-nums; }
  .more { position: absolute; left: 50%; translate: -50% 0; }

  .splitter { cursor: col-resize; background: var(--line); border-inline: 2px solid var(--surface); }
  .splitter:hover { background: var(--accent); }
  .detail { min-width: 0; min-height: 0; display: flex; flex-direction: column; }
</style>
