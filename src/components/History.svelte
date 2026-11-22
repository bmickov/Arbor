<script lang="ts">
  import { api, type CommitRow, type RefLabel, type SearchResult } from "../lib/api";
  import * as act from "../lib/actions";
  import { draggable, dropTarget, type Payload } from "../lib/drag.svelte";
  import { authorHue, initials, laneVar, relTime, fullDate } from "../lib/format";
  import { app, NO_FILTER } from "../lib/state.svelte";
  import { menu } from "../lib/ui.svelte";
  import { untrack } from "svelte";
  import CommitDetail from "./CommitDetail.svelte";
  import GraphCell from "./GraphCell.svelte";
  import Linked from "./Linked.svelte";
  import Icon from "./Icon.svelte";
  import RebaseEditor from "./RebaseEditor.svelte";

  // Row height and lane width follow the interface font size.
  const scale = $derived(app.settings.uiSize / 13);
  const ROW = $derived(Math.round(24 * scale));
  const LANE = $derived(Math.max(9, Math.round(11 * scale)));
  const MAX_LANES = 14;

  let scroller = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let viewH = $state(600);
  let rebaseFrom = $state<CommitRow | null>(null);
  let searchEl = $state<HTMLInputElement>();
  let seenSearch = 0;
  $effect(() => {
    const t = app.focusSearch;
    if (t > seenSearch && Date.now() - t < 1500 && searchEl) {
      seenSearch = t;
      searchEl.focus();
      searchEl.select();
    }
  });
  let dropLine = $state<{ oid: string; below: boolean } | null>(null);

  const q = $derived(app.historyQuery.trim().toLowerCase());
  const bis = $derived(app.status?.bisect ?? null);
  const bisGood = $derived(new Set(bis?.good ?? []));

  // Filters search the whole history on the backend (not just loaded commits).
  const f = $derived(app.historyFilter);
  const dated = $derived(f.when === "custom" ? !!(f.since || f.until) : !!f.when);
  const activeFilters = $derived([f.author, f.path, f.contains].filter((v) => v.trim()).length + (dated ? 1 : 0));
  let filtersOpen = $state(false);
  $effect(() => {
    if (activeFilters) filtersOpen = true;
  });
  let found = $state.raw<SearchResult | null>(null);
  let searching = $state(false);
  let searchSeq = 0;
  const day = (d: string, end: boolean) => (d ? Math.floor(new Date(`${d}T${end ? "23:59:59" : "00:00:00"}`).getTime() / 1000) : null);
  $effect(() => {
    void app.refsVersion;
    const filter = {
      text: q || null, author: f.author.trim() || null, path: f.path.trim() || null, contains: f.contains || null,
      since: f.when === "custom" ? day(f.since, false) : f.when ? Math.floor(Date.now() / 1000) - Number(f.when) * 86400 : null,
      until: f.when === "custom" ? day(f.until, true) : null,
    };
    const scope = app.scope;
    const repo = app.path;
    if (!activeFilters) {
      found = null;
      searching = false;
      return;
    }
    const seq = ++searchSeq;
    searching = true;
    const t = setTimeout(() => {
      api.searchLog(repo, scope, filter, 2000)
        .then((r) => seq === searchSeq && (found = r))
        .catch(() => seq === searchSeq && (found = { rows: [], scanned: 0, more: false }))
        .finally(() => seq === searchSeq && (searching = false));
    }, 300);
    return () => clearTimeout(t);
  });
  const authors = $derived([...new Set(app.log.slice(0, 5000).map((r) => r.author))].sort((a, b) => a.localeCompare(b)));
  const filtering = $derived(!!q || activeFilters > 0);

  const rows = $derived(
    activeFilters ? (found?.rows ?? [])
    : q ? app.log.filter((r) => r.summary.toLowerCase().includes(q) || r.author.toLowerCase().includes(q) || r.oid.startsWith(q) || r.refs.some((l) => l.name.toLowerCase().includes(q))) : app.log,
  );
  const lanes = $derived(Math.min(MAX_LANES, Math.max(1, ...app.log.slice(0, 4000).map((r) => r.graph.width))));
  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - 15));
  const last = $derived(Math.min(rows.length, Math.ceil((scrollTop + viewH) / ROW) + 15));
  const visible = $derived(rows.slice(first, last));
  const rewritableIds = $derived(new Set(act.rewritable().map((c) => c.oid)));
  const pushed = $derived(act.pushedSet());
  const head = $derived(app.status?.head);

  // Where the list is scrolled is kept as the commit at its top (and how far
  // into it), not as a pixel offset: the list is rebuilt after every change
  // (a commit, a fetch, switching branches or tabs), and new commits at the
  // top would otherwise push the view further into the past each time. At
  // the very top it stays at the top, so new commits come into view.
  let anchor: { oid: string; offset: number } | null = untrack(() => app.historyAnchor);
  let placedRows: CommitRow[] | null = null;
  let placedRow = 0;

  function remember() {
    if (!scroller) return;
    const top = scroller.scrollTop;
    scrollTop = top;
    const i = Math.floor(top / ROW);
    anchor = top >= ROW / 2 && rows[i] ? { oid: rows[i].oid, offset: top - i * ROW } : null;
    app.historyAnchor = anchor;
    app.historyScroll = Math.round(top);
  }

  function scrollTo(y: number) {
    if (!scroller) return;
    scroller.scrollTop = Math.max(0, y);
    remember();
  }

  /** Scroll so row `i` is in view; `centre` puts it a third of the way down. */
  function show(i: number, centre: boolean) {
    if (!scroller) return;
    const top = i * ROW;
    const h = scroller.clientHeight || viewH;
    if (top >= scroller.scrollTop && top + ROW <= scroller.scrollTop + h) return;
    if (centre) scrollTo(top - h / 3);
    else scrollTo(top < scroller.scrollTop ? top : top + ROW - h);
  }

  // Place the list when its commits change (or the row height does), and
  // scroll to a commit something asked to show.
  $effect(() => {
    const list = rows;
    const sc = scroller;
    const rowH = ROW;
    const want = app.revealOid;
    if (!sc || !list.length) return;
    untrack(() => {
      if (want) {
        const i = list.findIndex((r) => r.oid === want);
        if (i >= 0) {
          placedRows = list;
          placedRow = rowH;
          app.revealOid = null;
          show(i, true);
          return;
        }
        // Not among the loaded commits (or hidden by a search): drop the
        // request rather than jump there later, out of the blue.
        if (!app.logLoading) app.revealOid = null;
      }
      if (list === placedRows && rowH === placedRow) return;
      const firstTime = placedRows === null;
      placedRows = list;
      placedRow = rowH;
      if (!anchor) {
        // At the top (or saved by an older version, as a pixel offset).
        return scrollTo(firstTime ? app.historyScroll : 0);
      }
      const i = list.findIndex((r) => r.oid === anchor!.oid);
      if (i >= 0) return scrollTo(i * rowH + Math.min(anchor.offset, rowH - 1));
      // The commit at the top isn't in this list (another scope or a
      // search): start at the top, or at the selected commit if it's there.
      scrollTo(0);
      const sel = list.findIndex((r) => r.oid === app.selectedCommit);
      if (sel >= 0) show(sel, true);
    });
  });

  // Another branch's history starts from its newest commit, at the top.
  let lastScope = untrack(() => app.scope);
  $effect(() => {
    const sc = app.scope;
    if (sc !== lastScope) {
      lastScope = sc;
      anchor = null;
    }
  });

  // Clearing a search or filter goes back to the selected commit, now among
  // all the others.
  let wasFiltering = untrack(() => filtering);
  $effect(() => {
    const f = filtering;
    if (wasFiltering && !f && app.selectedCommit) app.revealOid = app.selectedCommit;
    wasFiltering = f;
  });

  // Select the head commit when nothing is selected.
  $effect(() => {
    if (!app.selectedCommit && rows.length) app.selectedCommit = rows.find((r) => r.oid === head)?.oid ?? rows[0].oid;
  });

  function key(e: KeyboardEvent) {
    const page = Math.max(1, Math.floor((scroller?.clientHeight || viewH) / ROW) - 1);
    const step: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, PageDown: page, PageUp: -page, End: Infinity, Home: -Infinity };
    if (!(e.key in step) || e.altKey || e.ctrlKey || e.metaKey) return;
    e.preventDefault();
    if (!rows.length) return;
    const i = rows.findIndex((r) => r.oid === app.selectedCommit);
    const j = Math.max(0, Math.min(rows.length - 1, (i < 0 ? 0 : i) + step[e.key]));
    if (rows[j]) {
      app.selectedCommit = rows[j].oid;
      show(j, false);
    }
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
    const set = (s: string) => app.setScope(s);
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
    const move = (ev: PointerEvent) => (app.layout.listW = Math.min(80, Math.max(30, ((ev.clientX - r.left) / r.width) * 100)));
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", () => window.removeEventListener("pointermove", move), { once: true });
  }
