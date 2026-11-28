<script lang="ts">
  // Pull, like Tower's Pull sheet: pick the branch to pull from (the
  // upstream by default, or any other remote branch), merge or rebase,
  // and whether to set uncommitted changes aside for it. The two choices
  // draw what the history will look like, from the real commit counts.
  import { api } from "../lib/api";
  import * as act from "../lib/actions";
  import { app } from "../lib/state.svelte";
  import { dialog, menu } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";
  import { tick } from "svelte";

  let { onclose }: { onclose: () => void } = $props();

  const into = $derived(app.headBranch ?? "");
  const remotes = $derived(app.refs?.remotes ?? []);
  const upstream = $derived(app.status?.upstream ?? null);
  /** Tracked files with uncommitted changes (what a merge or rebase needs out of the way). */
  const changed = $derived((app.status?.staged.length ?? 0) + (app.status?.unstaged.filter((f) => f.status !== "untracked").length ?? 0));

  /** Every remote branch, as "origin/feature". */
  const all = $derived(remotes.flatMap((r) => r.branches.filter((b) => b.name !== "HEAD").map((b) => ({ remote: r.name, name: b.name, full: b.fullName }))));

  /** Where to start: what was asked for, else the upstream, else the same name on a remote, else its main branch. */
  function initial(): string {
    const want = app.pullDialog?.from || upstream;
    if (want && all.some((b) => b.full === want)) return want;
    const origin = all.some((b) => b.remote === "origin") ? "origin" : (remotes[0]?.name ?? "");
    const on = all.filter((b) => b.remote === origin);
    return (on.find((b) => b.name === into) ?? on.find((b) => b.name === "main") ?? on.find((b) => b.name === "master") ?? on[0])?.full ?? "";
  }

  let from = $state(initial());
  let rebase = $state(app.pullDialog?.rebase ?? app.settings.pullRebase);
  let autostash = $state(app.settings.pullAutostash);
  let track = $state(false);
  let counts = $state<{ incoming: number; outgoing: number } | null>(null);
  /** The quiet fetch started on open, so the counts and branches are current. */
  let fetching = $state<Promise<void> | null>(null);
  let fetchedAt = $state(0);

  // The branch picker.
  let picking = $state(false);
  let query = $state("");
  let active = $state(0);
  let search = $state<HTMLInputElement>();
  let pickButton = $state<HTMLButtonElement>();
  let pullButton = $state<HTMLButtonElement>();
  let list = $state<HTMLDivElement>();

  const matches = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const found = q ? all.filter((b) => b.full.toLowerCase().includes(q)) : all;
    // The upstream first, then as the remotes list them.
    return [...found].sort((a, b) => Number(b.full === upstream) - Number(a.full === upstream));
  });

  const showActive = () => tick().then(() => list?.querySelector(".on")?.scrollIntoView({ block: "nearest" }));
  async function openPicker() {
    query = "";
    picking = true;
    await tick();
    active = Math.max(0, matches.findIndex((b) => b.full === from));
    search?.focus();
    showActive();
  }
  function closePicker() {
    picking = false;
    pickButton?.focus();
  }
  function choose(full: string) {
    from = full;
    closePicker();
  }
  function pickerKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      active = Math.max(0, Math.min(matches.length - 1, active + (e.key === "ArrowDown" ? 1 : -1)));
      showActive();
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (matches[active]) choose(matches[active].full);
    } else if (e.key === "Tab") {
      picking = false;
    }
  }

  // Fetch the chosen branch's remote, once, without asking for a password
  // (one that needs it just isn't fetched here; Pull asks).
  const fetchedRemotes = new Set<string>();
  $effect(() => {
    const remote = all.find((b) => b.full === from)?.remote;
    if (!remote || fetchedRemotes.has(remote) || !app.path || !into) return;
    fetchedRemotes.add(remote);
    const path = app.path;
    const p: Promise<void> = api.fetch(path, remote, false)
      .then(async () => {
        app.noteFetched(path);
        if (app.path === path) await app.refresh();
        fetchedAt = Date.now();
      })
      .catch(() => {})
      .finally(() => {
        if (fetching === p) fetching = null;
      });
    fetching = p;
  });

  // What the pull would bring in, and what the branch has of its own.
  $effect(() => {
    const f = from;
    void fetchedAt;
    if (!f || !app.path || !into) return void (counts = null);
    const path = app.path;
    api.betweenHeadAnd(path, f).then((c) => {
      if (from === f && app.path === path) counts = c;
    }).catch(() => {
      if (from === f) counts = null;
    });
  });

  $effect(() => {
    pullButton?.focus();
  });

  const fastForward = $derived(!!counts && counts.incoming > 0 && counts.outgoing === 0);
  const upToDate = $derived(!!counts && counts.incoming === 0);
  const choosing = $derived(!counts || (!fastForward && !upToDate));
  const canPull = $derived(!!from && !!into && !app.busy);

  async function pull(e?: Event) {
    e?.preventDefault();
    if (!canPull || picking) return;
    if (changed && autostash !== app.settings.pullAutostash) {
      app.settings.pullAutostash = autostash;
      app.saveSettings();
    }
    const opts = { from, rebase, autostash: changed ? autostash : false, track: track && !upstream };
    const pending = fetching;
    onclose();
    // Never two fetches of one repository at once.
    if (pending) await pending;
    await act.pull(opts);
  }

  // Escape closes the branch list first, then the dialog, wherever the
  // focus is (menus and dialogs on top take theirs first).
  function key(e: KeyboardEvent) {
    if (e.key !== "Escape" || e.defaultPrevented || menu.items.length || dialog.spec) return;
    e.preventDefault();
    if (picking) closePicker();
    else onclose();
  }

  const n = (k: number, one: string, many = one + "s") => `${k} ${k === 1 ? one : many}`;
  const split = (full: string) => {
    const r = remotes.filter((x) => full.startsWith(x.name + "/")).sort((a, b) => b.name.length - a.name.length)[0]?.name ?? "";
    return { remote: r, name: r ? full.slice(r.length + 1) : full };
  };

  // The little graphs: the shared commit on the left, time going right.
  const W = 216, H = 56, X0 = 10, TOP = 13, BOT = 41;
  /** Up to 4 dots for a count (while counting: 2 in, 1 out). */
  const dots = (k: number | undefined, fallback: number) => Math.min(4, Math.max(1, k ?? fallback));
