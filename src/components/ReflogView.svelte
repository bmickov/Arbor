<script lang="ts">
  // Reflog: where HEAD (or a branch) pointed recently. Commits nothing else
  // reaches any more are marked "lost"; recover them with a new branch.
  import { api, type ReflogEntry } from "../lib/api";
  import * as act from "../lib/actions";
  import { fullDate, relTime } from "../lib/format";
  import { app } from "../lib/state.svelte";
  import { menu, toasts } from "../lib/ui.svelte";
  import CommitDetail from "./CommitDetail.svelte";
  import Icon from "./Icon.svelte";
  import Splitter from "./Splitter.svelte";

  let refname = $state("HEAD");
  let entries = $state<ReflogEntry[]>([]);
  let onlyLost = $state(false);
  // Counted from the oldest entry, so the selection stays put when new
  // entries arrive at the top.
  let fromEnd = $state<number | null>(null);
  const selected = $derived(fromEnd === null ? null : entries.length - 1 - fromEnd);
  const select = (i: number) => (fromEnd = entries.length - 1 - i);

  $effect(() => {
    const r = refname;
    void app.refsVersion;
    api.reflog(app.path, r, 500)
      .then((e) => {
        entries = e;
        if (fromEnd === null || fromEnd >= e.length) {
          const lost = e.findIndex((x) => x.lost);
          fromEnd = e.length ? e.length - 1 - (lost >= 0 ? lost : 0) : null;
        }
      })
      .catch((e) => {
        entries = [];
        toasts.push("error", String(e));
      });
  });

  const shown = $derived(entries.map((e, i) => ({ e, i })).filter(({ e }) => !onlyLost || e.lost));
  const sel = $derived(selected !== null ? entries[selected] ?? null : null);
  const lostCount = $derived(entries.filter((e) => e.lost).length);

  /** "checkout: moving from a to b" → { verb: "checkout", detail: "moving from a to b" } */
  function parts(m: string) {
    const i = m.indexOf(": ");
    return i > 0 ? { verb: m.slice(0, i), detail: m.slice(i + 2) } : { verb: "", detail: m };
  }

  function entryMenu(e: MouseEvent, x: ReflogEntry, i: number) {
    select(i);
    if (!x.summary) return;
    menu.fromEvent(e, [
      { label: "New branch here…", hint: x.lost ? "recovers it" : undefined, action: () => act.newBranch(x.oid, x.short) },
      { label: "Check out this commit", action: () => app.act("Checking out", () => api.checkoutCommit(app.path, x.oid), { success: `Checked out ${x.short} (detached).` }) },
      { label: `Cherry-pick into ${app.headBranch ?? "HEAD"}`, action: () => act.cherryPick(x.oid) },
      { label: `Reset ${app.headBranch ?? "HEAD"} to here…`, disabled: x.oid === app.status?.head, hint: x.oid === app.status?.head ? "already here" : undefined, action: () => act.resetTo(x) },
      { separator: true },
      { label: "Show in history", disabled: x.lost, action: () => act.selectCommit(x.oid) },
      { label: "Copy commit ID", action: () => navigator.clipboard.writeText(x.oid) },
    ], `${x.short} ${x.summary}`);
  }
</script>

<div class="reflog" style:--list-w="{app.size('reflog.list', 380)}px">
  <div class="list">
    <div class="top">
      <select bind:value={refname} aria-label="Whose history of positions">
        <option value="HEAD">HEAD (everything you checked out)</option>
        {#each app.refs?.local ?? [] as b}<option value={"refs/heads/" + b.name}>{b.name}</option>{/each}
      </select>
      <label class="lost-only" title="Commits that no branch, tag or remote branch contains any more">
        <input type="checkbox" bind:checked={onlyLost} /> Only lost commits ({lostCount})
      </label>
    </div>
    <p class="intro faint">Every place {refname === "HEAD" ? "HEAD" : refname.replace("refs/heads/", "")} has pointed recently, newest first. Commits marked <b>lost</b> aren't on any branch any more; right-click → <b>New branch here</b> to get them back.</p>
    <div class="rows">
      {#each shown as { e, i } (i)}
        {@const p = parts(e.message)}
        <button class="item" class:sel={i === selected} class:lost={e.lost} onclick={() => select(i)} oncontextmenu={(ev) => entryMenu(ev, e, i)}>
          <span class="line1">
            {#if e.lost}<span class="tag">lost</span>{/if}
            <span class="sum">{e.summary ?? "(commit no longer exists)"}</span>
            <span class="sha mono">{e.short}</span>
          </span>
          <span class="line2 faint"><b>{p.verb || "update"}</b> {p.detail} · <span title={fullDate(e.time)}>{relTime(e.time)}</span></span>
        </button>
      {:else}
        <div class="empty"><div><strong>{onlyLost ? "Nothing lost" : "No reflog"}</strong>{onlyLost ? "Every commit here is still on a branch." : "Git hasn't recorded any moves yet."}</div></div>
      {/each}
    </div>
  </div>
  <Splitter id="reflog.list" min={220} keep={320} />
  <div class="detail">
    {#if sel?.summary}
      {#key sel.oid}
        <CommitDetail oid={sel.oid}>
          {#snippet header()}
            <div class="actions">
              <button class="btn small" class:primary={sel.lost} onclick={() => act.newBranch(sel.oid, sel.short)}><Icon name="branch" size={13} /> {sel.lost ? "Recover as new branch…" : "New branch here…"}</button>
              <button class="btn small" disabled={sel.oid === app.status?.head} title={sel.oid === app.status?.head ? `${app.headBranch ?? "HEAD"} is already here` : undefined} onclick={() => act.resetTo(sel)}>Reset {app.headBranch ?? "HEAD"} to here…</button>
              {#if !sel.lost}<button class="btn small ghost" onclick={() => act.selectCommit(sel.oid)}>Show in history</button>{/if}
            </div>
          {/snippet}
        </CommitDetail>
      {/key}
    {:else if sel}
      <div class="empty"><div><strong>The commit is gone</strong>Git has already cleaned it up, so it can't be recovered.</div></div>
    {/if}
  </div>
</div>

<style>
  .reflog { display: grid; grid-template-columns: var(--list-w) auto minmax(0, 1fr); height: 100%; min-height: 0; background: var(--surface); }
  .list { display: flex; flex-direction: column; min-height: 0; min-width: 0; }
  .top { display: flex; flex-direction: column; gap: 6px; padding: 10px 10px 4px; }
  .lost-only { display: flex; align-items: center; gap: 6px; font-size: calc(var(--fs) * 12); color: var(--ink-2); }
  .intro { margin: 2px 12px 8px; font-size: calc(var(--fs) * 11.5); line-height: 1.45; }
  .rows { flex: 1; overflow-y: auto; padding: 0 6px 10px; }
  .item { display: flex; flex-direction: column; gap: 2px; width: 100%; padding: 7px 10px; border-radius: 6px; text-align: left; }
  .item:hover { background: var(--surface-2); }
  .item.sel { background: var(--sel); }
  .line1 { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .sum { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .sha { color: var(--ink-3); font-size: calc(var(--cs) * 11); flex: none; }
  .line2 { font-size: calc(var(--fs) * 11.5); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .line2 b { font-weight: 600; color: var(--ink-2); }
  .tag { flex: none; font-size: calc(var(--fs) * 10); font-weight: 700; color: var(--warn); border: 1px solid currentColor; border-radius: 4px; padding: 0 4px; }
  .detail { min-width: 0; min-height: 0; }
  .actions { display: flex; gap: 6px; flex-wrap: wrap; margin-bottom: 4px; }
</style>
