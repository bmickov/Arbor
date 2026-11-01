// Changed files shown as a folder tree. Folders with a single subfolder
// and nothing else are merged into one row ("src/lib"), like most editors.

export type TreeRow<T> =
  | { kind: "dir"; path: string; name: string; depth: number; count: number; open: boolean }
  | { kind: "file"; item: T; name: string; depth: number };

interface Dir<T> { name: string; path: string; dirs: Map<string, Dir<T>>; files: { item: T; name: string }[]; count: number }

export function treeRows<T>(items: T[], pathOf: (t: T) => string, collapsed: Set<string>): TreeRow<T>[] {
  const root: Dir<T> = { name: "", path: "", dirs: new Map(), files: [], count: 0 };
  for (const item of items) {
    const parts = pathOf(item).split("/");
    let d = root;
    d.count++;
    for (let i = 0; i < parts.length - 1; i++) {
      const p = parts.slice(0, i + 1).join("/");
      let next = d.dirs.get(parts[i]);
      if (!next) {
        next = { name: parts[i], path: p, dirs: new Map(), files: [], count: 0 };
        d.dirs.set(parts[i], next);
      }
      next.count++;
      d = next;
    }
    d.files.push({ item, name: parts[parts.length - 1] });
  }
  const out: TreeRow<T>[] = [];
  const walk = (d: Dir<T>, depth: number) => {
    const dirs = [...d.dirs.values()].sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" }));
    for (let sub of dirs) {
      let name = sub.name;
      // Merge chains of lone folders.
      while (sub.files.length === 0 && sub.dirs.size === 1) {
        const only = [...sub.dirs.values()][0];
        name = `${name}/${only.name}`;
        sub = only;
      }
      const open = !collapsed.has(sub.path);
      out.push({ kind: "dir", path: sub.path, name, depth, count: sub.count, open });
      if (open) walk(sub, depth + 1);
    }
    const files = [...d.files].sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" }));
    for (const f of files) out.push({ kind: "file", item: f.item, name: f.name, depth });
  };
  walk(root, 0);
  return out;
}

/** Paths of items under a folder. */
export function under<T>(items: T[], pathOf: (t: T) => string, dir: string): string[] {
  return items.map(pathOf).filter((p) => p.startsWith(dir + "/"));
}
