// Word-level changes inside a changed line: which parts of a removed line and
// its added counterpart actually differ, so the diff can highlight just those.

export type Range = [number, number]; // [start, end) in characters

const TOKEN = /\w+|\s+|[^\w\s]/gu;
/** Lines with more tokens than this aren't compared (too slow, rarely useful). */
const MAX_TOKENS = 400;

function tokens(s: string): { t: string; at: number }[] {
  const out: { t: string; at: number }[] = [];
  for (const m of s.matchAll(TOKEN)) out.push({ t: m[0], at: m.index! });
  return out;
}

/** Merge touching ranges, and ranges separated only by whitespace. */
function merge(ranges: Range[], s: string): Range[] {
  const out: Range[] = [];
  for (const r of ranges) {
    const last = out[out.length - 1];
    if (last && (last[1] === r[0] || /^\s*$/.test(s.slice(last[1], r[0])))) last[1] = r[1];
    else out.push([r[0], r[1]]);
  }
  return out;
}

/**
 * The changed parts of `a` (old line) and `b` (new line), or null when the
 * lines are too different (or too long) for word highlighting to help.
 */
export function wordRanges(a: string, b: string): { a: Range[]; b: Range[] } | null {
  if (a === b) return null;
  // Minified or generated lines: don't even split them up.
  if (a.length > MAX_TOKENS * 16 || b.length > MAX_TOKENS * 16) return null;
  const ta = tokens(a), tb = tokens(b);
  // The common case is a word or two changed mid-line: the same tokens at
  // both ends are matched directly, and only the middle goes to the LCS.
  let pre = 0;
  while (pre < ta.length && pre < tb.length && ta[pre].t === tb[pre].t) pre++;
  let suf = 0;
  while (suf < ta.length - pre && suf < tb.length - pre && ta[ta.length - 1 - suf].t === tb[tb.length - 1 - suf].t) suf++;
  const sameEnds = pre + suf;
  const ca = ta.slice(pre, ta.length - suf), cb = tb.slice(pre, tb.length - suf);
  if (ca.length > MAX_TOKENS || cb.length > MAX_TOKENS) return null;
  return lcsRanges(a, b, ca, cb, sameEnds);
}

function lcsRanges(a: string, b: string, ta: { t: string; at: number }[], tb: { t: string; at: number }[], sameEnds: number): { a: Range[]; b: Range[] } | null {
  // Longest common subsequence of the differing middle.
  const n = ta.length, m = tb.length;
  const dp = new Uint16Array((n + 1) * (m + 1));
  const at = (i: number, j: number) => i * (m + 1) + j;
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[at(i, j)] = ta[i].t === tb[j].t ? dp[at(i + 1, j + 1)] + 1 : Math.max(dp[at(i + 1, j)], dp[at(i, j + 1)]);
    }
  }
  const ra: Range[] = [], rb: Range[] = [];
  let i = 0, j = 0;
  const end = (t: { t: string; at: number }) => t.at + t.t.length;
  while (i < n || j < m) {
    if (i < n && j < m && ta[i].t === tb[j].t) {
      i++;
      j++;
    } else if (j < m && (i === n || dp[at(i, j + 1)] >= dp[at(i + 1, j)])) {
      rb.push([tb[j].at, end(tb[j])]);
      j++;
    } else {
      ra.push([ta[i].at, end(ta[i])]);
      i++;
    }
  }
  // Ignore pure whitespace changes inside a line unless that's all there is.
  const meaningful = (r: Range[], s: string) => r.filter(([x, y]) => /\S/.test(s.slice(x, y)));
  let ma = meaningful(ra, a), mb = meaningful(rb, b);
  if (!ma.length && !mb.length) {
    ma = ra;
    mb = rb;
  }
  // Mostly rewritten lines: highlighting nearly everything is just noise.
  const changed = (r: Range[]) => r.reduce((s, [x, y]) => s + y - x, 0);
  const common = dp[0] + sameEnds;
  if (common === 0 || (changed(ma) > a.trim().length * 0.7 && changed(mb) > b.trim().length * 0.7)) return null;
  return { a: merge(ma, a), b: merge(mb, b) };
}

const ESC: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };
const escape = (s: string) => s.replace(/[&<>"']/g, (c) => ESC[c]);

/**
 * Wrap the characters in `ranges` with <mark class="wd">. `html` is the
 * line's highlighted HTML (tags plus escaped text) or null for plain text.
 * Marks are closed at every tag and reopened after it, so nesting stays valid.
 */
export function markRanges(html: string | null, text: string, ranges: Range[]): string {
  if (!ranges.length) return html ?? escape(text);
  const src = html ?? escape(text);
  let out = "";
  let pos = 0; // character position in the plain text
  let r = 0; // current range
  let open = false;
  const inRange = (p: number) => {
    while (r < ranges.length && p >= ranges[r][1]) r++;
    return r < ranges.length && p >= ranges[r][0];
  };
  for (let i = 0; i < src.length; ) {
    if (src[i] === "<") {
      const close = src.indexOf(">", i);
      // Close around tags; the next character reopens if it's still changed.
      if (open) out += "</mark>";
      open = false;
      out += src.slice(i, close + 1);
      i = close + 1;
      continue;
    }
    // One character of text (an entity counts as one).
    let ch = src[i];
    if (ch === "&") {
      const semi = src.indexOf(";", i);
      if (semi > i && semi - i < 10) ch = src.slice(i, semi + 1);
    }
    const want = inRange(pos);
    if (want && !open) {
      out += '<mark class="wd">';
      open = true;
    } else if (!want && open) {
      out += "</mark>";
      open = false;
    }
    out += ch;
    i += ch.length;
    pos++;
  }
  if (open) out += "</mark>";
  return out;
}