</script>

<div class="history" style:--list-w="{app.layout.listW}%" style:--row-h="{ROW}px">
  <div class="list">
    <div class="tools">
      <button class="scope" onclick={scopeMenu}><Icon name="history" size={14} /> {scopeLabel} <Icon name="down" size={12} /></button>
      <div class="search">
        <Icon name="search" size={14} />
        <input bind:this={searchEl} placeholder="Search message, author, ID" bind:value={app.historyQuery} spellcheck="false"
          onkeydown={(e) => { if (e.key === "Escape") { app.historyQuery = ""; scroller?.focus(); } if (e.key === "Enter") scroller?.focus(); }} />
        {#if app.historyQuery}<button class="clear" onclick={() => (app.historyQuery = "")} aria-label="Clear search"><Icon name="x" size={12} /></button>{/if}
      </div>
      <button class="ftoggle" class:on={filtersOpen || activeFilters} onclick={() => (filtersOpen = !filtersOpen)} aria-expanded={filtersOpen}
        title="Filter by author, date, path or changed text">
        <Icon name="filter" size={13} /> Filters{#if activeFilters}<span class="fcount">{activeFilters}</span>{/if}
      </button>
    </div>
    {#if filtersOpen}
      <div class="filters">
        <label><span>Author</span><input list="hist-authors" bind:value={app.historyFilter.author} placeholder="Name or email" spellcheck="false" /></label>
        <datalist id="hist-authors">{#each authors as a}<option value={a}></option>{/each}</datalist>
        <label><span>When</span>
          <select bind:value={app.historyFilter.when}>
            <option value="">Any time</option>
            <option value="1">Last 24 hours</option>
            <option value="7">Last 7 days</option>
            <option value="30">Last 30 days</option>
            <option value="90">Last 3 months</option>
            <option value="365">Last year</option>
            <option value="custom">Between dates…</option>
          </select>
        </label>
        {#if app.historyFilter.when === "custom"}
          <label><span>From</span><input type="date" bind:value={app.historyFilter.since} /></label>
          <label><span>To</span><input type="date" bind:value={app.historyFilter.until} /></label>
        {/if}
        <label class="wide"><span>Path</span><input bind:value={app.historyFilter.path} placeholder="File or folder, e.g. src/lib" spellcheck="false" /></label>
        <label class="wide" title="Commits that add or remove this text (like git log -S)"><span>Changes containing</span><input class="mono" bind:value={app.historyFilter.contains} placeholder="Code or text that was added or removed" spellcheck="false" /></label>
        <div class="fstatus">
          {#if searching}<span class="spin"></span> Searching history…
          {:else if found && activeFilters}{found.rows.length.toLocaleString()} commit{found.rows.length === 1 ? "" : "s"}{found.more ? "+" : ""} match · searched {found.scanned.toLocaleString()}{found.more ? " (stopped early; narrow the filters)" : ""}{/if}
          <span class="sp"></span>
          {#if activeFilters}<button class="link" onclick={() => (app.historyFilter = { ...NO_FILTER })}>Clear filters</button>{/if}
        </div>
      </div>
    {/if}

    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="rows" tabindex="0" role="listbox" aria-label="Commits" bind:this={scroller} bind:clientHeight={viewH}
      onscroll={remember} onkeydown={key}>
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
            {#if !filtering}
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
              {#if bis}
                {#if bis.bad === row.oid}<span class="bmark bad" title="Bisect: has the bug">{bis.found === row.oid ? "first bad" : "bad"}</span>
                {:else if bisGood.has(row.oid)}<span class="bmark good" title="Bisect: works">good</span>
                {:else if bis.skipped.includes(row.oid)}<span class="bmark" title="Bisect: skipped">skipped</span>
                {:else if bis.current === row.oid && !bis.found}<span class="bmark test" title="Bisect: being tested">testing</span>{/if}
              {/if}
              <span class="summary" class:merge={row.parents.length > 1}><Linked text={row.summary} /></span>
            </div>
            {#if canMove && !filtering}<span class="grip" title="Drag to reorder" class:pushed={pushed.has(row.oid)}><Icon name="grip" size={12} /></span>{/if}
            <span class="avatar" style:--h={authorHue(row.email)} title="{row.author} <{row.email}>">{initials(row.author)}</span>
            <span class="author">{row.author}</span>
            <span class="date" title={fullDate(row.time)}>{relTime(row.time)}</span>
          </div>
        {/each}
        {#if app.logMore && !filtering}
          <button class="btn more" style:top="{rows.length * ROW + 12}px" onclick={() => app.loadMoreLog()}>Load more commits</button>
        {/if}
      </div>
      {#if !rows.length}
        <div class="empty"><div>
          {#if app.logLoading}<strong>Loading history…</strong>
          {:else if activeFilters}<strong>{searching ? "Searching…" : "No matching commits"}</strong>{searching ? "" : "Across the whole history of this branch selection."}
          {:else if q}<strong>No matching commits</strong>Searching the {app.log.length} loaded commits. Open Filters to search the whole history.
          {:else}<strong>No commits yet</strong>Stage some changes and commit them.{/if}
        </div></div>
      {/if}
    </div>
  </div>

  <div class="splitter" role="separator" aria-orientation="vertical" title="Drag to resize · double-click to reset" onpointerdown={startResize} ondblclick={() => (app.layout.listW = 55)}></div>

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
  .tools { display: flex; gap: 8px; align-items: center; padding: 6px 10px; border-bottom: 1px solid var(--line); }
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
  .bmark { flex: none; font-size: calc(var(--fs) * 10.5); font-weight: 700; border-radius: 4px; padding: 0 5px; margin-right: 6px; color: var(--ink-3); border: 1px solid currentColor; }
  .bmark.bad { color: var(--danger); }
  .bmark.good { color: var(--add-ink); }
  .bmark.test { color: var(--warn); }
  .ftoggle { display: flex; align-items: center; gap: 5px; padding: 3px 8px; border-radius: 6px; color: var(--ink-2); font-weight: 600; font-size: calc(var(--fs) * 12); white-space: nowrap; }
  .ftoggle:hover, .ftoggle.on { background: var(--surface-3); color: var(--ink); }
  .fcount { background: var(--accent); color: var(--accent-ink); border-radius: 8px; padding: 0 6px; font-size: calc(var(--fs) * 10.5); }
  .filters { display: flex; flex-wrap: wrap; gap: 6px 10px; padding: 8px 10px; border-bottom: 1px solid var(--line); background: var(--surface-2); font-size: calc(var(--fs) * 12); }
  .filters label { display: flex; flex-direction: column; gap: 2px; color: var(--ink-3); font-weight: 600; flex: 1 1 130px; min-width: 0; }
  .filters label.wide { flex: 1 1 220px; }
  .filters input, .filters select { padding: 0 7px; height: 28px; font-weight: 400; }
  .fstatus { flex: 1 0 100%; display: flex; align-items: center; gap: 6px; color: var(--ink-3); min-height: 18px; }
  .fstatus .sp { flex: 1; }
  .spin { width: 10px; height: 10px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--accent); animation: hspin 0.8s linear infinite; }
  @keyframes hspin { to { rotate: 1turn; } }

  .rows { flex: 1; overflow: auto; position: relative; outline: none; }
  .spacer { position: relative; min-width: 100%; }
  .row {
    position: absolute; left: 0; right: 0; height: var(--row-h); display: flex; align-items: center; gap: 6px;
    padding-right: 10px; white-space: nowrap; font-size: calc(var(--fs) * 12.5);
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
    width: calc(var(--fs) * 16); height: calc(var(--fs) * 16); border-radius: 50%; flex: none; display: grid; place-items: center;
    font-size: calc(var(--fs) * 7.5); font-weight: 700; letter-spacing: 0.02em;
    background: hsl(var(--h) var(--av-bg-s) var(--av-bg-l)); color: hsl(var(--h) var(--av-ink-s) var(--av-ink-l));
  }
  .author { width: calc(var(--fs) * 96); overflow: hidden; text-overflow: ellipsis; color: var(--ink-2); font-size: calc(var(--fs) * 11.5); }
  .date { width: calc(var(--fs) * 68); text-align: right; color: var(--ink-3); font-size: calc(var(--fs) * 11.5); font-variant-numeric: tabular-nums; }
  .row .pill { height: calc(var(--fs) * 16); padding: 0 5px; font-size: calc(var(--fs) * 10.5); gap: 3px; }
  .more { position: absolute; left: 50%; translate: -50% 0; }

  .splitter { cursor: col-resize; background: var(--line); border-inline: 2px solid var(--surface); }
  .splitter:hover { background: var(--accent); }
  .detail { min-width: 0; min-height: 0; display: flex; flex-direction: column; }
</style>
