<script lang="ts">
  // Quick Actions (Ctrl+Shift+P): one search box for commands, branches,
  // tags, commits, files and repositories, like Tower's Quick Actions.
  import { api } from "../lib/api";
  import * as act from "../lib/actions";
  import { relTime } from "../lib/format";
  import { library } from "../lib/library.svelte";
  import { keyLabel, shortPath } from "../lib/platform";
  import { bindings, COMMANDS } from "../lib/shortcuts.svelte";
  import { app } from "../lib/state.svelte";
  import Icon from "./Icon.svelte";

  type Kind = "command" | "branch" | "remote" | "tag" | "commit" | "file" | "repo";
  interface Item {
    kind: Kind;
    key: string;
    label: string;
    hint: string;
    icon: string;
    score: number;
    run: () => unknown;
  }

  const KIND_LABEL: Record<Kind, string> = {
    command: "Commands", branch: "Branches", remote: "Remote branches", tag: "Tags",
    commit: "Commits", file: "Files", repo: "Repositories",
  };
  // How many of each kind to show while searching, and their order.
  const LIMIT: Record<Kind, number> = { command: 8, branch: 8, remote: 5, tag: 5, commit: 8, file: 10, repo: 6 };
  const ORDER: Kind[] = ["command", "branch", "remote", "tag", "commit", "file", "repo"];

  let query = $state("");
  let active = $state(0);
  let input = $state<HTMLInputElement>();
  let list = $state<HTMLDivElement>();
  let files = $state<string[]>([]);

  $effect(() => {
    input?.focus();
  });

  // Files and commits are loaded when the palette opens.
  $effect(() => {
    const path = app.path;
    if (!path) return;
    api.trackedFiles(path).then((f) => (files = f)).catch(() => (files = []));
    if (!app.log.length) app.loadLog();
  });

  /** Higher is better; -1 means no match. Substrings beat scattered letters, word starts beat the middle. */
  function score(text: string, q: string): number {
    if (!q) return 0;
    const n = text.toLowerCase();
    const i = n.indexOf(q);
    if (i >= 0) return 1000 - i * 2 - (n.length - q.length) * 0.05 + (i === 0 || " /-_.".includes(n[i - 1]) ? 80 : 0);
    let pos = 0;
    let gaps = 0;
    for (const ch of q) {
      if (ch === " ") continue;
      const at = n.indexOf(ch, pos);
      if (at < 0) return -1;
      gaps += at - pos;
      pos = at + 1;
    }
    return 500 - gaps;
  }

  const close = () => (app.quick = false);

  const items = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const all: Item[] = [];
    const add = (it: Omit<Item, "score">, ...texts: string[]) => {
      const s = Math.max(...texts.map((t) => score(t, q)));
      if (s >= 0) all.push({ ...it, score: s });
    };

    for (const c of COMMANDS) {
      if (c.id === "quick" || (c.repo && !app.repo)) continue;
      const k = bindings(c.id)[0];
      add({ kind: "command", key: c.id, label: c.label, hint: k ? keyLabel(k) : "", icon: "grid", run: c.run }, c.label, c.group);
    }
    if (app.repo) {
      for (const b of app.refs?.local ?? []) {
        add({
          kind: "branch", key: b.name, label: b.name, icon: "branch",
          hint: b.isHead ? "current branch" : `check out · ${relTime(b.time)}`,
          run: () => (b.isHead ? act.showHistory(b.name) : act.checkout(b.name)),
        }, b.name);
      }
      const locals = new Set((app.refs?.local ?? []).map((b) => b.name));
      for (const r of app.refs?.remotes ?? []) {
        for (const b of r.branches) {
          if (locals.has(b.name)) continue;
          add({ kind: "remote", key: b.fullName, label: b.fullName, icon: "remote", hint: "check out as local branch", run: () => act.checkoutRemote(b.fullName) }, b.fullName);
        }
      }
      for (const t of app.refs?.tags ?? []) {
        add({ kind: "tag", key: t.name, label: t.name, icon: "tag", hint: "show in history", run: () => act.selectCommit(t.oid) }, t.name);
      }
      if (q) {
        const hex = /^[0-9a-f]{4,40}$/.test(q);
        for (const c of app.log) {
          const it = { kind: "commit" as const, key: c.oid, label: c.summary || "(no message)", icon: "commit", hint: `${c.short} · ${c.author} · ${relTime(c.time)}`, run: () => act.selectCommit(c.oid) };
          if (hex && c.oid.startsWith(q)) all.push({ ...it, score: 2000 });
          else add(it, c.summary, c.author);
        }
        for (const f of files) {
          const base = f.slice(f.lastIndexOf("/") + 1);
          // Matching the file name counts for more than matching its folder.
          const sb = score(base, q);
          const s = Math.max(sb < 0 ? -1 : sb + 100, score(f, q));
          if (s >= 0) all.push({ kind: "file", key: f, label: base, hint: f, icon: "file", score: s, run: () => (app.fileHistoryPath = f) });
        }
      }
    }
    for (const r of library.repos) {
      if (r.path === app.path) continue;
      add({ kind: "repo", key: r.path, label: r.name, icon: "folder", hint: shortPath(r.path), run: () => app.open(r.path) }, r.name, r.path);
    }

    // Group by kind; within a kind, best match first.
    const out: Item[] = [];
    for (const k of ORDER) {
      const group = all.filter((i) => i.kind === k);
      if (q) group.sort((a, b) => b.score - a.score);
      const limit = q ? LIMIT[k] : k === "command" ? 100 : k === "repo" ? 6 : 0;
      out.push(...group.slice(0, limit));
    }
    // With a query, put the kind holding the single best match first.
    if (q && out.length) {
      const best = out.reduce((a, b) => (b.score > a.score ? b : a)).kind;
      return [...out.filter((i) => i.kind === best), ...out.filter((i) => i.kind !== best)];
    }
    return out;
  });

  $effect(() => {
    void query;
    active = 0;
  });

  function choose(it: Item | undefined) {
    if (!it) return;
    close();
    // Let the palette close before the action opens its own dialog or view.
    setTimeout(() => it.run(), 0);
  }

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "PageDown" || e.key === "PageUp") {
      e.preventDefault();
      const n = items.length;
      if (!n) return;
      const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : e.key === "PageDown" ? 8 : -8;
      active = e.key.startsWith("Page") ? Math.max(0, Math.min(n - 1, active + step)) : (active + step + n) % n;
      list?.querySelectorAll<HTMLElement>(".item")[active]?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter") {
      e.preventDefault();
      choose(items[active]);
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="backdrop" onclick={(e) => e.target === e.currentTarget && close()}>
  <div class="palette" role="dialog" aria-modal="true" aria-label="Quick actions">
    <div class="field">
      <Icon name="search" size={15} />
      <input bind:this={input} bind:value={query} onkeydown={key} spellcheck="false"
        placeholder={app.repo ? "Search commands, branches, tags, commits, files…" : "Search commands and repositories…"}
        role="combobox" aria-expanded="true" aria-controls="qa-list" aria-activedescendant="qa-{active}" />
    </div>
    <div class="list" id="qa-list" role="listbox" bind:this={list}>
      {#each items as it, i (it.kind + it.key)}
        {#if i === 0 || items[i - 1].kind !== it.kind}<div class="group">{KIND_LABEL[it.kind]}</div>{/if}
        <button class="item" id="qa-{i}" class:on={i === active} role="option" aria-selected={i === active}
          onclick={() => choose(it)} onpointermove={() => (active = i)}>
          <Icon name={it.icon} size={14} />
          <span class="name">{it.label}</span>
          <span class="hint" class:keys={it.kind === "command"} class:path={it.kind === "file" || it.kind === "repo"}><bdi>{it.hint}</bdi></span>
        </button>
      {:else}
        <div class="none">Nothing matches “{query}”.</div>
      {/each}
    </div>
    <div class="foot"><span><span class="kbd">↑</span> <span class="kbd">↓</span> to choose</span><span><span class="kbd">Enter</span> to run</span><span><span class="kbd">Esc</span> to close</span></div>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 95; background: var(--backdrop); display: flex; justify-content: center; align-items: flex-start; padding-top: 10vh; }
  .palette {
    width: min(640px, calc(100vw - 32px)); background: var(--surface); border: 1px solid var(--line);
    border-radius: 12px; box-shadow: var(--shadow-pop); overflow: hidden; display: flex; flex-direction: column;
    animation: drop 0.12s ease-out;
  }
  @keyframes drop { from { translate: 0 -6px; opacity: 0; } }
  .field { display: flex; align-items: center; gap: 10px; padding: 0 14px; border-bottom: 1px solid var(--line); color: var(--ink-3); }
  .field input { border: 0; background: transparent; box-shadow: none; padding: 13px 0; font-size: calc(var(--fs) * 15); }
  .list { max-height: 56vh; overflow-y: auto; padding: 4px 6px 6px; }
  .group { padding: 8px 10px 3px; font-size: calc(var(--fs) * 11); font-weight: 700; color: var(--ink-3); text-transform: uppercase; letter-spacing: 0.04em; }
  .item { display: flex; align-items: center; gap: 10px; width: 100%; padding: 6px 10px; border-radius: 7px; text-align: left; color: var(--ink-3); }
  .item.on { background: var(--sel); color: var(--ink-2); }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--ink); }
  .hint { font-size: calc(var(--fs) * 12); color: var(--ink-3); flex: none; max-width: 50%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* Long paths lose their start, not their file name. */
  .hint.path { direction: rtl; text-align: right; }
  .hint.keys { font-family: var(--font-mono); font-size: calc(var(--cs) * 11); }
  .none { padding: 16px; color: var(--ink-3); text-align: center; }
  .foot { display: flex; gap: 16px; padding: 7px 14px; border-top: 1px solid var(--line-soft); font-size: calc(var(--fs) * 11); color: var(--ink-3); }
</style>
