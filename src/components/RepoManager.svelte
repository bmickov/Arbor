<script lang="ts">
  import { shortPath } from "../lib/platform";
  import type { Bookmark, RepoSummary } from "../lib/api";
  import { api } from "../lib/api";
  import { draggable, dropTarget } from "../lib/drag.svelte";
  import { fullDate, relTime } from "../lib/format";
  import { library } from "../lib/library.svelte";
  import { app } from "../lib/state.svelte";
  import { menu, type MenuItem } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";

  let query = $state("");
  let filterBox = $state<HTMLInputElement>();
  // Ctrl+F on the repository list focuses the filter.
  $effect(() => {
    if (app.focusRepoFilter) {
      filterBox?.focus();
      filterBox?.select();
    }
  });
  let sort = $state<"name" | "recent">((() => {
    try {
      return (localStorage.getItem("arbor.repoSort") as "name" | "recent") || "name";
    } catch {
      return "name";
    }
  })());
  let selected = $state<Set<string>>(new Set());
  let anchor: string | null = null;
  let groupDrop = $state<string | null>(null);

  $effect(() => {
    try {
      localStorage.setItem("arbor.repoSort", sort);
    } catch {}
  });

  // Keep statuses fresh while the manager is on screen.
  $effect(() => {
    library.ensure().then(() => library.refreshSummaries());
    const t = setInterval(() => {
      if (document.hasFocus() && !library.bulk) library.refreshSummaries();
    }, 20000);
    const onFocus = () => library.refreshSummaries();
    window.addEventListener("focus", onFocus);
    return () => {
      clearInterval(t);
      window.removeEventListener("focus", onFocus);
    };
  });

  const q = $derived(query.trim().toLowerCase());
  function matches(r: Bookmark) {
    if (!q) return true;
    const s = library.summaries[r.path];
    return r.name.toLowerCase().includes(q) || r.path.toLowerCase().includes(q) || !!s?.branch?.toLowerCase().includes(q);
  }
  function sorted(list: Bookmark[]) {
    return [...list].sort((a, b) =>
      sort === "recent" ? b.opened - a.opened || a.name.localeCompare(b.name) : a.name.localeCompare(b.name, undefined, { sensitivity: "base", numeric: true }),
    );
  }

  const groupIds = $derived(new Set(library.groups.map((g) => g.id)));
  const sections = $derived([
    ...library.groups.map((g) => ({ group: g, repos: sorted(library.repos.filter((r) => r.group === g.id && matches(r))) })),
    { group: null, repos: sorted(library.repos.filter((r) => (!r.group || !groupIds.has(r.group)) && matches(r))) },
  ]);
  // Flat order of visible rows, for shift-click ranges.
  const visibleOrder = $derived(sections.flatMap((s) => (s.group?.collapsed ? [] : s.repos.map((r) => r.path))));
  const openPaths = $derived(new Set(app.tabs.map((t) => t.path)));

  function click(e: MouseEvent, path: string) {
    if (e.ctrlKey || e.metaKey) {
      const s = new Set(selected);
      s.has(path) ? s.delete(path) : s.add(path);
      selected = s;
      anchor = path;
      return;
    }
    if (e.shiftKey && anchor) {
      const a = visibleOrder.indexOf(anchor);
      const b = visibleOrder.indexOf(path);
      if (a >= 0 && b >= 0) {
        selected = new Set(visibleOrder.slice(Math.min(a, b), Math.max(a, b) + 1));
        return;
      }
    }
    selected = new Set();
    anchor = path;
    openRepo(path);
  }

  function openRepo(path: string, background = false) {
    const s = library.summaries[path];
    if (s && !s.ok) return;
    app.open(path, { background });
  }

  const targets = (path: string) => (selected.has(path) ? [...selected] : [path]);

  function repoMenu(e: MouseEvent, path: string) {
    const paths = targets(path);
    const many = paths.length > 1;
    const s = library.summaries[path];
    const missing = s && !s.ok;
    const moveItems: MenuItem[] = [
      ...library.groups.map((g) => ({ label: `Move to ${g.name}`, action: () => library.moveTo(paths, g.id) })),
      ...(library.groups.length ? [{ label: "Move to Ungrouped", action: () => library.moveTo(paths, null) }] : []),
      { label: "Move to a new group…", action: () => library.newGroup(paths) },
    ];
    menu.fromEvent(e, [
      ...(many
        ? [{ label: `Open ${paths.length} in tabs`, action: () => paths.forEach((p, i) => openRepo(p, i > 0)) }]
        : [
            { label: "Open", disabled: missing, action: () => openRepo(path) },
            { label: "Open in background tab", disabled: missing, action: () => openRepo(path, true) },
          ]),
      { label: many ? `Fetch ${paths.length} repositories` : "Fetch", disabled: missing, action: () => library.fetchAll(paths) },
      { separator: true },
      ...(many ? [] : [
        { label: "Show in file manager", disabled: missing, action: () => api.openExternal(path) },
        { label: "Open terminal here", disabled: missing, action: () => api.openTerminal(path) },
        { label: "Copy path", action: () => navigator.clipboard.writeText(path) },
        { label: "Rename in list…", action: async () => { await library.rename(path); const r = library.repos.find((x) => x.path === path); if (r) app.renameTab(path, r.name); } },
        ...(missing ? [{ label: "Locate…", action: () => library.locate(path) }] : []),
      ]),
      { separator: true },
      ...moveItems,
      { separator: true },
      { label: many ? `Remove ${paths.length} from list` : "Remove from list", danger: true, action: async () => { await library.remove(paths); selected = new Set(); } },
    ], many ? `${paths.length} repositories` : library.name(path));
  }

  function groupMenu(e: MouseEvent, id: string) {
    const paths = library.repos.filter((r) => r.group === id).map((r) => r.path);
    menu.fromEvent(e, [
      { label: "Open all in tabs", disabled: !paths.length, action: () => paths.forEach((p, i) => openRepo(p, i > 0)) },
      { label: "Fetch all in group", disabled: !paths.length, action: () => library.fetchAll(paths) },
      { separator: true },
      { label: "Rename group…", action: () => library.renameGroup(id) },
      { label: "Delete group…", danger: true, action: () => library.deleteGroup(id) },
    ]);
  }

  function addMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu.open(r.left, r.bottom + 4, [
      { label: "Add existing repositories…", action: () => library.addFolders() },
      { label: "Find repositories in a folder…", action: () => library.scanFolder() },
      { separator: true },
      { label: "Clone…", action: async () => { const p = await library.clone(); if (p) app.open(p); } },
      { label: "Create new repository…", action: async () => { const p = await library.create(); if (p) app.open(p); } },
    ]);
  }

  function groupStats(repos: Bookmark[]) {
    let dirty = 0, behind = 0, ahead = 0;
    for (const r of repos) {
      const s = library.summaries[r.path];
      if (!s?.ok) continue;
      if (s.changes) dirty++;
      if (s.behind) behind++;
      if (s.ahead) ahead++;
    }
    const parts = [];
    if (dirty) parts.push(`${dirty} with changes`);
    if (ahead) parts.push(`${ahead} to push`);
    if (behind) parts.push(`${behind} to pull`);
    return parts.join(" · ");
  }

  function stateLabel(s: RepoSummary) {
    if (s.state === "clean") return null;
    if (s.state.startsWith("rebase")) return "rebasing";
    if (s.state.startsWith("merge")) return "merging";
    if (s.state.startsWith("cherry")) return "cherry-picking";
    if (s.state.startsWith("revert")) return "reverting";
    return s.state;
  }

  const total = $derived(library.repos.length);


  // Each row is its own grid, so columns can't size to their content by
  // themselves. Measure the longest name/path and branch (in characters) and
  // size those columns to fit, within limits.
  const colWidths = $derived.by(() => {
    let name = 12, branch = 6;
    for (const r of library.repos) {
      const s = library.summaries[r.path];
      name = Math.max(name, r.name.length + (openPaths.has(r.path) ? 6 : 0), shortPath(r.path).length * 0.92);
      if (s?.ok) branch = Math.max(branch, (s.detached ? 8 : (s.branch?.length ?? 1)) + (stateLabel(s) ? 9 : 0));
    }
    // A column the user has dragged keeps exactly that width; the others size
    // themselves as above.
    const track = (id: string, auto: string) => (app.layout.sizes[`repos.${id}`] != null ? `${app.layout.sizes[`repos.${id}`]}px` : auto);
    return [
      `--c-name: ${track("name", `minmax(150px, calc(${Math.min(name, 44).toFixed(1)}ch + 8px))`)}`,
      `--c-branch: ${track("branch", `minmax(80px, calc(${Math.min(branch, 32)}ch + 26px))`)}`,
      `--c-sync: ${track("sync", "48px")}`,
      `--c-changes: ${track("changes", "132px")}`,
      `--c-last: ${track("last", "minmax(160px, 780px)")}`,
    ].join("; ");
  });

  /** Resizable columns: label, saved-size id, and the narrowest it may get. */
  const COLUMNS = [
    { id: "name", label: "Repository", min: 100 },
    { id: "branch", label: "Branch", min: 60 },
    { id: "sync", label: "Sync", min: 36 },
    { id: "changes", label: "Changes", min: 56 },
    { id: "last", label: "Last commit", min: 120 },
  ] as const;
  let resizing = $state<string | null>(null);

  function setCol(id: string, min: number, w: number) {
    app.layout.sizes[`repos.${id}`] = Math.round(Math.max(min, Math.min(w, 1600)));
  }

  function grab(e: PointerEvent, id: string, min: number) {
    if (e.button !== 0) return;
    e.preventDefault();
    const cell = (e.currentTarget as HTMLElement).parentElement!;
    const start = cell.getBoundingClientRect().width;
    const x0 = e.clientX;
    resizing = id;
    document.body.classList.add("resizing-x");
    const move = (ev: PointerEvent) => setCol(id, min, start + ev.clientX - x0);
    const up = () => {
      resizing = null;
      document.body.classList.remove("resizing-x");
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function nudge(e: KeyboardEvent, id: string, min: number) {
    const dir = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!dir) return;
    e.preventDefault();
    const w = (e.currentTarget as HTMLElement).parentElement!.getBoundingClientRect().width;
    setCol(id, min, w + dir * (e.shiftKey ? 64 : 16));
  }

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") selected = new Set();
    if ((e.key === "Delete" || e.key === "Backspace") && selected.size && !(e.target as HTMLElement).closest("input")) {
      library.remove([...selected]);
      selected = new Set();
    }
  }
