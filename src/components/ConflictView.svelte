<script lang="ts">
  // The conflict wizard: every conflict in a file side by side, mine against
  // theirs (and the common base when there is one). Pick a side, both, or
  // write the result yourself, then save and mark the file resolved.
  import { api, type ConflictFile } from "../lib/api";
  import { chosen, compose, parseConflicts, type Block, type Choice, type Parsed } from "../lib/conflicts";
  import { highlightLines, languageFor } from "../lib/highlight";
  import { app } from "../lib/state.svelte";
  import { toasts } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";
  import { SvelteSet } from "svelte/reactivity";

  let { path, onresolved }: { path: string; onresolved?: () => void } = $props();

  let file = $state.raw<ConflictFile | null>(null);
  let parsed = $state.raw<Parsed | null>(null);
  /** True when the parse is of the fresh merge rather than the file on disk. */
  let fresh = $state(false);
  let choices = $state<(Choice | null)[]>([]);
  let editing = $state<number | null>(null);
  let draft = $state("");
  let showBase = $state(false);
  let saving = $state(false);
  let loadError = $state<string | null>(null);
  const expanded = new SvelteSet<number>();
  let scroller = $state<HTMLDivElement>();

  function start(f: ConflictFile, useMerge: boolean) {
    const disk = !useMerge && f.text != null ? parseConflicts(f.text) : null;
    // Work from the file on disk when it still has markers (it may be half
    // resolved already); otherwise from a fresh merge of the three versions.
    if (disk && disk.conflicts.length) {
      parsed = disk;
      fresh = false;
    } else if (f.merged != null) {
      parsed = parseConflicts(f.merged);
      fresh = true;
    } else {
      parsed = disk;
      fresh = false;
    }
    choices = parsed ? parsed.conflicts.map(() => null) : [];
    editing = null;
    expanded.clear();
    showBase = false;
  }

  async function load() {
    const p = path;
    loadError = null;
    try {
      const f = await api.conflictFile(app.path, p);
      if (p !== path) return;
      file = f;
      start(f, false);
    } catch (e) {
      file = null;
      parsed = null;
      loadError = String(e);
    }
  }

  // Load when the file changes; not on every refresh, which would throw away choices.
  $effect(() => {
    void path;
    file = null;
    parsed = null;
    load();
  });

  const lang = $derived(app.settings.syntax ? languageFor(path) : null);
  const blocks = $derived(parsed ? parsed.conflicts.map((i) => (parsed!.segments[i] as { block: Block }).block) : []);
  const done = $derived(choices.filter(Boolean).length);
  const total = $derived(blocks.length);
  const hasBase = $derived(blocks.some((b) => b.base));
  const sideOf = (n: number, side: "ours" | "theirs") => {
    const c = choices[n];
    if (!c || "edit" in c) return 0;
    if (c.take === side) return 1;
    if (c.take === "ours+theirs") return side === "ours" ? 1 : 2;
    if (c.take === "theirs+ours") return side === "theirs" ? 1 : 2;
    return 0;
  };

  function pick(n: number, c: Choice | null) {
    choices[n] = c;
    if (editing === n) editing = null;
  }
  // Clicking a side adds it to (or removes it from) the result.
  function toggle(n: number, side: "ours" | "theirs") {
    const other = side === "ours" ? "theirs" : "ours";
    const has = sideOf(n, side) > 0, hasOther = sideOf(n, other) > 0;
    if (has) pick(n, hasOther ? { take: other } : null);
    else pick(n, hasOther ? { take: `${other}+${side}` as "ours+theirs" } : { take: side });
  }
  function edit(n: number) {
    const c = choices[n];
    draft = (c ? chosen(blocks[n], c) : [...blocks[n].ours, ...blocks[n].theirs]).join("\n");
    editing = n;
  }
  function saveEdit(n: number) {
    pick(n, { edit: draft === "" ? [] : draft.split(/\r?\n/) });
  }
  function all(take: "ours" | "theirs") {
    choices = blocks.map(() => ({ take }));
    editing = null;
  }

  function jump(n: number) {
    scroller?.querySelector<HTMLElement>(`[data-conflict="${n}"]`)?.scrollIntoView({ block: "start", behavior: "smooth" });
  }
  function nextOpen() {
    const n = choices.findIndex((c) => !c);
    if (n >= 0) jump(n);
  }

  async function save() {
    if (!parsed || done < total) return;
    saving = true;
    try {
      await app.run("Resolving", () => api.resolveWithContent(app.path, path, compose(parsed!, choices)), { success: `Resolved ${path.split("/").pop()}.` });
      onresolved?.();
    } catch {
      // The toast shows what went wrong.
    } finally {
      saving = false;
    }
  }
  async function markResolved() {
    saving = true;
    try {
      await app.run("Resolving", () => api.stageFiles(app.path, [path]), { success: `Marked ${path.split("/").pop()} resolved.` });
      onresolved?.();
    } catch {
      // shown
    } finally {
      saving = false;
    }
  }
  async function takeSide(side: "ours" | "theirs") {
    try {
      await app.run("Resolving", () => api.resolveConflict(app.path, path, side));
      onresolved?.();
    } catch {
      // shown
    }
  }

  const code = (lines: string[]) => (lang ? highlightLines(lines, lang) : lines.map(() => null));
  const CONTEXT = 3;
  const opName = $derived(file?.operation === "stash" ? "the stash" : file?.theirsLabel ?? "theirs");

  function onkey(e: KeyboardEvent) {
    if (editing !== null || (e.target as HTMLElement).closest("textarea, input")) return;
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") { e.preventDefault(); save(); }
  }
