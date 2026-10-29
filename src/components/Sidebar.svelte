<script lang="ts">
  import * as act from "../lib/actions";
  import { draggable, dropTarget, type Payload } from "../lib/drag.svelte";
  import { app, type View } from "../lib/state.svelte";
  import { menu } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";

  let filter = $state("");
  // Expanded sections are part of the saved layout.
  const open = $derived(app.layout.sections);
  const openRemote = $derived(app.layout.remotes);

  const st = $derived(app.status);
  const refs = $derived(app.refs);
  const changeCount = $derived((st?.staged.length ?? 0) + (st?.unstaged.length ?? 0) + (st?.conflicted.length ?? 0));
  const match = (s: string) => !filter || s.toLowerCase().includes(filter.toLowerCase());
  const locals = $derived((refs?.local ?? []).filter((b) => match(b.name)));
  const total = $derived((refs?.local.length ?? 0) + (refs?.remotes.reduce((n, r) => n + r.branches.length, 0) ?? 0));

  function go(v: View) {
    if (v === "history" && app.view === "history" && app.scope !== "all") {
      app.scope = "all";
      app.loadLog();
    }
    app.view = v;
    if (v === "history" && !app.log.length) app.loadLog();
  }

  function dropOnRemote(p: Payload, remote: string, e: PointerEvent) {
    if (p.kind !== "branch") return;
    menu.open(e.clientX, e.clientY, [
      { label: `Push ${p.name} to ${remote}`, action: () => act.push(p.name, remote) },
    ]);
  }

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
  </div>

  <div class="lists">
    {#if total > 12}
      <div class="filter">
        <Icon name="search" size={14} />
        <input placeholder="Filter branches" bind:value={filter} spellcheck="false" />
      </div>
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
          {#each locals as b (b.name)}
            <li>
              <button class="item" class:head={b.isHead}
                onclick={() => act.showHistory(b.name)}
                ondblclick={() => !b.isHead && act.checkout(b.name)}
                oncontextmenu={(e) => act.branchMenu(e, b.name)}
                use:draggable={() => ({ payload: { kind: "branch", name: b.name }, label: b.name })}
                use:dropTarget={{ accepts: act.acceptsOnBranch(b.name, b.isHead), drop: (p, e) => act.dropOnBranch(p, b.name, e) }}>
                <span class="dot" class:current={b.isHead}></span>
                <span class="name">{b.name}</span>
                {#if b.upstreamGone}<span class="gone" title="The remote branch was deleted">gone</span>{/if}
                {#if b.ahead}<span class="ab" title="{b.ahead} to push">↑{b.ahead}</span>{/if}
                {#if b.behind}<span class="ab" title="{b.behind} to pull">↓{b.behind}</span>{/if}
              </button>
            </li>
          {:else}
            <li class="none faint">{filter ? "No matching branches" : "No branches yet — make a first commit"}</li>
          {/each}
        </ul>
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
  .item.remote { color: var(--ink-2); padding-left: 4px; gap: 6px; }
  .name { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .dot { width: 8px; height: 8px; border-radius: 50%; border: 2px solid var(--ink-3); flex: none; }
  .dot.current { background: var(--lane-0); border-color: var(--lane-0); box-shadow: 0 0 0 3px color-mix(in srgb, var(--lane-0) 25%, transparent); }
  .dot.remote { border-style: dashed; border-width: 1.5px; }
  .ab { font-size: calc(var(--fs) * 11); color: var(--ink-2); font-variant-numeric: tabular-nums; }
  .gone { font-size: calc(var(--fs) * 10); color: var(--warn); border: 1px solid currentColor; border-radius: 3px; padding: 0 3px; }
  .none { padding: 4px 10px; font-size: calc(var(--fs) * 12); }
</style>
