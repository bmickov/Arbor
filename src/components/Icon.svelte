<script lang="ts">
  // A small stroked icon set drawn for Arbor (16px grid).
  let { name, size = 16 }: { name: string; size?: number } = $props();

  const paths: Record<string, string> = {
    fetch: "M8 2.5v8M4.8 7.6 8 10.8l3.2-3.2M3 13.5h10",
    pull: "M8 1.5v7.5M5 6l3 3 3-3M2.5 10.5v2.5a1 1 0 0 0 1 1h9a1 1 0 0 0 1-1v-2.5",
    push: "M8 10.5V2.5M5 5.5l3-3 3 3M2.5 10.5v2.5a1 1 0 0 0 1 1h9a1 1 0 0 0 1-1v-2.5",
    stash: "M2.5 5.5h11v7a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1zM1.5 3h13v2.5h-13zM6.5 8.5h3",
    undo: "M5.5 3.5 2.5 6.5l3 3M2.8 6.5h6.7a4 4 0 0 1 0 8H7",
    branch: "M4.5 2.5v11M4.5 9.5c0-3 7-2 7-5M11.5 4.5m-1.6 0a1.6 1.6 0 1 0 3.2 0a1.6 1.6 0 1 0 -3.2 0M4.5 13.5m-1.6 0a1.6 1.6 0 1 0 3.2 0",
    changes: "M3 2.5h7l3 3v8a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1v-10a1 1 0 0 1 1-1zM5 9h6M8 6v6",
    history: "M8 2.5a5.5 5.5 0 1 1-5.2 3.7M2.5 2.5v3.7h3.7M8 5v3.3l2.2 1.4",
    tag: "M2 2.5h5.5l6.5 6.5-5 5-6.5-6.5zM5.2 5.2h.1",
    remote: "M4.5 12.5h7a3 3 0 0 0 .4-6 4 4 0 0 0-7.8 1A2.5 2.5 0 0 0 4.5 12.5z",
    folder: "M1.5 4.5a1 1 0 0 1 1-1h3.5l1.5 1.5h6a1 1 0 0 1 1 1v6.5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z",
    terminal: "M2 3h12v10H2zM4.5 6.5l2 1.5-2 1.5M8 10h3.5",
    refresh: "M13 4.5v3h-3M3 11.5v-3h3M12.6 7.5A5 5 0 0 0 3.8 5.4M3.4 8.5a5 5 0 0 0 8.8 2.1",
    search: "M7 2.5a4.5 4.5 0 1 1 0 9a4.5 4.5 0 1 1 0-9zM10.3 10.3 13.5 13.5",
    chevron: "M6 4l4 4-4 4",
    down: "M4 6l4 4 4-4",
    plus: "M8 3v10M3 8h10",
    minus: "M3 8h10",
    check: "M3 8.5l3 3 7-7",
    x: "M4 4l8 8M12 4l-8 8",
    more: "M3.5 8h.1M8 8h.1M12.5 8h.1",
    merge: "M4.5 2.5v11M4.5 6.5c0 3 7 3 7 7M11.5 13.5m-1.6 0a1.6 1.6 0 1 0 3.2 0",
    commit: "M1.5 8h4M10.5 8h4M8 5.5a2.5 2.5 0 1 1 0 5a2.5 2.5 0 1 1 0-5z",
    discard: "M3 4.5h10M6 4.5V3h4v1.5M4.5 4.5l.7 9h5.6l.7-9",
    file: "M4 1.5h5.5l3 3V14a.5.5 0 0 1-.5.5H4a.5.5 0 0 1-.5-.5V2a.5.5 0 0 1 .5-.5zM9.5 1.5v3h3",
    warning: "M8 2l6.5 11.5h-13zM8 6.5v3.3M8 11.6h.1",
    split: "M2 2.5h12v11H2zM8 2.5v11",
    unified: "M2 2.5h12v11H2zM2 8h12",
    grid: "M2.5 2.5h4.5v4.5H2.5zM9 2.5h4.5v4.5H9zM2.5 9h4.5v4.5H2.5zM9 9h4.5v4.5H9z",
    group: "M1.5 4a1 1 0 0 1 1-1h3.5l1.5 1.5h6a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z",
    grip: "M6 4h.1M10 4h.1M6 8h.1M10 8h.1M6 12h.1M10 12h.1",
    settings: "M2 4.5h6.5M11.5 4.5H14M2 11.5h2.5M7.5 11.5H14M10 2.8v3.4M6 9.8v3.4",
  };
</script>

<svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  <path d={paths[name] ?? ""} />
</svg>
