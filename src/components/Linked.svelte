<script lang="ts">
  // Text with issue keys turned into links (opened in the browser).
  import { api } from "../lib/api";
  import { linkify } from "../lib/issues";
  import { app } from "../lib/state.svelte";

  let { text }: { text: string } = $props();
  const pieces = $derived.by(() => {
    void app.settings.issueLinks;
    void app.refs;
    return linkify(text);
  });
</script>

{#each pieces as p}{#if p.href}<a class="issue" href={p.href} title="Open {p.href}"
  onclick={(e) => { e.preventDefault(); e.stopPropagation(); api.openExternal(p.href!); }}>{p.text}</a>{:else}{p.text}{/if}{/each}

<style>
  .issue { color: var(--accent); text-decoration: none; border-bottom: 1px dotted currentColor; cursor: pointer; }
  .issue:hover { border-bottom-style: solid; }
</style>
