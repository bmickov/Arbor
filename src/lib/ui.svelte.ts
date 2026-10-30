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

class ToastState {
  list = $state<Toast[]>([]);
  private n = 0;
  push(kind: Toast["kind"], text: string, action?: Toast["action"], ms?: number) {
    const id = ++this.n;
    this.list = [...this.list, { id, kind, text, action }];
    const life = ms ?? (kind === "error" ? 9000 : action ? 7000 : 3500);
    setTimeout(() => this.dismiss(id), life);
  }
  dismiss(id: number) {
    this.list = this.list.filter((t) => t.id !== id);
  }
}
export const toasts = new ToastState();
