// Conflict markers, parsed into plain text and conflict blocks, and put back
// together from the user's choices.
//
//   <<<<<<< ours-label
//   our lines
//   ||||||| base-label        (diff3 style only)
//   base lines
//   =======
//   their lines
//   >>>>>>> theirs-label

export interface Block {
  ours: string[];
  base: string[] | null;
  theirs: string[];
  oursLabel: string;
  theirsLabel: string;
}
export type Segment = { kind: "text"; lines: string[] } | { kind: "conflict"; block: Block };

export interface Parsed {
  segments: Segment[];
  /** Index into segments of each conflict, in order. */
  conflicts: number[];
  eol: "\n" | "\r\n";
  finalNewline: boolean;
}

/** A side, both sides in either order, the base, or text the user typed. */
export type Choice =
  | { take: "ours" | "theirs" | "base" | "ours+theirs" | "theirs+ours" }
  | { edit: string[] };

function marker(line: string, ch: string): string | null {
  const m = ch.repeat(7);
  if (!line.startsWith(m)) return null;
  const rest = line.slice(7);
  if (rest === "") return "";
  if (rest[0] === " ") return rest.slice(1);
  return null;
}

export function parseConflicts(text: string): Parsed {
  const eol = text.includes("\r\n") ? "\r\n" : "\n";
  const finalNewline = text.endsWith("\n");
  const body = finalNewline ? text.slice(0, text.endsWith("\r\n") ? -2 : -1) : text;
  const lines = body === "" && finalNewline ? [""] : body.split(/\r?\n/);
  if (text === "") lines.length = 0;

  const segments: Segment[] = [];
  const conflicts: number[] = [];
  let plain: string[] = [];
  const flush = () => {
    if (plain.length) segments.push({ kind: "text", lines: plain });
    plain = [];
  };

  let i = 0;
  while (i < lines.length) {
    const start = marker(lines[i], "<");
    if (start === null) {
      plain.push(lines[i++]);
      continue;
    }
    // Read a whole block; a malformed one stays as text.
    const ours: string[] = [];
    let base: string[] | null = null;
    const theirs: string[] = [];
    let part: "ours" | "base" | "theirs" = "ours";
    let end: string | null = null;
    let j = i + 1;
    for (; j < lines.length; j++) {
      const l = lines[j];
      if (part === "ours" && marker(l, "|") !== null) { part = "base"; base = []; continue; }
      if (part !== "theirs" && l === "=======") { part = "theirs"; continue; }
      if (part === "theirs" && (end = marker(l, ">")) !== null) break;
      if (marker(l, "<") !== null) break; // nested or broken: give up on this one
      (part === "ours" ? ours : part === "base" ? base! : theirs).push(l);
    }
    if (end === null || j >= lines.length) {
      plain.push(lines[i++]);
      continue;
    }
    flush();
    conflicts.push(segments.length);
    segments.push({ kind: "conflict", block: { ours, base, theirs, oursLabel: start, theirsLabel: end } });
    i = j + 1;
  }
  flush();
  return { segments, conflicts, eol, finalNewline };
}

/** The lines a choice puts in place of a conflict block. */
export function chosen(b: Block, c: Choice): string[] {
  if ("edit" in c) return c.edit;
  switch (c.take) {
    case "ours": return b.ours;
    case "theirs": return b.theirs;
    case "base": return b.base ?? [];
    case "ours+theirs": return [...b.ours, ...b.theirs];
    case "theirs+ours": return [...b.theirs, ...b.ours];
  }
}

/** The file with each conflict replaced by its choice; unchosen ones keep their markers. */
export function compose(p: Parsed, choices: (Choice | null)[]): string {
  const out: string[] = [];
  let n = 0;
  for (const s of p.segments) {
    if (s.kind === "text") {
      out.push(...s.lines);
      continue;
    }
    const c = choices[n++];
    const b = s.block;
    if (c) {
      out.push(...chosen(b, c));
    } else {
      out.push(`<<<<<<< ${b.oursLabel}`.trimEnd(), ...b.ours);
      if (b.base) out.push("|||||||", ...b.base);
      out.push("=======", ...b.theirs, `>>>>>>> ${b.theirsLabel}`.trimEnd());
    }
  }
  if (!out.length) return "";
  return out.join(p.eol) + (p.finalNewline ? p.eol : "");
}
