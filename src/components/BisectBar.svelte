<script lang="ts">
  // Shown while a bisect runs: what's being tested, and the verdict buttons.
  import * as act from "../lib/actions";
  import { app } from "../lib/state.svelte";
  import Icon from "./Icon.svelte";

  const b = $derived(app.status?.bisect ?? null);
  const subject = $derived(app.status?.headSummary ?? "");
</script>

{#if b}
  <div class="bisect" class:found={b.found} role="status">
    <Icon name="bisect" size={15} />
    {#if b.found}
      <div class="what">
        <b>Found the first bad commit</b>
        <span><button class="sha mono" onclick={() => { if (b.found) app.reveal(b.found); app.view = "history"; }}>{b.found.slice(0, 7)}</button> {subject}</span>
      </div>
      <button class="btn small" onclick={() => { if (b.found) app.reveal(b.found); app.view = "history"; }}>Show commit</button>
      <button class="btn small primary" onclick={act.bisectStop} disabled={!!app.busy}>Stop bisect</button>
    {:else if b.ambiguous.length}
      <div class="what">
        <b>Only skipped commits are left</b>
        <span>The first bad commit is one of {b.ambiguous.length}: {b.ambiguous.map((x) => x.slice(0, 7)).join(", ")}.</span>
      </div>
      <button class="btn small primary" onclick={act.bisectStop} disabled={!!app.busy}>Stop bisect</button>
    {:else}
      <div class="what">
        <b>Bisecting: does <span class="mono">{b.current?.slice(0, 7)}</span> have the bug?</b>
        <span>{subject} · {b.remaining} commit{b.remaining === 1 ? "" : "s"} left to check, about {b.steps} step{b.steps === 1 ? "" : "s"}. Build or run it, then:</span>
      </div>
      <button class="btn small good" onclick={() => act.bisectMark("good")} disabled={!!app.busy} title="This commit works">Good</button>
      <button class="btn small bad" onclick={() => act.bisectMark("bad")} disabled={!!app.busy} title="This commit has the bug">Bad</button>
      <button class="btn small" onclick={() => act.bisectMark("skip")} disabled={!!app.busy} title="Can't test this one (doesn't build, say)">Skip</button>
      <button class="btn small ghost" onclick={act.bisectStop} disabled={!!app.busy} title="Go back to {b.original}">Stop</button>
    {/if}
  </div>
{/if}

<style>
  .bisect { display: flex; align-items: center; gap: 10px; padding: 8px 14px; background: var(--warn-soft); border-bottom: 1px solid var(--line); font-size: calc(var(--fs) * 12.5); }
  .bisect.found { background: var(--add-bg); }
  .bisect > :global(svg) { color: var(--warn); flex: none; }
  .bisect.found > :global(svg) { color: var(--add-ink); }
  .what { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .what span { color: var(--ink-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sha { color: var(--accent); }
  .sha:hover { text-decoration: underline; }
  .good { color: var(--add-ink); border-color: var(--add-ink); font-weight: 700; }
  .bad { color: var(--danger); border-color: var(--danger); font-weight: 700; }
</style>
