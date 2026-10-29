// Pointer-based drag and drop. HTML5 drag-and-drop is unreliable inside
// WebKitGTK, so drags are tracked with pointer events and drop targets are
// found with elementFromPoint.

export type Payload =
  | { kind: "branch"; name: string }
  | { kind: "remote-branch"; fullName: string }
  | { kind: "commit"; oid: string; summary: string }
  | { kind: "files"; paths: string[]; from: "staged" | "unstaged" }
  | { kind: "stash"; index: number; message: string };

interface Target {
  accepts: (p: Payload) => boolean;
  drop: (p: Payload, e: PointerEvent) => void;
  /** Called while hovering, with the pointer's vertical position (0..1) in the element. */
  hover?: (p: Payload, fraction: number) => void;
  leave?: () => void;
}

class DragState {
  payload = $state<Payload | null>(null);
  label = $state("");
  x = $state(0);
  y = $state(0);
  overAccepts = $state(false);
}
export const drag = new DragState();

const targets = new Map<string, Target>();
let nextId = 0;
let suppressClick = false;

function targetAt(x: number, y: number): [HTMLElement, Target] | null {
  let el = document.elementFromPoint(x, y) as HTMLElement | null;
  while (el) {
    const id = el.dataset?.dropId;
    if (id && targets.has(id)) return [el, targets.get(id)!];
    el = el.parentElement;
  }
  return null;
}

let current: [HTMLElement, Target] | null = null;

function setCurrent(next: [HTMLElement, Target] | null) {
  if (current && current[0] !== next?.[0]) {
    current[0].classList.remove("drop-over");
    current[1].leave?.();
  }
  current = next;
}

/** Svelte action: make an element draggable. */
export function draggable(node: HTMLElement, get: () => { payload: Payload; label: string } | null) {
  let start: { x: number; y: number; id: number } | null = null;

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    // Controls inside the draggable keep working; the draggable itself may be a button.
    const ctl = (e.target as HTMLElement).closest("button, input, textarea, select, [data-nodrag]");
    if (ctl && ctl !== node && node.contains(ctl)) return;
    start = { x: e.clientX, y: e.clientY, id: e.pointerId };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up, { once: true });
  }

  function move(e: PointerEvent) {
    if (!start) return;
    if (!drag.payload) {
      if (Math.hypot(e.clientX - start.x, e.clientY - start.y) < 6) return;
      const d = get();
      if (!d) {
        cleanup();
        return;
      }
      drag.payload = d.payload;
      drag.label = d.label;
      document.body.classList.add("dragging");
    }
    drag.x = e.clientX;
    drag.y = e.clientY;
    const t = targetAt(e.clientX, e.clientY);
    const ok = !!t && t[1].accepts(drag.payload!);
    setCurrent(ok ? t : null);
    drag.overAccepts = ok;
    if (ok && t) {
      t[0].classList.add("drop-over");
      const r = t[0].getBoundingClientRect();
      t[1].hover?.(drag.payload!, (e.clientY - r.top) / Math.max(1, r.height));
    }
  }

  function up(e: PointerEvent) {
    if (drag.payload) {
      suppressClick = true;
      setTimeout(() => (suppressClick = false), 0);
      const p = drag.payload;
      const t = targetAt(e.clientX, e.clientY);
      drag.payload = null;
      document.body.classList.remove("dragging");
      // Drop before clearing hover state: targets read it (e.g. above/below).
      if (t && t[1].accepts(p)) t[1].drop(p, e);
      setCurrent(null);
    }
    cleanup();
  }

  function cleanup() {
    start = null;
    window.removeEventListener("pointermove", move);
  }

  function click(e: MouseEvent) {
    if (suppressClick) {
      e.stopPropagation();
      e.preventDefault();
    }
  }

  // WebKitGTK starts its own native drag on mouse drags, which stops pointer
  // events from reaching the page; cancel it so ours runs instead.
  const nativeDrag = (e: DragEvent) => e.preventDefault();
  node.addEventListener("dragstart", nativeDrag);
  node.setAttribute("draggable", "false");
  node.addEventListener("pointerdown", down);
  node.addEventListener("click", click, true);
  return {
    destroy() {
      node.removeEventListener("pointerdown", down);
      node.removeEventListener("dragstart", nativeDrag);
      node.removeEventListener("click", click, true);
      cleanup();
    },
  };
}

/** Svelte action: make an element a drop target. */
export function dropTarget(node: HTMLElement, t: Target) {
  const id = `d${nextId++}`;
  node.dataset.dropId = id;
  targets.set(id, t);
  return {
    update(nt: Target) {
      targets.set(id, nt);
    },
    destroy() {
      targets.delete(id);
    },
  };
}
