// Dialogs, context menus and toasts. Each is a small piece of global state
// rendered once by App.svelte; callers just await a promise.

export interface MenuItem {
  label?: string;
  hint?: string;
  danger?: boolean;
  disabled?: boolean;
  separator?: boolean;
  action?: () => void;
}

class MenuState {
  items = $state<MenuItem[]>([]);
  x = $state(0);
  y = $state(0);
  title = $state<string | null>(null);
  open(x: number, y: number, items: MenuItem[], title: string | null = null) {
    this.items = items;
    this.title = title;
    this.x = x;
    this.y = y;
  }
  fromEvent(e: MouseEvent, items: MenuItem[], title: string | null = null) {
    e.preventDefault();
    e.stopPropagation();
    this.open(e.clientX, e.clientY, items, title);
  }
  close() {
    this.items = [];
    this.title = null;
  }
}
export const menu = new MenuState();

export interface Field {
  key: string;
  label: string;
  value?: string;
  placeholder?: string;
  type?: "text" | "password" | "textarea" | "checkbox";
  checked?: boolean;
  hint?: string;
}

export interface DialogSpec {
  title: string;
  body?: string;
  /** A short scrollable list shown under the body (what's about to be deleted, say). */
  items?: { text: string; note?: string; warn?: boolean }[];
  fields?: Field[];
  confirm: string;
  cancel?: string | null;
  danger?: boolean;
  /** Extra buttons that resolve with their id. */
  extra?: { id: string; label: string; danger?: boolean }[];
  validate?: (values: Record<string, string | boolean>) => string | null;
}

export interface DialogResult {
  button: string; // "confirm", "cancel" or an extra id
  values: Record<string, string | boolean>;
}

class DialogState {
  spec = $state<DialogSpec | null>(null);
  private resolve: ((r: DialogResult) => void) | null = null;

  show(spec: DialogSpec): Promise<DialogResult> {
    if (this.resolve) this.resolve({ button: "cancel", values: {} });
    this.spec = spec;
    return new Promise((res) => (this.resolve = res));
  }
  finish(r: DialogResult) {
    const res = this.resolve;
    this.resolve = null;
    this.spec = null;
    res?.(r);
  }

  async confirm(title: string, body: string, confirm: string, danger = false): Promise<boolean> {
    const r = await this.show({ title, body, confirm, danger });
    return r.button === "confirm";
  }

  async prompt(title: string, label: string, opts: { value?: string; confirm: string; placeholder?: string; body?: string; validate?: (v: string) => string | null }): Promise<string | null> {
    const r = await this.show({
      title,
      body: opts.body,
      confirm: opts.confirm,
      fields: [{ key: "v", label, value: opts.value ?? "", placeholder: opts.placeholder }],
      validate: opts.validate ? (v) => opts.validate!(String(v.v ?? "")) : undefined,
    });
    return r.button === "confirm" ? String(r.values.v ?? "").trim() : null;
  }
}
export const dialog = new DialogState();

export interface Toast {
  id: number;
  kind: "info" | "success" | "error";
  text: string;
  action?: { label: string; run: () => void };
}

const MAX_TOASTS = 4;

class ToastState {
  list = $state<Toast[]>([]);
  private n = 0;
  private timers = new Map<number, { handle: ReturnType<typeof setTimeout>; life: number }>();
  push(kind: Toast["kind"], text: string, action?: Toast["action"], ms?: number) {
    const life = ms ?? (kind === "error" ? 9000 : action ? 7000 : 3500);
    // The same message again (a retried action, a repeated error) refreshes
    // the one on screen instead of stacking a copy.
    const same = this.list.find((t) => t.kind === kind && t.text === text);
    if (same) {
      this.list = [...this.list.filter((t) => t !== same), { ...same, action }];
      this.start(same.id, life);
      return;
    }
    const id = ++this.n;
    const list = [...this.list, { id, kind, text, action }];
    // Keep a handful: the oldest go first, errors last.
    while (list.length > MAX_TOASTS) {
      const i = list.findIndex((t) => t.kind !== "error");
      const [gone] = list.splice(i >= 0 ? i : 0, 1);
      this.stop(gone.id);
    }
    this.list = list;
    this.start(id, life);
  }
  private start(id: number, life: number) {
    this.stop(id);
    this.timers.set(id, { handle: setTimeout(() => this.dismiss(id), life), life });
  }
  private stop(id: number) {
    const t = this.timers.get(id);
    if (t) clearTimeout(t.handle);
    this.timers.delete(id);
  }
  /** While the pointer is on a toast it stays, so a long message can be read. */
  hold(id: number) {
    const t = this.timers.get(id);
    if (t) clearTimeout(t.handle);
  }
  release(id: number) {
    const t = this.timers.get(id);
    if (t) this.start(id, Math.min(t.life, 3000));
  }
  dismiss(id: number) {
    this.stop(id);
    this.list = this.list.filter((t) => t.id !== id);
  }
}
export const toasts = new ToastState();