</script>

<svelte:window onkeydown={key} />

<div class="mgr">
  {#if total === 0}
    <div class="first">
      <svg class="mark" viewBox="0 0 64 64" aria-hidden="true">
        <g fill="none" stroke-linecap="round" stroke-width="5">
          <path d="M24 54V14" stroke="var(--lane-0)" />
          <path d="M24 42c0-8 16-6 16-16v-6" stroke="var(--lane-1)" />
          <path d="M24 32c0-5-8-5-8-10" stroke="var(--lane-2)" />
        </g>
        <circle cx="24" cy="54" r="5" fill="var(--lane-0)" />
        <circle cx="24" cy="14" r="5" fill="var(--lane-0)" />
        <circle cx="40" cy="20" r="5" fill="var(--lane-1)" />
        <circle cx="16" cy="22" r="4.5" fill="var(--lane-2)" />
      </svg>
      <h1>Welcome to Arbor</h1>
      <p class="muted">Add the repositories you work on. They stay in this list, with their status at a glance.</p>
      <div class="cards">
        <button class="card" onclick={() => library.scanFolder()}>
          <Icon name="search" size={18} />
          <b>Find repositories in a folder</b>
          <span>Point at a folder like ~/code and add everything inside it at once.</span>
        </button>
        <button class="card" onclick={() => library.addFolders()}>
          <Icon name="folder" size={18} />
          <b>Add existing repositories</b>
          <span>Pick one or more repository folders.</span>
        </button>
        <button class="card" onclick={async () => { const p = await library.clone(); if (p) app.open(p); }}>
          <Icon name="fetch" size={18} />
          <b>Clone</b>
          <span>Copy a repository from GitHub, GitLab or any Git server.</span>
        </button>
        <button class="card" onclick={async () => { const p = await library.create(); if (p) app.open(p); }}>
          <Icon name="plus" size={18} />
          <b>Create new repository</b>
          <span>Start version control in a new or existing folder.</span>
        </button>
      </div>
      {#if library.bulk}<p class="faint working"><span class="spinner"></span>{library.bulk}</p>{/if}
    </div>
  {:else}
    <header>
      <div class="title">
        <h1>Repositories</h1>
        <span class="faint">{total}</span>
      </div>
      <div class="search">
        <Icon name="search" size={14} />
        <input placeholder="Filter by name, path or branch" bind:value={query} bind:this={filterBox} spellcheck="false" />
        {#if query}<button class="clear" onclick={() => (query = "")} aria-label="Clear filter"><Icon name="x" size={12} /></button>{/if}
      </div>
      <div class="actions">
        <div class="seg" role="group" aria-label="Sort">
          <button class:on={sort === "name"} onclick={() => (sort = "name")}>Name</button>
          <button class:on={sort === "recent"} onclick={() => (sort = "recent")}>Recent</button>
        </div>
        <button class="btn" onclick={() => library.newGroup([...selected])} title="Create a group"><Icon name="group" size={14} /> New group</button>
        <button class="btn" onclick={() => library.fetchAll()} disabled={!!library.bulk}><Icon name="fetch" size={14} /> Fetch all</button>
        <button class="btn primary" onclick={addMenu}><Icon name="plus" size={14} /> Add <Icon name="down" size={12} /></button>
      </div>
    </header>

    {#if library.bulk || selected.size}
      <div class="statusline">
        {#if library.bulk}<span class="working"><span class="spinner"></span>{library.bulk}</span>{/if}
        {#if selected.size}
          <span class="sel">{selected.size} selected — drag them onto a group, or right-click for more.</span>
          <button class="link" onclick={() => (selected = new Set())}>Clear</button>
        {/if}
      </div>
    {/if}

    <div class="colhead" style={colWidths} role="row">
      <span></span>
      {#each COLUMNS as c (c.id)}
        <span class="col col-{c.id}" role="columnheader">
          <span class="clabel">{c.label}</span>
          <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
          <span class="grip" class:on={resizing === c.id} role="separator" aria-orientation="vertical" tabindex="0"
            aria-label="Resize {c.label} column"
            title="Drag to resize · double-click to fit"
            onpointerdown={(e) => grab(e, c.id, c.min)}
            onkeydown={(e) => nudge(e, c.id, c.min)}
            ondblclick={() => delete app.layout.sizes[`repos.${c.id}`]}></span>
        </span>
      {/each}
    </div>

    <div class="list" style={colWidths}>
      {#each sections as sec (sec.group?.id ?? "__ungrouped")}
        {@const gid = sec.group?.id ?? null}
        {#if sec.group || sec.repos.length || library.groups.length === 0}
          <section class:dropping={groupDrop === (gid ?? "__ungrouped")}>
            {#if sec.group || library.groups.length}
              <div class="ghead" role="group" aria-label={sec.group?.name ?? "Ungrouped"}
                use:dropTarget={{
                  accepts: (p) => p.kind === "repos" || (p.kind === "group" && !!sec.group && p.id !== sec.group.id),
                  hover: () => (groupDrop = gid ?? "__ungrouped"),
                  leave: () => (groupDrop = null),
                  drop: (p) => {
                    groupDrop = null;
                    if (p.kind === "repos") { library.moveTo(p.paths, gid); selected = new Set(); }
                    if (p.kind === "group") library.moveGroup(p.id, gid);
                  },
                }}
                use:draggable={() => (sec.group ? { payload: { kind: "group", id: sec.group.id }, label: sec.group.name } : null)}
                oncontextmenu={(e) => sec.group && groupMenu(e, sec.group.id)}>
                <button class="gtoggle" onclick={() => sec.group && library.toggleGroup(sec.group.id)} aria-expanded={!sec.group?.collapsed} disabled={!sec.group}>
                  {#if sec.group}<span class="chev" class:open={!sec.group.collapsed}><Icon name="chevron" size={12} /></span>{/if}
                  <span class="gname">{sec.group?.name ?? "Ungrouped"}</span>
                  <span class="faint gcount">{sec.repos.length}</span>
                </button>
                <span class="gstats faint">{groupStats(sec.repos)}</span>
                {#if sec.group}
                  <button class="gmore" aria-label="Group options" onclick={(e) => groupMenu(e, sec.group!.id)}><Icon name="more" size={16} /></button>
                {/if}
              </div>
            {/if}

            {#if !sec.group?.collapsed}
              <div class="rows"
                use:dropTarget={{
                  accepts: (p) => p.kind === "repos",
                  hover: () => (groupDrop = gid ?? "__ungrouped"),
                  leave: () => (groupDrop = null),
                  drop: (p) => { groupDrop = null; if (p.kind === "repos") { library.moveTo(p.paths, gid); selected = new Set(); } },
                }}>
                {#each sec.repos as r (r.path)}
                  {@const s = library.summaries[r.path]}
                  {@const missing = s && !s.ok}
                  <div class="row" class:sel={selected.has(r.path)} class:missing class:open={openPaths.has(r.path)}
                    role="button" tabindex="0"
                    onclick={(e) => click(e, r.path)}
                    onkeydown={(e) => e.key === "Enter" && openRepo(r.path)}
                    oncontextmenu={(e) => repoMenu(e, r.path)}
                    use:draggable={() => {
                      const paths = targets(r.path);
                      return { payload: { kind: "repos", paths }, label: paths.length > 1 ? `${paths.length} repositories` : r.name };
                    }}>
                    <span class="state">
                      {#if library.working[r.path]}
                        <span class="spinner" title={library.working[r.path]}></span>
                      {:else if missing}
                        <span class="dot gone" title={s?.error ?? ""}></span>
                      {:else if s?.conflicts}
                        <span class="dot conflict" title="Conflicts"></span>
                      {:else if s?.changes}
                        <span class="dot dirty" title="Uncommitted changes"></span>
                      {:else if s}
                        <span class="dot clean" title="Clean"></span>
                      {/if}
                    </span>
                    <span class="who">
                      <span class="rname">{r.name}{#if openPaths.has(r.path)}<span class="opentag">open</span>{/if}</span>
                      <span class="rpath faint"><bdi>{shortPath(r.path)}</bdi></span>
                    </span>
                    {#if missing}
                      <span class="missing-msg">{s?.error}</span>
                      <span class="fix">
                        <button class="btn small" onclick={(e) => { e.stopPropagation(); library.locate(r.path); }}>Locate…</button>
                        <button class="btn small" onclick={(e) => { e.stopPropagation(); library.remove([r.path]); }}>Remove</button>
                      </span>
                    {:else}
                      <span class="branch" title={s?.upstream ? `tracks ${s.upstream}` : ""}>
                        {#if s}
                          <Icon name="branch" size={13} />
                          <span>{s.detached ? "detached" : s.branch ?? "—"}</span>
                          {#if stateLabel(s)}<span class="tag warn">{stateLabel(s)}</span>{/if}
                        {/if}
                      </span>
                      <span class="sync">
                        {#if s?.ahead}<span title="{s.ahead} to push">↑{s.ahead}</span>{/if}
                        {#if s?.behind}<span class="behind" title="{s.behind} to pull">↓{s.behind}</span>{/if}
                      </span>
                      <span class="changes">
                        {#if s?.changes}<span title="Files with uncommitted changes">{s.changes} changed</span>{/if}
                        {#if s?.stashes}<span class="faint" title="Stashes"><Icon name="stash" size={12} /> {s.stashes}</span>{/if}
                      </span>
                      <span class="last" title={s?.lastCommit ? `${s.lastSummary}\n${fullDate(s.lastCommit)}` : ""}>
                        {#if s?.lastCommit}
                          <span class="lsum">{s.lastSummary}</span>
                          <span class="faint ltime">{relTime(s.lastCommit)}</span>
                        {:else if s}
                          <span class="faint">No commits yet</span>
                        {/if}
                      </span>
                    {/if}
                  </div>
                {:else}
                  <div class="none faint">
                    {q ? "No matches in this group." : sec.group ? "Drag repositories here." : ""}
                  </div>
                {/each}
              </div>
            {/if}
          </section>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .mgr { height: 100%; display: flex; flex-direction: column; min-height: 0; background: var(--surface); }
  header {
    display: grid; grid-template-columns: auto minmax(200px, 360px) 1fr; gap: 16px; align-items: center;
    padding: 18px 24px 14px; border-bottom: 1px solid var(--line);
  }
  .title { display: flex; align-items: baseline; gap: 8px; }
  h1 { margin: 0; font-size: calc(var(--fs) * 20); font-weight: 700; letter-spacing: -0.01em; }
  .search {
    display: flex; align-items: center; gap: 6px; padding: 0 9px; height: 30px;
    border: 1px solid var(--line); border-radius: var(--radius); color: var(--ink-3); background: var(--surface);
  }
  .search input { border: 0; padding: 0; height: 28px; background: transparent; box-shadow: none; }
  .clear { color: var(--ink-3); display: grid; }
  .actions { display: flex; gap: 8px; justify-content: flex-end; flex-wrap: wrap; }
  .seg { display: flex; background: var(--surface-3); border-radius: 7px; padding: 2px; }
  .seg button { padding: 3px 10px; border-radius: 5px; font-weight: 600; color: var(--ink-2); font-size: calc(var(--fs) * 12); }
  .seg button.on { background: var(--surface); color: var(--ink); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08); }

  .statusline { display: flex; align-items: center; gap: 14px; padding: 7px 24px; background: var(--surface-2); border-bottom: 1px solid var(--line-soft); font-size: calc(var(--fs) * 12); }
  .working { display: inline-flex; align-items: center; gap: 8px; color: var(--ink-2); }
  .sel { color: var(--ink-2); }
  .link { color: var(--accent); font-weight: 600; }

  .list { flex: 1; overflow-y: auto; padding: 8px 16px 40px; }
  section { border-radius: 10px; margin-top: 6px; }
  section.dropping { background: var(--accent-soft); box-shadow: inset 0 0 0 2px var(--accent); }
  .ghead { display: flex; align-items: center; gap: 10px; padding: 8px 8px 4px; border-radius: 8px; }
  .ghead:global(.drop-over), .rows:global(.drop-over) { box-shadow: none !important; background: transparent !important; }
  .gtoggle { display: flex; align-items: center; gap: 6px; font-weight: 700; color: var(--ink); }
  .gtoggle:disabled { opacity: 1; cursor: default; }
  .chev { display: inline-grid; color: var(--ink-3); transition: rotate 0.12s; }
  .chev.open { rotate: 90deg; }
  .gname { font-size: calc(var(--fs) * 14); }
  .gcount { font-weight: 500; }
  .gstats { flex: 1; font-size: calc(var(--fs) * 12); }
  .gmore { display: grid; place-items: center; width: 26px; height: 26px; border-radius: 6px; color: var(--ink-3); opacity: 0; }
  .ghead:hover .gmore { opacity: 1; }
  .gmore:hover { background: var(--surface-3); color: var(--ink); }

  .rows { padding: 2px 0 8px; }
  .row {
    /* Name and branch columns are sized to the longest entry (see colWidths),
       the rest have sensible maximums, and spare width collects in an empty
       last column: on wide windows the columns stay together instead of
       drifting apart (name → branch, and commit message → its time). */
    display: grid; grid-template-columns: 18px var(--c-name) var(--c-branch) var(--c-sync) var(--c-changes) var(--c-last) minmax(0, 1fr);
    gap: 12px; align-items: center; padding: 7px 10px; border-radius: 7px; cursor: default;
  }
  /* Column labels, lined up with the rows below (same grid, same insets). */
  .colhead {
    display: grid; grid-template-columns: 18px var(--c-name) var(--c-branch) var(--c-sync) var(--c-changes) var(--c-last) minmax(0, 1fr);
    gap: 12px; align-items: center; padding: 6px 26px 5px; border-bottom: 1px solid var(--line-soft); user-select: none;
  }
  .col { position: relative; min-width: 0; display: flex; }
  /* Label type is set here, not on .colhead: the auto widths are in ch, so the
     grid itself must use the rows' font size to line up with them. */
  .clabel {
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
    font-size: calc(var(--fs) * 10.5); font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; color: var(--ink-3);
  }
  /* The handle sits in the gap after the column, centred on it. */
  .grip { position: absolute; top: -6px; bottom: -5px; right: -12px; width: 12px; cursor: col-resize; z-index: 1; }
  .grip::after { content: ""; position: absolute; left: 5.5px; top: 4px; bottom: 4px; width: 1px; background: var(--line); }
  .colhead:hover .grip::after { background: var(--ink-3); }
  .grip:hover::after, .grip.on::after, .grip:focus-visible::after { background: var(--accent); width: 2px; left: 5px; }
  .grip:focus-visible { outline: none; }
  .row:hover { background: var(--surface-2); }
  .row.sel { background: var(--sel); }
  .row:focus-visible { outline: 2px solid var(--focus); outline-offset: -2px; }
  .state { display: grid; place-items: center; }
  .dot { width: 9px; height: 9px; border-radius: 50%; }
  .dot.clean { border: 2px solid var(--lane-0); }
  .dot.dirty { background: var(--lane-1); }
  .dot.conflict { background: var(--danger); }
  .dot.gone { border: 2px dashed var(--ink-3); }
  .who { min-width: 0; display: flex; flex-direction: column; }
  .rname { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; display: flex; align-items: center; gap: 6px; }
  .opentag { font-size: calc(var(--fs) * 10); font-weight: 700; color: var(--accent); border: 1px solid currentColor; border-radius: 4px; padding: 0 4px; line-height: 14px; }
  .rpath { font-size: calc(var(--fs) * 12); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; direction: rtl; text-align: left; }
  .branch { display: flex; align-items: center; gap: 5px; color: var(--ink-2); min-width: 0; font-size: calc(var(--fs) * 12); }
  .branch span:not(.tag) { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .branch :global(svg) { flex: none; }
  .tag { font-size: calc(var(--fs) * 10); font-weight: 700; border-radius: 4px; padding: 0 4px; flex: none; }
  .tag.warn { color: var(--warn); background: var(--warn-soft); }
  .sync { display: flex; gap: 6px; font-size: calc(var(--fs) * 12); font-variant-numeric: tabular-nums; color: var(--ink-2); }
  .sync .behind { color: var(--accent); font-weight: 700; }
  .changes { display: flex; gap: 8px; font-size: calc(var(--fs) * 12); color: var(--lane-1); font-weight: 600; align-items: center; }
  .changes .faint { display: inline-flex; gap: 3px; align-items: center; font-weight: 500; }
  .last { display: flex; gap: 8px; min-width: 0; font-size: calc(var(--fs) * 12); align-items: baseline; }
  .lsum { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; color: var(--ink-2); }
  .ltime { flex: none; min-width: 72px; text-align: right; }
  .row.missing .rname { color: var(--ink-3); }
  .missing-msg { grid-column: 3 / 5; color: var(--ink-3); font-size: calc(var(--fs) * 12); }
  .fix { grid-column: 5 / 7; display: flex; gap: 6px; justify-content: flex-end; }
  .none { padding: 6px 38px 10px; font-size: calc(var(--fs) * 12); }

  .spinner {
    display: inline-block; width: 12px; height: 12px; border-radius: 50%; border: 2px solid var(--line);
    border-top-color: var(--accent); animation: spin 0.8s linear infinite; flex: none;
  }
  @keyframes spin { to { transform: rotate(360deg); } }

  .first { margin: auto; width: min(720px, 100%); padding: 40px 24px; }
  .mark { width: 56px; height: 56px; }
  .first h1 { font-size: calc(var(--fs) * 26); margin: 8px 0 4px; }
  .first p { margin: 0 0 24px; }
  .cards { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  .card {
    display: grid; grid-template-columns: auto 1fr; gap: 4px 12px; text-align: left; padding: 14px 16px;
    border: 1px solid var(--line); border-radius: 10px; background: var(--surface);
  }
  .card:hover { border-color: var(--accent); background: var(--accent-soft); }
  .card :global(svg) { grid-row: span 2; color: var(--accent); margin-top: 2px; }
  .card b { font-weight: 700; }
  .card span { color: var(--ink-2); font-size: calc(var(--fs) * 12); }
  .first .working { margin-top: 16px; }

  @media (max-width: 1100px) {
    .row, .colhead { grid-template-columns: 18px var(--c-name) var(--c-branch) var(--c-sync) var(--c-changes); }
    .last, .col-last { display: none; }
  }
</style>
