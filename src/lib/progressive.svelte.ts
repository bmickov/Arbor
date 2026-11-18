// Long lists (ten thousand changed files, a 50 000-line blame) are put on
// screen a slice at a time: the first rows straight away, more as the list
// is scrolled towards its end. Building every row up front froze the window
// for seconds.

export class Progressive {
  limit = $state(0);
  #step: number;

  constructor(step = 300) {
    this.#step = step;
    this.limit = step;
  }

  /** Back to the first slice (e.g. when the list is replaced by another). */
  reset() {
    this.limit = this.#step;
  }

  more = () => {
    this.limit += this.#step;
  };

  /** Make sure row `i` is rendered (keyboard navigation, a selection far down). */
  reveal(i: number) {
    if (i >= this.limit) this.limit = (Math.floor(i / this.#step) + 1) * this.#step;
  }
}

/**
 * Calls `onvisible` while the element is in (or near) view. Put it after the
 * last rendered row: each call renders more rows, which pushes it further
 * down, until it's out of view.
 */
export function nearEnd(node: HTMLElement, onvisible: () => void) {
  let cb = onvisible;
  let frame = 0;
  const io = new IntersectionObserver(
    (entries) => {
      if (!entries.some((e) => e.isIntersecting)) return;
      cb();
      // Still in view after the new rows? Observing again reports it afresh.
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        io.unobserve(node);
        io.observe(node);
      });
    },
    { rootMargin: "800px 0px" },
  );
  io.observe(node);
  return {
    update(next: () => void) {
      cb = next;
    },
    destroy() {
      cancelAnimationFrame(frame);
      io.disconnect();
    },
  };
}
