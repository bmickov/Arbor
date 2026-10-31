<script lang="ts">
  // A draggable divider between two panels. It sizes the panel before it
  // (or, with `after`, the panel after it) in pixels, saved in the layout
  // under `id`. Double-click puts the default size back; arrow keys nudge it.
  import { app } from "../lib/state.svelte";

  let {
    id,
    axis = "x",
    min = 160,
    /** Room always left for the other panel. */
    keep = 240,
    after = false,
  }: { id: string; axis?: "x" | "y"; min?: number; keep?: number; after?: boolean } = $props();

  let node = $state<HTMLDivElement>();
  let dragging = $state(false);

  function panel(): HTMLElement | null {
    return (after ? node?.nextElementSibling : node?.previousElementSibling) as HTMLElement | null;
  }

  function set(v: number) {
    const parent = node?.parentElement?.getBoundingClientRect();
    const total = parent ? (axis === "x" ? parent.width : parent.height) : Infinity;
    app.layout.sizes[id] = Math.round(Math.max(min, Math.min(v, total - keep)));
  }

  function down(e: PointerEvent) {
    const p = panel();
    if (!p || e.button !== 0) return;
    e.preventDefault();
    const r = p.getBoundingClientRect();
    // Measure from the panel's far edge when sizing the panel after us.
    const edge = axis === "x" ? (after ? r.right : r.left) : after ? r.bottom : r.top;
    dragging = true;
    document.body.classList.add(axis === "x" ? "resizing-x" : "resizing-y");
    const move = (ev: PointerEvent) => {
      const pos = axis === "x" ? ev.clientX : ev.clientY;
      set(after ? edge - pos : pos - edge);
    };
    const up = () => {
      dragging = false;
      document.body.classList.remove("resizing-x", "resizing-y");
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function key(e: KeyboardEvent) {
    const grow = axis === "x" ? ["ArrowRight", "ArrowLeft"] : ["ArrowDown", "ArrowUp"];
    const i = grow.indexOf(e.key);
    if (i < 0) return;
    e.preventDefault();
    const p = panel();
    if (!p) return;
    const r = p.getBoundingClientRect();
    const cur = axis === "x" ? r.width : r.height;
    const step = (i === 0 ? 1 : -1) * (after ? -1 : 1) * (e.shiftKey ? 64 : 16);
    set(cur + step);
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div bind:this={node} class="splitter {axis}" class:dragging role="separator" tabindex="0"
  aria-orientation={axis === "x" ? "vertical" : "horizontal"}
  title="Drag to resize · double-click to reset"
  onpointerdown={down} onkeydown={key} ondblclick={() => delete app.layout.sizes[id]}></div>

<style>
  .splitter { position: relative; flex: none; background: var(--line); z-index: 2; }
  .splitter.x { width: 1px; cursor: col-resize; align-self: stretch; }
  .splitter.y { height: 1px; cursor: row-resize; }
  /* A wider grab area than the 1px line. */
  .splitter::after { content: ""; position: absolute; }
  .splitter.x::after { inset: 0 -4px; }
  .splitter.y::after { inset: -4px 0; }
  .splitter:hover, .splitter.dragging, .splitter:focus-visible { background: var(--accent); outline: none; }
  .splitter.x:hover, .splitter.x.dragging { box-shadow: 0 0 0 1px var(--accent); }
  .splitter.y:hover, .splitter.y.dragging { box-shadow: 0 0 0 1px var(--accent); }
</style>
