// Syntax highlighting with highlight.js (its common ~36 languages).
//
// Code is highlighted in chunks of consecutive lines, only when they're
// shown, and the HTML is split back into one string per line with every
// <span> closed at the end of a line and reopened on the next. That keeps
// multi-line comments and strings coloured within a chunk while big files
// and diffs stay fast.

import hljs from "highlight.js/lib/common";

/** Extensions highlight.js doesn't know by name. */
const BY_EXTENSION: Record<string, string> = {
  mjs: "javascript", cjs: "javascript", jsx: "javascript", tsx: "typescript", mts: "typescript", cts: "typescript",
  svelte: "xml", vue: "xml", html: "xml", htm: "xml", svg: "xml", xsd: "xml", plist: "xml",
  toml: "ini", cfg: "ini", conf: "ini", properties: "ini", env: "bash", zsh: "bash", fish: "bash",
  yml: "yaml", h: "c", hpp: "cpp", hh: "cpp", cc: "cpp", cxx: "cpp", mm: "objectivec",
  kt: "kotlin", kts: "kotlin", gradle: "kotlin", rb: "ruby", rake: "ruby", pl: "perl", pm: "perl",
  md: "markdown", markdown: "markdown", mdx: "markdown", sql: "sql", psql: "sql", graphql: "graphql", gql: "graphql",
  rs: "rust", go: "go", py: "python", pyi: "python", cs: "csharp", vb: "vbnet", lua: "lua", r: "r",
  scss: "scss", sass: "scss", less: "less", css: "css", json: "json", jsonc: "json", json5: "json", lock: "json",
  swift: "swift", php: "php", java: "java", diff: "diff", patch: "diff", wat: "wasm", sh: "bash", bash: "bash",
};
/** Whole file names. */
const BY_NAME: Record<string, string> = {
  makefile: "makefile", gnumakefile: "makefile", dockerfile: "bash", containerfile: "bash",
  ".bashrc": "bash", ".zshrc": "bash", ".profile": "bash", ".gitconfig": "ini", ".editorconfig": "ini",
  "cargo.lock": "ini", gemfile: "ruby", rakefile: "ruby", "go.mod": "go",
};

/** The highlight.js language for a file, or null to show it plain. */
export function languageFor(path: string): string | null {
  const base = path.slice(path.lastIndexOf("/") + 1).toLowerCase();
  const named = BY_NAME[base];
  if (named) return named;
  const dot = base.lastIndexOf(".");
  if (dot < 0) return null;
  const ext = base.slice(dot + 1);
  const lang = BY_EXTENSION[ext] ?? ext;
  return hljs.getLanguage(lang) ? lang : null;
}

/** Lines longer than this (minified code, data) are left plain: highlighting them is slow and unhelpful. */
const MAX_LINE = 2000;

/** Highlight consecutive lines; returns one HTML string per line (null where left plain). */
export function highlightLines(lines: string[], lang: string): (string | null)[] {
  if (lines.some((l) => l.length > MAX_LINE)) return lines.map(() => null);
  let html: string;
  try {
    html = hljs.highlight(lines.join("\n"), { language: lang, ignoreIllegals: true }).value;
  } catch {
    return lines.map(() => null);
  }
  const out: string[] = [];
  const open: string[] = [];
  let cur = "";
  // highlight.js output is escaped text plus <span …> and </span> only.
  for (const m of html.matchAll(/<span[^>]*>|<\/span>|\n|[^<\n]+/g)) {
    const t = m[0];
    if (t === "\n") {
      out.push(cur + "</span>".repeat(open.length));
      cur = open.join("");
    } else if (t === "</span>") {
      open.pop();
      cur += t;
    } else if (t.startsWith("<span")) {
      open.push(t);
      cur += t;
    } else {
      cur += t;
    }
  }
  out.push(cur + "</span>".repeat(open.length));
  return out;
}

const CHUNK = 200;

/**
 * Lazily highlights a long sequence of lines (a file, or one side of a diff
 * hunk) in chunks. `get(i)` returns line i's HTML, or null to show it plain.
 */
export class Highlighter {
  private cache = new Map<number, string | null>();
  private lang: string;
  private line: (i: number) => string;
  private count: number;
  constructor(lang: string, line: (i: number) => string, count: number) {
    this.lang = lang;
    this.line = line;
    this.count = count;
  }

  get(i: number): string | null {
    const hit = this.cache.get(i);
    if (hit !== undefined) return hit;
    const start = Math.floor(i / CHUNK) * CHUNK;
    const end = Math.min(this.count, start + CHUNK);
    const lines: string[] = [];
    for (let k = start; k < end; k++) lines.push(this.line(k));
    highlightLines(lines, this.lang).forEach((h, k) => this.cache.set(start + k, h));
    return this.cache.get(i) ?? null;
  }
}
