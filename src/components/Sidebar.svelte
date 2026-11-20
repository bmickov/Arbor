<script lang="ts">
  import * as act from "../lib/actions";
  import { api, type Stacked, type SubmoduleInfo, type WorktreeInfo } from "../lib/api";
  import { draggable, dropTarget, type Payload } from "../lib/drag.svelte";
  import { app, type View } from "../lib/state.svelte";
  import { menu, toasts } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";
  import { SvelteSet } from "svelte/reactivity";

  let filter = $state("");

  // Several local branches can be picked with Ctrl+click / Shift+click and
  // deleted together. A plain click still just shows the branch's history.
  const picked = new SvelteSet<string>();
  let anchor: string | null = null;
  // The branch whose history is on screen.
  const shown = $derived(app.view === "history" && app.scope !== "all" ? app.scope : null);

  function clickBranch(e: MouseEvent, name: string) {
    if (e.ctrlKey || e.metaKey) {
      // Like a file manager: the branch already shown joins the selection.
      if (!picked.size && shown && shown !== name && locals.some((b) => b.name === shown)) picked.add(shown);
      picked.has(name) ? picked.delete(name) : picked.add(name);
      anchor = name;
    } else if (e.shiftKey) {
      const names = locals.map((b) => b.name);
      const from = anchor ?? shown ?? name;
      const a = names.includes(from) ? names.indexOf(from) : names.indexOf(name), b = names.indexOf(name);
      picked.clear();
      for (let i = Math.min(a, b); i <= Math.max(a, b); i++) picked.add(names[i]);
    } else {
      picked.clear();
      anchor = name;
      act.showHistory(name);
    }
  }

  function branchContext(e: MouseEvent, name: string) {
    if (picked.size > 1 && picked.has(name)) {
      menu.fromEvent(e, [
        { label: `Delete ${picked.size} branches…`, danger: true, action: deletePicked },
        { label: "Clear selection", action: () => picked.clear() },
      ], `${picked.size} branches`);
    } else act.branchMenu(e, name);
  }

  async function deletePicked() {
    if ((await act.deleteBranches([...picked])).length) picked.clear();
  }

  function branchKey(e: KeyboardEvent) {
    if (!picked.size) return;
    if (e.key === "Delete") {
      e.preventDefault();
      deletePicked();
    } else if (e.key === "Escape") picked.clear();
  }

  // Forget picks for branches that no longer exist (deleted, renamed).
  $effect(() => {
    const all = new Set(app.refs?.local.map((b) => b.name) ?? []);
    for (const n of [...picked]) if (!all.has(n)) picked.delete(n);
  });
  // Expanded sections are part of the saved layout.
  const open = $derived(app.layout.sections);
  const openRemote = $derived(app.layout.remotes);

  const st = $derived(app.status);
  const refs = $derived(app.refs);
  const changeCount = $derived((st?.staged.length ?? 0) + (st?.unstaged.length ?? 0) + (st?.conflicted.length ?? 0));
  const match = (s: string) => !filter || s.toLowerCase().includes(filter.toLowerCase());
  const locals = $derived((refs?.local ?? []).filter((b) => match(b.name)));

  // Pinned branches (local or remote) at the top.
  const pins = $derived.by(() => {
    void app.settings.pinned;
    const remoteNames = new Set((refs?.remotes ?? []).flatMap((r) => r.branches.map((b) => b.fullName)));
    return act.pinnedBranches().map((name) => {
      const local = refs?.local.find((b) => b.name === name);
      return { name, local, missing: !local && !remoteNames.has(name) };
    });
  });

  // Git LFS: what's tracked, and whether files still need downloading.
  $effect(() => {
    void app.path;
    app.loadLfs();
  });
  let lastLfsVersion = -1;
  $effect(() => {
    const v = app.version;
    if (app.lfs?.used && v !== lastLfsVersion) {
      lastLfsVersion = v;
      app.loadLfs();
    }
  });
  let locks = $state.raw<{ id: string; path: string; owner: { name: string } | null }[] | null>(null);
  let locksError = $state<string | null>(null);
  async function loadLocks() {
    locksError = null;
    try {
      locks = await api.lfsLocks(app.path);
    } catch (e) {
      locks = null;
      locksError = String(e);
    }
  }
  function lfsPatternMenu(e: MouseEvent, pattern: string) {
    menu.fromEvent(e, [{ label: `Stop tracking ${pattern} with LFS`, danger: true, action: () => act.lfsUntrack(pattern) }], pattern);
  }

  // Stacked branches sit under their parent, indented.
  let stacks = $state.raw<Stacked[]>([]);
  $effect(() => {
    void app.refsVersion;
    const p = app.path;
    if (!p) return;
    api.stackList(p).then((s) => {
      if (app.path !== p) return;
      stacks = s;
      act.setStacks(s);
    }).catch(() => {});
  });
  const stackOf = $derived(new Map(stacks.map((s) => [s.branch, s])));
  const ordered = $derived.by(() => {
    const names = new Set(locals.map((b) => b.name));
    const kids = new Map<string, typeof locals>();
    const roots: typeof locals = [];
    for (const b of locals) {
      const p = stackOf.get(b.name)?.parent;
      if (p && names.has(p) && !filter) kids.set(p, [...(kids.get(p) ?? []), b]);
      else roots.push(b);
    }
    const out: { b: (typeof locals)[number]; depth: number }[] = [];
    const walk = (b: (typeof locals)[number], depth: number) => {
      out.push({ b, depth });
      for (const k of kids.get(b.name) ?? []) walk(k, depth + 1);
    };
    roots.forEach((b) => walk(b, 0));
    return out;
  });
  const total = $derived((refs?.local.length ?? 0) + (refs?.remotes.reduce((n, r) => n + r.branches.length, 0) ?? 0));

  function go(v: View) {
    if (v === "history" && app.view === "history" && app.scope !== "all") app.setScope("all");
    app.view = v;
    if (v === "history" && !app.log.length) app.loadLog();
  }

  function dropOnRemote(p: Payload, remote: string, e: PointerEvent) {
    if (p.kind !== "branch") return;
    menu.open(e.clientX, e.clientY, [
      { label: `Push ${p.name} to ${remote}`, action: () => act.push(p.name, remote) },
    ]);
  }

  // Worktrees and submodules, reloaded whenever the repository changes.
  let worktrees = $state.raw<WorktreeInfo[]>([]);
  let submodules = $state.raw<SubmoduleInfo[]>([]);
  $effect(() => {
    void app.version;
    const p = app.path;
    if (!p) return;
    api.worktrees(p).then((w) => app.path === p && (worktrees = w)).catch(() => (worktrees = []));
    api.submodules(p).then((s) => app.path === p && (submodules = s)).catch(() => (submodules = []));
  });
  const linked = $derived(worktrees.filter((w) => !w.isMain));

  function openWorktree(w: WorktreeInfo) {
    if (w.isCurrent) return;
    if (w.missing) return toasts.push("error", `${w.path} doesn't exist any more. Right-click it to prune it.`);
    app.open(w.path);
  }
  function worktreeMenu(e: MouseEvent, w: WorktreeInfo) {
    menu.fromEvent(e, [
      { label: w.isCurrent ? "Open in this tab (already open)" : "Open in a tab", disabled: w.isCurrent || w.missing, action: () => app.open(w.path) },
      { label: "Show in file manager", disabled: w.missing, action: () => api.openExternal(w.path) },
      { label: "Open in terminal", disabled: w.missing, action: () => api.openTerminal(w.path) },
      { label: "Copy path", action: () => navigator.clipboard.writeText(w.path).then(() => toasts.push("info", "Copied path.")) },
      { separator: true },
      ...(w.missing
        ? [{ label: "Prune missing worktrees", action: () => app.act("Pruning", () => api.pruneWorktrees(app.path), { success: (n: number) => `Pruned ${n} worktree${n === 1 ? "" : "s"}.` }) }]
        : [{ label: "Remove worktree…", danger: true, disabled: w.isMain || w.isCurrent, action: () => act.removeWorktree(w) }]),
    ], w.branch ?? w.path);
  }

  function openSubmodule(s: SubmoduleInfo) {
    if (s.state === "uninitialized") act.updateSubmodules([s.path], s.path);
    else app.open(s.fullPath);
  }
  function submoduleMenu(e: MouseEvent, s: SubmoduleInfo) {
    const ready = s.state !== "uninitialized";
    menu.fromEvent(e, [
      { label: "Open in a tab", disabled: !ready, action: () => app.open(s.fullPath) },
      { label: ready ? `Update to ${s.recorded ?? "the recorded commit"}` : "Clone and check out", disabled: s.dirty, action: () => act.updateSubmodules([s.path], s.path) },
      { separator: true },
      { label: "Show in file manager", disabled: !ready, action: () => api.openExternal(s.fullPath) },
      { label: "Open in terminal", disabled: !ready, action: () => api.openTerminal(s.fullPath) },
      { label: "Copy URL", disabled: !s.url, action: () => navigator.clipboard.writeText(s.url!).then(() => toasts.push("info", "Copied URL.")) },
    ], s.path);
  }
  const smNote = (s: SubmoduleInfo) =>
    s.state === "uninitialized" ? "not cloned" : s.state === "moved" ? "moved" : s.state === "dirty" ? "changed" : "";
  const smTitle = (s: SubmoduleInfo) =>
    [s.url, s.state === "uninitialized" ? "Not cloned yet. Click to clone it." : s.state === "moved" ? `Checked out ${s.checkedOut}, but this repository records ${s.recorded}. Update to switch back.` : s.state === "dirty" ? "Has uncommitted changes inside." : `At ${s.recorded}`].filter(Boolean).join("\n");

  function dropOnChanges(p: Payload) {
    if (p.kind === "stash") act.applyStash(p.index, false);
  }
