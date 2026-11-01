// Issue-tracker links in commit messages: "ENR-194" → your Jira, "#42" →
// the GitHub/GitLab issue. Rules come from Preferences → Commits, plus an
// automatic one for #123 when origin is on GitHub, GitLab or Codeberg.

import { app } from "./state.svelte";

export interface IssueRule { pattern: string; url: string }
export type Piece = { text: string; href?: string };

/** The web address of a hosted remote: git@github.com:o/r.git → https://github.com/o/r */
export function webUrl(remote: string): string | null {
  const m =
    remote.match(/^git@([^:]+):(.+?)(?:\.git)?\/?$/) ??
    remote.match(/^ssh:\/\/(?:[^@]+@)?([^/:]+)(?::\d+)?\/(.+?)(?:\.git)?\/?$/) ??
    remote.match(/^https?:\/\/(?:[^@/]+@)?([^/]+)\/(.+?)(?:\.git)?\/?$/);
  return m ? `https://${m[1]}/${m[2]}` : null;
}

function autoRule(): IssueRule | null {
  const origin = app.refs?.remotes.find((r) => r.name === "origin") ?? app.refs?.remotes[0];
  const web = origin ? webUrl(origin.url) : null;
  if (!web || !/github\.com|gitlab\.|codeberg\.org|gitea/.test(web)) return null;
  return { pattern: "#(\\d+)", url: `${web}/issues/$1` };
}

const cache = new Map<string, RegExp | null>();
function compile(p: string): RegExp | null {
  if (!cache.has(p)) {
    try {
      cache.set(p, new RegExp(p, "g"));
    } catch {
      cache.set(p, null);
    }
  }
  return cache.get(p)!;
}

export function rules(): IssueRule[] {
  const user = app.settings.issueLinks.filter((r) => r.pattern.trim() && r.url.trim());
  const auto = autoRule();
  return auto && !user.some((r) => r.pattern === auto.pattern) ? [...user, auto] : user;
}

/** Split text into plain pieces and linked issue keys. */
export function linkify(text: string, rs: IssueRule[] = rules()): Piece[] {
  type Hit = { start: number; end: number; href: string };
  const hits: Hit[] = [];
  for (const r of rs) {
    const re = compile(r.pattern);
    if (!re) continue;
    re.lastIndex = 0;
    for (const m of text.matchAll(re)) {
      if (!m[0]) continue;
      const start = m.index!, end = start + m[0].length;
      if (hits.some((h) => start < h.end && end > h.start)) continue;
      const href = r.url.replace(/\$(\d)/g, (_, d) => encodeURIComponent(m[Number(d)] ?? ""));
      hits.push({ start, end, href });
    }
  }
  if (!hits.length) return [{ text }];
  hits.sort((a, b) => a.start - b.start);
  const out: Piece[] = [];
  let at = 0;
  for (const h of hits) {
    if (h.start > at) out.push({ text: text.slice(at, h.start) });
    out.push({ text: text.slice(h.start, h.end), href: h.href });
    at = h.end;
  }
  if (at < text.length) out.push({ text: text.slice(at) });
  return out;
}