</script>

<svelte:window onkeydown={key} />
<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="backdrop" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
  <form class="sheet" role="dialog" aria-modal="true" aria-labelledby="pd-title" onsubmit={pull}>
    <header>
      <div>
        <h2 id="pd-title">{into ? `Pull into ${into}` : "Pull"}</h2>
        <p class="sub">{into ? "Fetch a branch and bring its new commits into this one." : "Check out a branch first: commits are pulled into the branch you're on."}</p>
      </div>
      <button type="button" class="close" aria-label="Close" onclick={onclose}><Icon name="x" size={14} /></button>
    </header>

    {#if !remotes.length}
      <p class="empty">This repository has no remotes yet. Add one in Repository settings, under Remotes, to pull from it.</p>
    {:else if into}
      <div class="source">
        <span class="lbl" id="pd-from">From</span>
        <div class="picker">
          <button type="button" class="pick" bind:this={pickButton} aria-haspopup="listbox" aria-expanded={picking} aria-labelledby="pd-from"
            onclick={() => (picking ? closePicker() : openPicker())}>
            <Icon name="remote" size={14} />
            {#if from}
              {@const s = split(from)}
              <span class="ref"><span class="remote">{s.remote}/</span>{s.name}</span>
              {#if from === upstream}<span class="tag">upstream</span>{/if}
            {:else}
              <span class="ref faint">Choose a branch</span>
            {/if}
            <span class="caret"><Icon name="chevron" size={12} /></span>
          </button>
          {#if picking}
            <div class="pop">
              <input bind:this={search} bind:value={query} oninput={() => (active = 0)} onkeydown={pickerKey}
                placeholder="Find a branch" aria-label="Find a branch" spellcheck="false" autocomplete="off" />
              <div class="list" role="listbox" aria-label="Remote branches" bind:this={list}>
                {#each matches as b, i (b.full)}
                  <button type="button" role="option" aria-selected={b.full === from} class:on={i === active}
                    onmousemove={() => (active = i)} onclick={() => choose(b.full)}>
                    <span class="ref"><span class="remote">{b.remote}/</span>{b.name}</span>
                    {#if b.full === upstream}<span class="tag">upstream</span>{/if}
                    {#if b.full === from}<span class="tick"><Icon name="check" size={13} /></span>{/if}
                  </button>
                {:else}
                  <p class="none">No remote branch matches “{query}”.</p>
                {/each}
              </div>
            </div>
          {/if}
        </div>
      </div>

      <p class="status" aria-live="polite">
        {#if counts}
          {#if upToDate}
            {into} already has every commit on {from}{counts.outgoing ? `, plus ${n(counts.outgoing, "commit")} of its own` : ""}.
          {:else}
            <b class="in">{n(counts.incoming, "new commit")}</b> to bring in{#if counts.outgoing}; <b class="out">{counts.outgoing}</b> on {into} {counts.outgoing === 1 ? "isn't" : "aren't"} on {from}{/if}.
          {/if}
        {:else if from}
          Counting commits…
        {/if}
        {#if fetching}<span class="fetch"><span class="spin" aria-hidden="true"></span>Checking {split(from).remote} for newer commits</span>{/if}
      </p>

      {#if choosing}
        {@const inN = dots(counts?.incoming, 2)}
        {@const outN = dots(counts?.outgoing, 1)}
        <!-- One spacing for both drawings, as wide as the longer one allows. -->
        {@const STEP = Math.min(50, (W - 2 * X0) / (inN + outN))}
        {@const mx = X0 + (Math.max(inN, outN) + 1) * STEP}
        <div class="choices" class:guess={!counts} role="radiogroup" aria-label="How to bring the commits in">
          <button type="button" role="radio" aria-checked={!rebase} class="choice" class:on={!rebase} onclick={() => (rebase = false)}>
            <svg viewBox="0 0 {W} {H}" aria-hidden="true">
              <path class="edge in" d="M{X0} {BOT} C{X0 + STEP * 0.6} {BOT} {X0 + STEP * 0.4} {TOP} {X0 + STEP} {TOP} H{X0 + inN * STEP} C{mx - STEP * 0.4} {TOP} {mx - STEP * 0.6} {BOT} {mx} {BOT}" />
              <path class="edge out" d="M{X0} {BOT} H{mx}" />
              <circle class="dot base" cx={X0} cy={BOT} r="4.5" />
              {#each Array(inN) as _, i}<circle class="dot in" cx={X0 + (i + 1) * STEP} cy={TOP} r="4.5" />{/each}
              {#each Array(outN) as _, i}<circle class="dot out" cx={X0 + (i + 1) * STEP} cy={BOT} r="4.5" />{/each}
              <circle class="dot merge" cx={mx} cy={BOT} r="6.5" />
            </svg>
            <span class="name">Merge</span>
            <span class="what">Keeps both lines of work and joins them with a merge commit.</span>
          </button>
          <button type="button" role="radio" aria-checked={rebase} class="choice" class:on={rebase} onclick={() => (rebase = true)}>
            <svg viewBox="0 0 {W} {H}" aria-hidden="true">
              <path class="edge in" d="M{X0} {BOT} H{X0 + inN * STEP}" />
              <path class="edge out" d="M{X0 + inN * STEP} {BOT} H{X0 + (inN + outN) * STEP}" />
              <circle class="dot base" cx={X0} cy={BOT} r="4.5" />
              {#each Array(inN) as _, i}<circle class="dot in" cx={X0 + (i + 1) * STEP} cy={BOT} r="4.5" />{/each}
              {#each Array(outN) as _, i}<circle class="dot moved" cx={X0 + (inN + i + 1) * STEP} cy={BOT} r="4.5" />{/each}
            </svg>
            <span class="name">Rebase</span>
            <span class="what">Moves {counts?.outgoing ? `${into}'s ${n(counts.outgoing, "commit")}` : `${into}'s commits`} on top, in one straight line. They get new ids.</span>
          </button>
        </div>
      {:else if fastForward}
        <p class="single"><Icon name="check" size={14} /><span>{into} has nothing of its own since, so it just moves up to {from}. No merge commit, nothing rewritten.</span></p>
      {/if}

      {#if changed || !upstream}
        <div class="options">
          {#if changed}
            <label class="switch">
              <input type="checkbox" bind:checked={autostash} />
              <span class="track" aria-hidden="true"></span>
              <span class="text">Set my uncommitted changes aside, and put them back after<small>{n(changed, "changed file")}. Without this, the pull stops if they're in the way.</small></span>
            </label>
          {/if}
          {#if !upstream}
            <label class="switch">
              <input type="checkbox" bind:checked={track} />
              <span class="track" aria-hidden="true"></span>
              <span class="text">Track {from || "this branch"} from now on<small>{into} has no upstream yet; Pull and Push would use this one.</small></span>
            </label>
          {/if}
        </div>
      {/if}
    {/if}

    <footer>
      <button type="button" class="btn" onclick={onclose}>Cancel</button>
      <button type="submit" class="btn primary" bind:this={pullButton} disabled={!canPull || !remotes.length}>
        {!counts || fastForward || upToDate ? "Pull" : rebase ? "Pull with rebase" : "Pull and merge"}
      </button>
    </footer>
  </form>
</div>

<style>
  .backdrop {
    position: fixed; inset: 0; z-index: 95; display: grid; place-items: start center;
    padding: 11vh 16px 16px; background: var(--backdrop);
  }
  .sheet {
    width: min(540px, 100%); background: var(--surface); border-radius: 12px; border: 1px solid var(--line);
    box-shadow: var(--shadow-pop); display: flex; flex-direction: column; animation: rise 0.14s ease-out;
  }
  @keyframes rise { from { transform: translateY(6px); opacity: 0; } }

  header { display: flex; align-items: flex-start; gap: 12px; padding: 18px 20px 2px; }
  header > div { flex: 1; min-width: 0; }
  h2 { margin: 0; font-size: calc(var(--fs) * 17); font-weight: 700; overflow-wrap: anywhere; }
  .sub { margin: 3px 0 0; color: var(--ink-3); font-size: calc(var(--fs) * 12.5); }
  .close { display: grid; place-items: center; width: 28px; height: 28px; border-radius: 7px; color: var(--ink-3); margin: -4px -8px 0 0; flex: none; }
  .close:hover { background: var(--surface-3); color: var(--ink); }
  .empty { margin: 12px 20px 0; color: var(--ink-2); line-height: 1.5; }

  /* From: a branch chip that opens a searchable list. */
  .source { display: flex; align-items: center; gap: 12px; padding: 16px 20px 0; }
  .lbl { width: 40px; flex: none; color: var(--ink-3); font-size: calc(var(--fs) * 12.5); }
  .picker { position: relative; flex: 1; min-width: 0; }
  .pick {
    display: flex; align-items: center; gap: 8px; width: 100%; height: 36px; padding: 0 10px 0 12px;
    border: 1px solid var(--line); border-radius: 8px; background: var(--surface-2); color: var(--ink-3); text-align: left;
  }
  .pick:hover { border-color: var(--ink-3); }
  .pick[aria-expanded="true"] { border-color: var(--focus); box-shadow: 0 0 0 3px var(--accent-soft); }
  .caret { margin-left: auto; display: grid; rotate: 90deg; color: var(--ink-3); flex: none; }
  .pick[aria-expanded="true"] .caret { rotate: -90deg; }
  .ref { font-family: var(--font-mono); font-size: calc(var(--cs) * 12.5); font-variant-ligatures: none; color: var(--ink); min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .remote { color: var(--ink-3); }
  .tag { flex: none; font-size: calc(var(--fs) * 11); color: var(--accent); background: var(--accent-soft); border-radius: 4px; padding: 1px 6px; font-weight: 600; }
  .pop {
    position: absolute; z-index: 2; top: calc(100% + 6px); left: 0; right: 0; background: var(--surface);
    border: 1px solid var(--line); border-radius: 9px; box-shadow: var(--shadow-pop); overflow: hidden;
  }
  .pop input { border: 0; border-bottom: 1px solid var(--line-soft); border-radius: 0; box-shadow: none; width: 100%; padding: 10px 12px; background: transparent; }
  .list { max-height: 248px; overflow-y: auto; padding: 4px; }
  .list button { display: flex; align-items: center; gap: 8px; width: 100%; padding: 6px 8px; border-radius: 6px; text-align: left; }
  .list button.on { background: var(--sel); }
  .tick { margin-left: auto; display: grid; color: var(--accent); flex: none; }
  .none { margin: 0; padding: 10px 8px; color: var(--ink-3); }

  .status { margin: 10px 20px 0 72px; color: var(--ink-2); font-size: calc(var(--fs) * 12.5); line-height: 1.5; min-height: 1.5em; }
  .status b.in { color: var(--lane-3); }
  .status b.out { color: var(--accent); }
  .fetch { display: flex; align-items: center; gap: 6px; color: var(--ink-3); font-size: calc(var(--fs) * 11.5); }
  .spin { width: 9px; height: 9px; border-radius: 50%; border: 1.5px solid var(--line); border-top-color: var(--ink-3); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { rotate: 360deg; } }

  /* Merge or rebase, each drawn as the history it leaves. */
  .choices { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; padding: 16px 20px 0; }
  .choice {
    display: flex; flex-direction: column; align-items: flex-start; gap: 3px; padding: 12px 14px 13px; text-align: left;
    border: 1px solid var(--line); border-radius: 10px; background: var(--surface); color: var(--ink);
  }
  .choice:hover:not(.on) { border-color: var(--ink-3); }
  .choice.on { border-color: var(--accent); background: var(--accent-soft); box-shadow: inset 0 0 0 1px var(--accent); }
  .choice:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
  .choice svg { width: 100%; height: auto; margin: 2px 0 8px; overflow: visible; }
  .name { font-weight: 700; font-size: calc(var(--fs) * 13.5); }
  .what { color: var(--ink-2); font-size: calc(var(--fs) * 12); line-height: 1.45; }
  .guess svg { opacity: 0.4; }
  .edge { fill: none; stroke-width: 2.2; stroke-linecap: round; }
  .edge.in { stroke: var(--lane-3); }
  .edge.out { stroke: var(--accent); }
  .dot { stroke-width: 2; }
  .dot.base { fill: var(--surface); stroke: var(--ink-3); }
  .dot.in { fill: var(--lane-3); stroke: var(--lane-3); }
  .dot.out { fill: var(--accent); stroke: var(--accent); }
  /* Rebased commits: the same work, new commits. */
  .dot.moved { fill: var(--surface); stroke: var(--accent); stroke-width: 2.4; }
  .dot.merge { fill: var(--surface); stroke: var(--accent); stroke-width: 2.6; }

  .single { display: flex; gap: 8px; align-items: flex-start; margin: 14px 20px 0; padding: 10px 12px; border-radius: 9px; background: var(--surface-2); color: var(--ink-2); line-height: 1.45; }
  .single :global(svg) { color: var(--accent); flex: none; margin-top: 3px; }

  /* Options as switches. */
  .options { display: flex; flex-direction: column; gap: 12px; padding: 18px 20px 0; }
  .switch { position: relative; display: flex; align-items: flex-start; gap: 10px; }
  .switch input { position: absolute; opacity: 0; width: 1px; height: 1px; }
  .track {
    flex: none; position: relative; width: 30px; height: 18px; margin-top: 1px; border-radius: 9px;
    background: var(--surface-3); border: 1px solid var(--line); transition: background 0.15s;
  }
  .track::after {
    content: ""; position: absolute; top: 2px; left: 2px; width: 12px; height: 12px; border-radius: 50%;
    background: var(--surface); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.25); transition: translate 0.15s;
  }
  .switch input:checked + .track { background: var(--accent); border-color: var(--accent); }
  .switch input:checked + .track::after { translate: 12px 0; }
  .switch input:focus-visible + .track { outline: 2px solid var(--focus); outline-offset: 2px; }
  .text { display: flex; flex-direction: column; gap: 2px; font-size: calc(var(--fs) * 13); line-height: 1.4; }
  .text small { color: var(--ink-3); font-size: calc(var(--fs) * 12); }

  footer { display: flex; justify-content: flex-end; gap: 8px; padding: 20px 20px 16px; }
  footer .primary { min-width: 128px; }

  @media (prefers-reduced-motion: reduce) {
    .sheet, .spin { animation: none; }
    .track, .track::after { transition: none; }
  }
  @media (max-width: 520px) {
    .choices { grid-template-columns: 1fr; }
    .status { margin-left: 20px; }
  }
</style>