</script>

<svelte:window onkeydown={onkey} />

{#snippet lines(ls: string[], cls = "")}
  {@const html = code(ls)}
  {#each ls as l, i}
    <div class="cl {cls}">{#if html[i] != null}{@html html[i]}{:else}{l}{/if}{#if l === ""}&#8203;{/if}</div>
  {/each}
{/snippet}

<div class="cv">
  {#if loadError}
    <div class="empty"><div><strong>Can't open this conflict</strong>{loadError}</div></div>
  {:else if !file}
    <div class="empty"><div><span class="spin"></span> Loading…</div></div>
  {:else if file.binary || !file.hasOurs || !file.hasTheirs || (!parsed && file.text == null)}
    <!-- Nothing to merge line by line: one side deleted the file, or it's binary. -->
    <div class="whole">
      <h3>{file.binary ? "Binary file in conflict" : !file.hasOurs ? `Deleted on ${file.oursLabel}, changed on ${opName}` : !file.hasTheirs ? `Changed on ${file.oursLabel}, deleted on ${opName}` : "Conflict"}</h3>
      <p class="faint">Keep one version of the whole file.</p>
      <div class="cards">
        <button class="card mine" onclick={() => takeSide("ours")}>
          <span class="who">Mine · {file.oursLabel}</span>
          <strong>{file.hasOurs ? "Keep my version" : "Delete the file"}</strong>
        </button>
        <button class="card theirs" onclick={() => takeSide("theirs")}>
          <span class="who">Theirs · {opName}</span>
          <strong>{file.hasTheirs ? "Take their version" : "Delete the file"}</strong>
        </button>
      </div>
    </div>
  {:else if parsed && !total}
    <div class="empty">
      <div>
        <strong>No conflict markers left</strong>
        The file looks resolved. Mark it resolved to stage it, or start over from the original conflict.
        <div class="row">
          <button class="btn small" onclick={() => file && start(file, true)} disabled={!file.merged}>Start over</button>
          <button class="btn small primary" onclick={markResolved} disabled={saving}>Mark as resolved</button>
        </div>
      </div>
    </div>
  {:else if parsed}
    <div class="bar">
      <div class="progress" title="{done} of {total} conflicts resolved">
        <span class="meter"><span style="width: {(done / total) * 100}%"></span></span>
        <span><b>{done}</b> of {total} resolved</span>
      </div>
      <div class="nav">
        {#each blocks as _, n}
          <button class="dot" class:ok={choices[n]} title="Conflict {n + 1}" aria-label="Go to conflict {n + 1}" onclick={() => jump(n)}>{n + 1}</button>
        {/each}
      </div>
      <span class="sp"></span>
      {#if hasBase}
        <label class="chk"><input type="checkbox" bind:checked={showBase} /> Show base</label>
      {/if}
      <button class="btn small ghost" onclick={() => all("ours")} title="Use my side for every conflict">All mine</button>
      <button class="btn small ghost" onclick={() => all("theirs")} title="Use their side for every conflict">All theirs</button>
      <button class="btn small ghost" onclick={() => file && start(file, true)} disabled={!file.merged} title="Forget all choices and edits, and merge the three versions again">Start over</button>
      {#if done < total}
        <button class="btn small primary" onclick={nextOpen}>Next conflict <Icon name="down" size={12} /></button>
      {:else}
        <button class="btn small primary" onclick={save} disabled={saving} title="Ctrl+Enter">Save and mark resolved</button>
      {/if}
    </div>
    {#if fresh && file.text != null}
      <div class="note"><Icon name="warning" size={13} /> The file on disk has no conflict markers left, so this starts from a fresh merge. Saving replaces your edits.</div>
    {/if}

    <div class="doc mono" bind:this={scroller}>
      {#each parsed.segments as seg, s}
        {#if seg.kind === "text"}
          {@const ls = seg.lines}
          {@const first = s === 0}
          {@const last = s === parsed.segments.length - 1}
          {#if expanded.has(s) || ls.length <= CONTEXT * 2 + 1}
            <div class="ctx">{@render lines(ls)}</div>
          {:else}
            <div class="ctx">
              {#if !first}{@render lines(ls.slice(0, CONTEXT))}{/if}
              <button class="fold" onclick={() => expanded.add(s)}>⋯ {(ls.length - (first || last ? CONTEXT : CONTEXT * 2)).toLocaleString()} unchanged lines</button>
              {#if !last}{@render lines(ls.slice(-CONTEXT))}{/if}
            </div>
          {/if}
        {:else}
          {@const n = parsed.conflicts.indexOf(s)}
          {@const b = seg.block}
          {@const c = choices[n]}
          <section class="conflict" class:resolved={c} data-conflict={n}>
            <header>
              <span class="ttl">Conflict {n + 1} <span class="faint">of {total}</span></span>
              {#if c}<span class="badge"><Icon name="check" size={11} /> {"edit" in c ? "Edited" : c.take === "ours" ? "Mine" : c.take === "theirs" ? "Theirs" : c.take === "base" ? "Base" : c.take === "ours+theirs" ? "Mine, then theirs" : "Theirs, then mine"}</span>{/if}
              <span class="sp"></span>
              <div class="seg" role="group" aria-label="Resolution">
                <button class:on={c && !("edit" in c) && c.take === "ours"} onclick={() => pick(n, { take: "ours" })}>Mine</button>
                <button class:on={c && !("edit" in c) && c.take === "theirs"} onclick={() => pick(n, { take: "theirs" })}>Theirs</button>
                <button class:on={c && !("edit" in c) && c.take === "ours+theirs"} onclick={() => pick(n, { take: "ours+theirs" })} title="My lines, then theirs">Both</button>
                <button class:on={c && !("edit" in c) && c.take === "theirs+ours"} onclick={() => pick(n, { take: "theirs+ours" })} title="Their lines, then mine">Both ⇅</button>
                {#if b.base}<button class:on={c && !("edit" in c) && c.take === "base"} onclick={() => pick(n, { take: "base" })} title="Neither change: keep the original">Base</button>{/if}
                <button class:on={editing === n || (c && "edit" in c)} onclick={() => edit(n)}>Edit…</button>
              </div>
              {#if c}<button class="btn small ghost" onclick={() => pick(n, null)} title="Undecide">Reset</button>{/if}
            </header>
            <div class="sides" class:three={showBase && b.base}>
              {#if showBase && b.base}
                <div class="side base">
                  <div class="who">Base <span class="faint">· common ancestor</span></div>
                  <div class="code">{#if b.base.length}{@render lines(b.base)}{:else}<div class="none">No lines</div>{/if}</div>
                </div>
              {/if}
              {#each ["ours", "theirs"] as const as side}
                {@const order = sideOf(n, side)}
                <div class="side {side} pick" class:on={order > 0} role="button" tabindex="0" onclick={() => toggle(n, side)}
                  onkeydown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), toggle(n, side))}
                  title={order ? "Click to leave this side out" : "Click to use this side"}>
                  <div class="who">
                    {side === "ours" ? "Mine" : "Theirs"} <span class="faint">· {side === "ours" ? file.oursLabel : opName}</span>
                    {#if order}<span class="ord">{order === 1 && sideOf(n, side === "ours" ? "theirs" : "ours") ? "1st" : order === 2 ? "2nd" : ""}<Icon name="check" size={12} /></span>{/if}
                  </div>
                  <div class="code">{#if b[side].length}{@render lines(b[side])}{:else}<div class="none">No lines (removed on this side)</div>{/if}</div>
                </div>
              {/each}
            </div>
            {#if editing === n}
              <div class="editor">
                <div class="who">Result <span class="faint">· write it the way it should be</span></div>
                <!-- svelte-ignore a11y_autofocus -->
                <textarea class="mono" bind:value={draft} rows={Math.min(24, Math.max(4, draft.split("\n").length + 1))} autofocus spellcheck="false"
                  onkeydown={(e) => {
                    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") { e.preventDefault(); saveEdit(n); }
                    if (e.key === "Escape") { e.stopPropagation(); editing = null; }
                  }}></textarea>
                <div class="row">
                  <button class="btn small" onclick={() => (editing = null)}>Cancel</button>
                  <button class="btn small primary" onclick={() => saveEdit(n)}>Use this</button>
                </div>
              </div>
            {:else if c}
              {@const res = chosen(b, c)}
              <div class="result">
                <div class="who">Result</div>
                <div class="code">{#if res.length}{@render lines(res)}{:else}<div class="none">Nothing: these lines are removed</div>{/if}</div>
              </div>
            {/if}
          </section>
        {/if}
      {/each}
      <div class="end">
        {#if done < total}
          <span class="faint">{total - done} conflict{total - done === 1 ? "" : "s"} left.</span>
        {:else}
          <button class="btn primary" onclick={save} disabled={saving}><Icon name="check" size={14} /> Save and mark resolved</button>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .cv { height: 100%; display: flex; flex-direction: column; min-height: 0; background: var(--surface); }
  .bar { display: flex; align-items: center; gap: 8px; padding: 7px 12px; border-bottom: 1px solid var(--line); flex-wrap: wrap; }
  .progress { display: flex; align-items: center; gap: 8px; font-size: calc(var(--fs) * 12); color: var(--ink-2); }
  .meter { width: 70px; height: 6px; border-radius: 3px; background: var(--surface-3); overflow: hidden; }
  .meter span { display: block; height: 100%; background: var(--add-ink); transition: width 0.2s; }
  .nav { display: flex; gap: 3px; flex-wrap: wrap; max-width: 40%; }
  .dot { min-width: 20px; height: 20px; padding: 0 4px; border-radius: 10px; font-size: calc(var(--fs) * 10.5); font-weight: 700; color: var(--warn); border: 1px solid currentColor; }
  .dot.ok { color: var(--add-ink); background: var(--add-bg); border-color: transparent; }
  .sp { flex: 1; }
  .chk { display: flex; align-items: center; gap: 5px; font-size: calc(var(--fs) * 12); color: var(--ink-2); }
  .note { display: flex; gap: 6px; align-items: center; padding: 6px 12px; background: var(--warn-soft); font-size: calc(var(--fs) * 12); }
  .doc { flex: 1; overflow: auto; padding: 10px 14px 30px; font-size: calc(var(--cs) * 12); }
  .cl { white-space: pre; tab-size: 4; min-height: calc(var(--cs) * 19px / 12); line-height: 1.55; padding: 0 10px; }
  .ctx { color: var(--ink-2); padding: 2px 0; }
  .fold { display: block; width: 100%; text-align: left; padding: 3px 10px; margin: 2px 0; color: var(--ink-3); background: var(--surface-2); border-radius: 4px; font-family: var(--font-ui); font-size: calc(var(--fs) * 11.5); }
  .fold:hover { color: var(--ink); background: var(--surface-3); }
  .conflict { margin: 8px 0; border: 1px solid var(--warn); border-radius: 8px; overflow: hidden; background: var(--surface); scroll-margin-top: 8px; }
  .conflict.resolved { border-color: var(--line); }
  .conflict header { display: flex; align-items: center; gap: 8px; padding: 6px 8px 6px 12px; background: var(--warn-soft); font-family: var(--font-ui); font-size: calc(var(--fs) * 12); flex-wrap: wrap; }
  .conflict.resolved header { background: var(--surface-2); }
  .ttl { font-weight: 700; }
  .badge { display: inline-flex; align-items: center; gap: 3px; color: var(--add-ink); background: var(--add-bg); border-radius: 4px; padding: 1px 6px; font-weight: 600; font-size: calc(var(--fs) * 11); }
  .seg { display: flex; background: var(--surface-3); border-radius: 6px; padding: 2px; }
  .seg button { padding: 2px 9px; border-radius: 4px; font-weight: 600; color: var(--ink-2); font-size: calc(var(--fs) * 11.5); }
  .seg button:hover { color: var(--ink); }
  .seg button.on { background: var(--surface); color: var(--accent); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.1); }
  .sides { display: grid; grid-template-columns: 1fr 1fr; }
  .sides.three { grid-template-columns: 1fr 1fr 1fr; }
  .side { display: flex; flex-direction: column; min-width: 0; text-align: left; border-top: 1px solid var(--line-soft); border-left: 3px solid transparent; font: inherit; color: inherit; }
  .side + .side { border-left-color: transparent; box-shadow: inset 1px 0 0 var(--line-soft); }
  .side.pick { cursor: pointer; }
  .side.pick:hover { background: var(--surface-2); }
  .side.ours.on { background: var(--sel); border-left-color: var(--accent); }
  .side.theirs.on { background: color-mix(in srgb, var(--lane-1) 14%, transparent); border-left-color: var(--lane-1); }
  .side .code, .result .code { overflow-x: auto; padding: 4px 0 6px; }
  .who { display: flex; align-items: center; gap: 4px; padding: 5px 10px 2px; font-family: var(--font-ui); font-size: calc(var(--fs) * 11); font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: 0.03em; }
  .who .faint { text-transform: none; letter-spacing: 0; font-weight: 500; }
  .side.ours .who { color: var(--accent); }
  .side.theirs .who { color: var(--lane-1); }
  .ord { margin-left: auto; display: inline-flex; align-items: center; gap: 3px; text-transform: none; }
  .none { padding: 2px 10px; color: var(--ink-3); font-style: italic; font-family: var(--font-ui); }
  .result { border-top: 1px dashed var(--line); background: var(--add-bg); }
  .result .who { color: var(--add-ink); }
  .editor { border-top: 1px dashed var(--line); padding-bottom: 8px; }
  .editor textarea { display: block; width: calc(100% - 20px); margin: 4px 10px; font-family: var(--font-mono); font-size: calc(var(--cs) * 12); line-height: 1.55; tab-size: 4; white-space: pre; resize: vertical; }
  .row { display: flex; gap: 6px; justify-content: flex-end; padding: 0 10px; margin-top: 8px; font-family: var(--font-ui); }
  .empty .row { justify-content: center; }
  .end { display: flex; justify-content: center; padding: 16px; font-family: var(--font-ui); }
  .whole { padding: 30px; max-width: 640px; margin: 0 auto; text-align: center; }
  .whole h3 { margin: 0 0 4px; }
  .cards { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; margin-top: 18px; }
  .card { display: flex; flex-direction: column; gap: 6px; padding: 16px; border: 1px solid var(--line); border-radius: 10px; text-align: left; background: var(--surface-2); }
  .card:hover { border-color: var(--accent); background: var(--sel); }
  .card .who { padding: 0; }
  .card.mine .who { color: var(--accent); }
  .card.theirs .who { color: var(--lane-1); }
  .spin { display: inline-block; width: 12px; height: 12px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--accent); animation: sp 0.8s linear infinite; vertical-align: -2px; }
  @keyframes sp { to { rotate: 1turn; } }
</style>