</script>

<nav class="side">
  <div class="views">
    <button class="nav" class:on={app.view === "changes"} onclick={() => go("changes")}
      use:dropTarget={{ accepts: (p) => p.kind === "stash", drop: dropOnChanges }}>
      <Icon name="changes" /><span>Changes</span>
      {#if changeCount}<b class="badge">{changeCount}</b>{/if}
      {#if st?.conflicted.length}<b class="badge warn" title="Conflicts">!</b>{/if}
    </button>
    <button class="nav" class:on={app.view === "history"} onclick={() => go("history")}>
      <Icon name="history" /><span>History</span>
      {#if app.view === "history" && app.scope !== "all"}<span class="scope faint">{app.scope}</span>{/if}
    </button>
    <button class="nav" class:on={app.view === "stashes"} onclick={() => go("stashes")}>
      <Icon name="stash" /><span>Stashes</span>
      {#if app.stashes.length}<b class="badge plain">{app.stashes.length}</b>{/if}
    </button>
    <button class="nav" class:on={app.view === "review"} onclick={() => go("review")} title="Which branches are merged, stale or gone">
      <Icon name="branch" /><span>Branches Review</span>
    </button>
    <button class="nav" class:on={app.view === "reflog"} onclick={() => go("reflog")} title="Recent positions of HEAD; recover lost commits">
      <Icon name="undo" /><span>Reflog</span>
    </button>
    {#if app.view === "compare" || app.view === "browse"}
      <button class="nav on">
        <Icon name={app.view === "compare" ? "split" : "folder"} />
        <span>{app.view === "compare" ? "Compare" : "Files"}</span>
        <span class="scope faint">{app.view === "compare" ? app.compare?.target : app.browse?.label}</span>
      </button>
    {/if}
  </div>

  <div class="lists">
    {#if total > 12}
      <div class="filter">
        <Icon name="search" size={14} />
        <input placeholder="Filter branches" bind:value={filter} spellcheck="false" />
      </div>
    {/if}

    {#if pins.length}
      <section>
        <header>
          <button class="head" onclick={() => (open.pinned = !open.pinned)} aria-expanded={open.pinned}>
            <span class="chev" class:open={open.pinned}><Icon name="chevron" size={12} /></span> Pinned
          </button>
        </header>
        {#if open.pinned}
          <ul>
            {#each pins as p (p.name)}
              <li>
                <button class="item" class:head={p.local?.isHead} class:shown={shown === p.name}
                  onclick={() => act.showHistory(p.name)}
                  ondblclick={() => (p.local ? !p.local.isHead && act.checkout(p.name) : act.checkoutRemote(p.name))}
                  oncontextmenu={(e) => (p.local ? branchContext(e, p.name) : act.remoteBranchMenu(e, p.name))}
                  use:dropTarget={{ accepts: act.acceptsOnBranch(p.name, !!p.local?.isHead), drop: (pl, e) => act.dropOnBranch(pl, p.name, e) }}>
                  <span class="pin" class:current={p.local?.isHead}><Icon name="pin" size={12} /></span>
                  <span class="name">{p.name}</span>
                  {#if p.local?.ahead}<span class="ab" title="{p.local.ahead} to push">↑{p.local.ahead}</span>{/if}
                  {#if p.local?.behind}<span class="ab" title="{p.local.behind} to pull">↓{p.local.behind}</span>{/if}
                  {#if p.missing}<span class="gone" title="This branch doesn't exist any more">gone</span>{/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/if}

    <section>
      <header>
        <button class="head" onclick={() => (open.branches = !open.branches)} aria-expanded={open.branches}>
          <span class="chev" class:open={open.branches}><Icon name="chevron" size={12} /></span> Branches
        </button>
        <button class="add" title="New branch" aria-label="New branch" onclick={() => act.newBranch()}><Icon name="plus" size={14} /></button>
      </header>
      {#if open.branches}
        <ul>
          {#each ordered as { b, depth } (b.name)}
            {@const st = stackOf.get(b.name)}
            <li style:padding-left="{depth * 14}px">
              <button class="item" class:head={b.isHead} class:picked={picked.has(b.name)} class:shown={!picked.size && shown === b.name}
                onclick={(e) => clickBranch(e, b.name)}
                onkeydown={branchKey}
                ondblclick={(e) => !b.isHead && !e.ctrlKey && !e.shiftKey && act.checkout(b.name)}
                oncontextmenu={(e) => branchContext(e, b.name)}
                use:draggable={() => ({ payload: { kind: "branch", name: b.name }, label: b.name })}
                use:dropTarget={{ accepts: act.acceptsOnBranch(b.name, b.isHead), drop: (p, e) => act.dropOnBranch(p, b.name, e) }}>
                {#if depth}<span class="stem" title="Stacked on {st?.parent}">↳</span>{:else}<span class="dot" class:current={b.isHead}></span>{/if}
                <span class="name" title={st ? `${b.name} — stacked on ${st.parent}` : b.name}>{b.name}</span>
                {#if st?.behind}
                  <span class="restack" role="button" tabindex="-1" title="{st.parent} has {st.behind} new commit{st.behind === 1 ? '' : 's'}. Click to restack {b.name} onto it."
                    onclick={(e) => { e.stopPropagation(); act.restack(b.name); }} onkeydown={() => {}}>restack</span>
                {/if}
                {#if b.upstreamGone}<span class="gone" title="The remote branch was deleted">gone</span>{/if}
                {#if b.ahead}<span class="ab" title="{b.ahead} to push">↑{b.ahead}</span>{/if}
                {#if b.behind}<span class="ab" title="{b.behind} to pull">↓{b.behind}</span>{/if}
              </button>
            </li>
          {:else}
            <li class="none faint">{filter ? "No matching branches" : "No branches yet — make a first commit"}</li>
          {/each}
        </ul>
        {#if picked.size}
          <div class="pickbar">
            <span>{picked.size} selected</span>
            <button class="btn small danger" onclick={deletePicked}><Icon name="discard" size={12} /> Delete…</button>
            <button class="clear" title="Clear selection (Esc)" aria-label="Clear selection" onclick={() => picked.clear()}><Icon name="x" size={12} /></button>
          </div>
        {/if}
      {/if}
    </section>

    <section>
      <header>
        <button class="head" onclick={() => (open.remotes = !open.remotes)} aria-expanded={open.remotes}>
          <span class="chev" class:open={open.remotes}><Icon name="chevron" size={12} /></span> Remotes
        </button>
        <button class="add" title="Add remote" aria-label="Add remote" onclick={act.addRemote}><Icon name="plus" size={14} /></button>
      </header>
      {#if open.remotes}
        <ul>
          {#each refs?.remotes ?? [] as r (r.name)}
            {@const isOpen = openRemote[r.name] ?? true}
            <li>
              <button class="item remote"
                onclick={() => (openRemote[r.name] = !isOpen)}
                oncontextmenu={(e) => act.remoteMenu(e, r.name)}
                use:dropTarget={{ accepts: (p) => p.kind === "branch", drop: (p, e) => dropOnRemote(p, r.name, e) }}>
                <span class="chev" class:open={isOpen}><Icon name="chevron" size={12} /></span>
                <Icon name="remote" size={14} />
                <span class="name">{r.name}</span>
              </button>
              {#if isOpen}
                <ul class="nested">
                  {#each r.branches.filter((b) => match(b.fullName)) as rb (rb.fullName)}
                    <li>
                      <button class="item"
                        onclick={() => act.showHistory(rb.fullName)}
                        ondblclick={() => act.checkoutRemote(rb.fullName)}
                        oncontextmenu={(e) => act.remoteBranchMenu(e, rb.fullName)}
                        use:draggable={() => ({ payload: { kind: "remote-branch", fullName: rb.fullName }, label: rb.fullName })}>
                        <span class="dot remote"></span>
                        <span class="name">{rb.name}</span>
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}
            </li>
          {:else}
            <li class="none faint">No remotes</li>
          {/each}
        </ul>
      {/if}
    </section>

    <section>
      <header>
        <button class="head" onclick={() => (open.tags = !open.tags)} aria-expanded={open.tags}>
          <span class="chev" class:open={open.tags}><Icon name="chevron" size={12} /></span> Tags
          {#if refs?.tags.length}<span class="faint count">{refs.tags.length}</span>{/if}
        </button>
      </header>
      {#if open.tags}
        <ul>
          {#each (refs?.tags ?? []).filter((t) => match(t.name)) as t (t.name)}
            <li>
              <button class="item" title={t.message ?? t.name} onclick={() => act.selectCommit(t.oid)} oncontextmenu={(e) => act.tagMenu(e, t.name, t.oid)}>
                <Icon name="tag" size={13} />
                <span class="name">{t.name}</span>
              </button>
            </li>
          {:else}
            <li class="none faint">No tags</li>
          {/each}
        </ul>
      {/if}
    </section>

    {#if app.lfs?.used}
      <section>
        <header>
          <button class="head" onclick={() => (open.lfs = !open.lfs)} aria-expanded={open.lfs}>
            <span class="chev" class:open={open.lfs}><Icon name="chevron" size={12} /></span> Git LFS
            <span class="faint count">{app.lfs.files}</span>
          </button>
          <button class="add" title="Track files with LFS" aria-label="Track files with LFS" onclick={() => act.lfsTrack()}><Icon name="plus" size={14} /></button>
        </header>
        {#if open.lfs}
          {#if !app.lfs.installed}
            <p class="lfsnote warn">Git LFS isn't installed, so large files show as small pointer files. Install git-lfs.</p>
          {:else}
            {#if app.lfs.missing}
              <p class="lfsnote">{app.lfs.missing} file{app.lfs.missing === 1 ? " isn't" : "s aren't"} downloaded. <button class="link" onclick={act.lfsPull}>Download</button></p>
            {/if}
            <ul>
              {#each app.lfs.patterns as p (p)}
                <li><button class="item lfs" title="Files matching {p} are stored with LFS" oncontextmenu={(e) => lfsPatternMenu(e, p)}><Icon name="file" size={12} /><span class="name mono">{p}</span></button></li>
              {:else}
                <li class="none faint">No patterns tracked</li>
              {/each}
            </ul>
            <div class="lfslocks">
              {#if locks}
                {#each locks as l (l.id)}
                  <div class="lock" title="Locked by {l.owner?.name ?? 'someone'}">
                    <Icon name="lock" size={12} /><span class="name">{l.path}</span><span class="faint">{l.owner?.name ?? ""}</span>
                    <button class="link" onclick={() => act.lfsLock(l.path, true).then(loadLocks)}>Unlock</button>
                  </div>
                {:else}
                  <p class="lfsnote">No files are locked.</p>
                {/each}
              {:else if locksError}
                <p class="lfsnote" title={locksError}>Locks aren't available for this remote.</p>
              {/if}
              <button class="link" onclick={loadLocks}>{locks ? "Refresh locks" : "Show file locks"}</button>
            </div>
          {/if}
        {/if}
      </section>
    {/if}

    {#if linked.length}
      <section>
        <header>
          <button class="head" onclick={() => (open.worktrees = !open.worktrees)} aria-expanded={open.worktrees}>
            <span class="chev" class:open={open.worktrees}><Icon name="chevron" size={12} /></span> Worktrees
            <span class="faint count">{worktrees.length}</span>
          </button>
          <button class="add" title="New worktree" aria-label="New worktree" onclick={() => act.addWorktree()}><Icon name="plus" size={14} /></button>
        </header>
        {#if open.worktrees}
          <ul>
            {#each worktrees as w (w.path)}
              <li>
                <button class="item wt" class:head={w.isCurrent} class:missing={w.missing} title="{w.path}{w.head ? `\n${w.head} ${w.summary}` : ''}{w.changes ? `\n${w.changes} uncommitted change${w.changes === 1 ? '' : 's'}` : ''}"
                  onclick={() => openWorktree(w)} oncontextmenu={(e) => worktreeMenu(e, w)}>
                  <Icon name="folder" size={13} />
                  <span class="name">{w.branch ?? (w.head ? `${w.head} (detached)` : w.path.split(/[\\/]/).pop())}</span>
                  {#if w.isMain}<span class="tagn">main</span>{/if}
                  {#if w.missing}<span class="gone">missing</span>{:else if w.changes}<span class="ab" title="Uncommitted changes">●{w.changes}</span>{/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/if}

    {#if submodules.length}
      <section>
        <header>
          <button class="head" onclick={() => (open.submodules = !open.submodules)} aria-expanded={open.submodules}>
            <span class="chev" class:open={open.submodules}><Icon name="chevron" size={12} /></span> Submodules
            <span class="faint count">{submodules.length}</span>
          </button>
          <button class="add" title="Update all submodules (clone missing ones, check out the recorded commits)" aria-label="Update all submodules"
            onclick={() => act.updateSubmodules([], "submodules")}><Icon name="sync" size={13} /></button>
        </header>
        {#if open.submodules}
          <ul>
            {#each submodules as s (s.path)}
              <li>
                <button class="item sm" class:off={s.state === "uninitialized"} title={smTitle(s)} onclick={() => openSubmodule(s)} oncontextmenu={(e) => submoduleMenu(e, s)}>
                  <Icon name="remote" size={13} />
                  <span class="name">{s.path}</span>
                  {#if smNote(s)}<span class="gone" class:soft={s.state === "uninitialized"}>{smNote(s)}</span>{/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/if}
  </div>
</nav>

<style>
  .side { display: flex; flex-direction: column; min-height: 0; height: 100%; background: var(--bg); }
  .views { padding: 10px 8px 8px; display: flex; flex-direction: column; gap: 1px; border-bottom: 1px solid var(--line); }
  .nav {
    display: flex; align-items: center; gap: 9px; height: 32px; padding: 0 10px;
    border-radius: var(--radius); color: var(--ink-2); font-weight: 600; text-align: left;
  }
  .nav span:first-of-type { flex: 1; }
  .nav:hover { background: var(--surface-3); color: var(--ink); }
  .nav.on { background: var(--surface); color: var(--ink); box-shadow: 0 0 0 1px var(--line); }
  .badge {
    min-width: 20px; height: 18px; padding: 0 6px; border-radius: 9px; font-size: calc(var(--fs) * 11);
    line-height: 18px; text-align: center; background: var(--accent); color: var(--accent-ink);
  }
  .badge.plain { background: var(--surface-3); color: var(--ink-2); }
  .badge.warn { background: var(--danger); color: #fff; min-width: 18px; padding: 0; }
  .scope { font-weight: 500; font-size: calc(var(--fs) * 11); max-width: 90px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .lists { flex: 1; overflow-y: auto; padding: 6px 8px 24px; }
  .filter {
    display: flex; align-items: center; gap: 6px; padding: 0 8px; margin: 4px 0 6px;
    border: 1px solid var(--line); border-radius: var(--radius); background: var(--surface); color: var(--ink-3);
  }
  .filter input { border: 0; padding: 5px 0; background: transparent; box-shadow: none; }
  section { margin-top: 8px; }
  section header { display: flex; align-items: center; }
  .head {
    flex: 1; display: flex; align-items: center; gap: 4px; padding: 4px 4px; text-align: left;
    color: var(--ink-3); font-size: calc(var(--fs) * 12); font-weight: 700;
  }
  .head:hover { color: var(--ink-2); }
  .count { font-weight: 500; margin-left: 4px; }
  .add { display: grid; place-items: center; width: 22px; height: 22px; border-radius: 4px; color: var(--ink-3); }
  .add:hover { background: var(--surface-3); color: var(--ink); }
  .chev { display: inline-grid; transition: rotate 0.12s; }
  .chev.open { rotate: 90deg; }
  ul { list-style: none; margin: 0; padding: 0; }
  .nested { padding-left: 14px; }
  .item {
    display: flex; align-items: center; gap: 8px; width: 100%; height: 26px; padding: 0 8px 0 10px;
    border-radius: 5px; text-align: left; color: var(--ink);
  }
  .item:hover { background: var(--surface-3); }
  .item.head .name { font-weight: 700; }
  .item.picked, .item.picked:hover { background: var(--sel-strong); }
  .item.shown { background: var(--sel); }
  .pickbar {
    position: sticky; bottom: 0; display: flex; align-items: center; gap: 6px; margin: 4px 0 0; padding: 5px 6px 5px 10px;
    border: 1px solid var(--line); border-radius: var(--radius); background: var(--surface); box-shadow: var(--shadow-pop);
    font-size: calc(var(--fs) * 12); font-weight: 600;
  }
  .pickbar span { flex: 1; }
  .pickbar .clear { display: grid; place-items: center; width: 22px; height: 22px; border-radius: 4px; color: var(--ink-3); }
  .pickbar .clear:hover { background: var(--surface-3); color: var(--ink); }
  .item.remote { color: var(--ink-2); padding-left: 4px; gap: 6px; }
  .name { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .dot { width: 8px; height: 8px; border-radius: 50%; border: 2px solid var(--ink-3); flex: none; }
  .dot.current { background: var(--lane-0); border-color: var(--lane-0); box-shadow: 0 0 0 3px color-mix(in srgb, var(--lane-0) 25%, transparent); }
  .dot.remote { border-style: dashed; border-width: 1.5px; }
  .ab { font-size: calc(var(--fs) * 11); color: var(--ink-2); font-variant-numeric: tabular-nums; }
  .gone { font-size: calc(var(--fs) * 10); color: var(--warn); border: 1px solid currentColor; border-radius: 3px; padding: 0 3px; }
  .none { padding: 4px 10px; font-size: calc(var(--fs) * 12); }
  .stem { width: 8px; flex: none; color: var(--ink-3); font-size: calc(var(--fs) * 11); line-height: 1; display: grid; place-items: center; }
  .item.head .stem { color: var(--lane-0); font-weight: 700; }
  .restack { font-size: calc(var(--fs) * 10); color: var(--accent); border: 1px solid currentColor; border-radius: 3px; padding: 0 4px; cursor: pointer; }
  .restack:hover { background: var(--accent); color: var(--accent-ink); }
  .item.wt, .item.sm { gap: 7px; color: var(--ink-2); }
  .item.wt .name, .item.sm .name { color: var(--ink); }
  .item.missing .name, .item.off .name { color: var(--ink-3); }
  .tagn { font-size: calc(var(--fs) * 10); color: var(--ink-3); border: 1px solid var(--line); border-radius: 3px; padding: 0 3px; }
  .gone.soft { color: var(--ink-3); }
  .pin { width: 12px; flex: none; display: grid; color: var(--ink-3); }
  .pin.current { color: var(--lane-0); }
  .item.lfs { gap: 7px; color: var(--ink-3); }
  .item.lfs .name { color: var(--ink-2); font-size: calc(var(--cs) * 12); }
  .lfsnote { margin: 2px 10px 6px; font-size: calc(var(--fs) * 11.5); color: var(--ink-3); }
  .lfsnote.warn { color: var(--warn); }
  .lfslocks { padding: 2px 10px 6px; font-size: calc(var(--fs) * 12); display: flex; flex-direction: column; gap: 3px; align-items: flex-start; }
  .lock { display: flex; gap: 6px; width: 100%; align-items: center; }
  .lock .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
