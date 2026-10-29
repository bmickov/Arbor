<script lang="ts">
  import type { GraphRow } from "../lib/api";
  import { laneVar } from "../lib/format";

  let { g, h, lane, lanes, head = false, merge = false }: {
    g: GraphRow; h: number; lane: number; lanes: number; head?: boolean; merge?: boolean;
  } = $props();

  const pad = 8;
  const x = (i: number) => pad + i * lane + lane / 2;
  const cy = $derived(h / 2);

  function path(s: [number, number, number, number]): string {
    const [a, b, part] = s;
    const x1 = x(a), x2 = x(b);
    if (part === 0) return `M${x1} 0V${h}`;
    if (part === 1) return a === b ? `M${x1} 0V${cy}` : `M${x1} 0C${x1} ${cy * 0.9} ${x2} ${cy * 0.1} ${x2} ${cy}`;
    return a === b ? `M${x1} ${cy}V${h}` : `M${x1} ${cy}C${x1} ${cy + cy * 0.9} ${x2} ${cy + cy * 0.1} ${x2} ${h}`;
  }
</script>

<svg width={pad * 2 + lanes * lane} height={h} aria-hidden="true">
  {#each g.segs as s}
    {#if s[0] < lanes && s[1] < lanes}
      <path d={path(s)} stroke={laneVar(s[3])} />
    {/if}
  {/each}
  {#if g.col < lanes}
    {#if merge}
      <circle cx={x(g.col)} cy={cy} r="3.6" fill={laneVar(g.color)} />
    {:else}
      <circle cx={x(g.col)} cy={cy} r={head ? 6 : 5} class:head fill={head ? laneVar(g.color) : "var(--surface)"} stroke={laneVar(g.color)} />
    {/if}
  {/if}
</svg>

<style>
  svg { display: block; flex: none; overflow: visible; }
  path { fill: none; stroke-width: 2.6; stroke-linecap: round; }
  circle { stroke-width: 2.6; }
  circle.head { stroke: var(--surface); stroke-width: 2; filter: drop-shadow(0 0 0 currentColor); }
</style>
