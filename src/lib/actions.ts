// User-facing operations that need confirmation, naming or follow-up
// choices. Menus, buttons and drag-and-drop all call into these.

import { open as pickFile, save as pickSaveFile } from "@tauri-apps/plugin-dialog";
import { api, type CommitRow, type MergeOutcome, type RebaseAction, type Step } from "./api";
import type { Payload } from "./drag.svelte";
import { relTime } from "./format";
import { keyText } from "./platform";
import { app } from "./state.svelte";
import { dialog, menu, toasts, type MenuItem } from "./ui.svelte";

const P = () => app.path;

function mergeMessage(m: MergeOutcome | undefined, source: string): string | null {
  if (!m) return null;
  switch (m.result) {
    case "up-to-date": return `Already up to date with ${source}.`;
    case "fast-forward": return `Fast-forwarded to ${source}.`;
    case "merged": return `Merged ${source}.`;
    case "conflicts": return null;
  }
}

function afterMerge(m: MergeOutcome | undefined) {
  if (m?.result === "conflicts") {
    app.view = "changes";
    toasts.push("error", `The merge stopped on ${m.conflicts.length} conflicted file${m.conflicts.length === 1 ? "" : "s"}. Resolve them, then commit.`);
  }
}

const validBranch = (v: string) =>
  !v ? "Enter a name." : /\s|\.\.|[~^:?*\[\\]|^\/|\/$|\.lock$|^-/.test(v) ? "Branch names can't contain spaces or ~ ^ : ? * [ \\ or '..'." : null;

// ------------------------------------------------------------------ remote

export async function fetchAll() {
  try {
    await app.run("Fetching", (c) => api.fetch(P(), null, true, c), { success: "Fetched from all remotes." });
    app.noteFetched(P());
  } catch {
    // run() already showed the error.
  }
}

export async function pull(): Promise<boolean> {
  const r = await app.act("Pulling", (c) => api.pull(P(), app.settings.pullRebase, c), {
    success: (r) => (r.result === "up-to-date" ? "Already up to date." : r.result === "conflicts" ? null : `Pulled (${r.result.replace("-", " ")}).`),
  });
  if (r) app.noteFetched(P());
  if (r?.result === "conflicts") afterMerge({ result: "conflicts", conflicts: r.conflicts });
  return !!r && r.result !== "conflicts";
}

/** Push; resolves to true when the branch ended up on the remote. */
export async function push(branch?: string, remote?: string): Promise<boolean> {
  const target = branch ?? app.headBranch ?? undefined;
  try {
    await app.run("Pushing", (c) => api.push(P(), { branch: target, remote }, c), {
      quiet: true,
      success: (dest) => `Pushed to ${dest}.`,
    });
    return true;
  } catch (e) {
    const err = String(e);
    if (err.startsWith("REJECTED:")) {
      const r = await dialog.show({
        title: "The push was rejected",
        body: err.slice(9) + "\n\nPull to bring in the remote's commits, or force push to replace the remote branch with yours.",
        confirm: "Pull",
        extra: [{ id: "force", label: "Force push…", danger: true }],
      });
      if (r.button === "confirm") return (await pull()) && false;
      if (r.button === "force") return forcePush(target, remote);
    } else if (!err.startsWith("AUTH")) {
      toasts.push("error", err.replace(/^\w+_?\w*:/, ""));
    }
    return false;
  }
}

/**
 * Force push "with lease", like `git push --force-with-lease`: it only
 * overwrites the remote branch if nobody pushed to it since the last fetch.
 */
export async function forcePush(branch?: string, remote?: string): Promise<boolean> {
  const target = branch ?? app.headBranch ?? undefined;
  try {
    await app.run("Force pushing", (c) => api.push(P(), { branch: target, remote, force: true }, c), {
      quiet: true,
      success: (d) => `Force pushed to ${d}.`,
    });
    return true;
  } catch (e) {
    const err = String(e);
    if (err.startsWith("LEASE:")) {
      const r = await dialog.show({
        title: "Someone else pushed to this branch",
        body: err.slice(6) + "\n\nFetch to see their commits first. Overwrite only if you're sure their work isn't needed.",
        confirm: "Fetch",
        extra: [{ id: "overwrite", label: "Overwrite their commits", danger: true }],
      });
      if (r.button === "confirm") await fetchAll();
      if (r.button === "overwrite") {
        const ok = await app.act("Force pushing", (c) => api.push(P(), { branch: target, remote, force: true, lease: false }, c), {
          success: (d) => `Force pushed to ${d}.`,
        });
        return ok !== undefined;
      }
    } else if (!err.startsWith("AUTH")) {
      toasts.push("error", err.replace(/^\w+_?\w*:/, ""));
    }
    return false;
  }
}

/**
 * Sync, like Tower: pull the remote's commits, then push yours. A branch
 * that doesn't track a remote branch yet is just pushed (and starts tracking).
 */
export async function sync() {
  const st = app.status;
  if (!app.headBranch) return toasts.push("info", "Check out a branch to sync it.");
  if (st?.upstream) {
    const r = await app.act("Syncing: pulling", (c) => api.pull(P(), app.settings.pullRebase, c));
    app.noteFetched(P());
    if (r === undefined) return;
    if (r.result === "conflicts") return afterMerge({ result: "conflicts", conflicts: r.conflicts });
    await app.refresh();
    if (!app.status?.ahead) {
      return toasts.push("success", r.result === "up-to-date" ? "Already in sync." : `Pulled (${r.result.replace("-", " ")}); nothing to push.`);
    }
  }
  await push();
}

// ---------------------------------------------------------------- branches

export async function checkout(name: string) {
  await app.act(`Switching to ${name}`, () => api.checkoutBranch(P(), name), { success: `Switched to ${name}.` });
}

export async function checkoutRemote(fullName: string) {
  await app.act(`Checking out ${fullName}`, () => api.checkoutRemote(P(), fullName), { success: (n) => `Switched to ${n}, tracking ${fullName}.` });
}

export async function newBranch(from: string | null = null, fromLabel?: string) {
  const r = await dialog.show({
    title: "New branch",
    body: fromLabel ? `Starts at ${fromLabel}.` : "Starts at the current commit.",
    fields: [
      { key: "name", label: "Name", placeholder: "feature/short-description" },
      { key: "checkout", label: "Check it out", type: "checkbox", checked: true },
    ],
    confirm: "Create branch",
    validate: (v) => validBranch(String(v.name ?? "").trim()),
  });
  if (r.button !== "confirm") return;
  const name = String(r.values.name).trim();
  await app.act("Creating branch", () => api.createBranch(P(), name, from, !!r.values.checkout), { success: `Created ${name}.` });
}

export async function renameBranch(name: string) {
  const to = await dialog.prompt("Rename branch", "New name", { value: name, confirm: "Rename", validate: validBranch });
  if (!to || to === name) return;
  await app.act("Renaming", () => api.renameBranch(P(), name, to), { success: `Renamed to ${to}.` });
}

/** Delete one or more local branches after one confirmation. Resolves to the
 *  names that were deleted (empty when cancelled). */
export async function deleteBranches(names: string[]): Promise<string[]> {
  const head = app.headBranch;
  const list = names.filter((n) => n !== head);
  if (!list.length) {
    toasts.push("error", `${head} is checked out, so it can't be deleted. Check out another branch first.`);
    return [];
  }
  let unmerged: string[];
  try {
    unmerged = await api.unmergedBranches(P(), list);
  } catch (e) {
    toasts.push("error", String(e));
    return [];
  }
  const merged = list.filter((n) => !unmerged.includes(n));
  const one = list.length === 1;
  const into = head ? `merged into ${head}` : "merged into the current commit";
  let body = !unmerged.length
    ? `${one ? "It's" : "They're all"} ${into} or pushed, so no commits are lost.`
    : one
      ? `It has commits that aren't ${into} or pushed anywhere.`
      : `${unmerged.length === list.length ? "All of them have" : `${unmerged.length} of them ${unmerged.length === 1 ? "has" : "have"}`} commits that aren't ${into} or pushed anywhere.`;
  body += keyText(` Undo (Ctrl+Z) brings ${one ? "it" : "them"} back.`);
  if (names.length > list.length) body += `\n\n${head} is checked out, so it's kept.`;

  const r = await dialog.show({
    title: one ? `Delete branch ${list[0]}?` : `Delete ${list.length} branches?`,
    body,
    items: one ? undefined : list.map((n) => ({ text: n, note: unmerged.includes(n) ? "unmerged" : "merged", warn: unmerged.includes(n) })),
    confirm: unmerged.length ? (one ? "Delete anyway" : `Delete all ${list.length}`) : one ? "Delete" : `Delete ${list.length} branches`,
    danger: true,
    extra: unmerged.length && merged.length ? [{ id: "merged", label: `Delete only merged (${merged.length})` }] : undefined,
  });
  const target = r.button === "confirm" ? list : r.button === "merged" ? merged : [];
  if (!target.length) return [];
  const n = await app.act("Deleting branches", () => api.deleteBranches(P(), target, true), {
    success: target.length === 1 ? `Deleted ${target[0]}. Undo brings it back.` : `Deleted ${target.length} branches. Undo brings them back.`,
  });
  return n === undefined ? [] : target;
}

export const deleteBranch = (name: string) => deleteBranches([name]);

export async function deleteRemoteBranch(fullName: string) {
  const ok = await dialog.confirm("Delete remote branch?", `${fullName} will be deleted on the server. This can't be undone from here.`, "Delete on remote", true);
  if (ok) await app.act("Deleting remote branch", (c) => api.deleteRemoteBranch(P(), fullName, c), { success: `Deleted ${fullName}.` });
}

export async function setUpstream(branch: string) {
  const options = app.refs?.remotes.flatMap((r) => r.branches.map((b) => b.fullName)) ?? [];
  const current = app.refs?.local.find((b) => b.name === branch)?.upstream ?? "";
  const v = await dialog.prompt("Track remote branch", "Remote branch", {
    value: current || options.find((o) => o.endsWith(`/${branch}`)) || "",
    confirm: "Save",
    body: options.length ? `For example: ${options.slice(0, 3).join(", ")}` : undefined,
  });
  if (v === null) return;
  await app.act("Setting upstream", () => api.setUpstream(P(), branch, v || null), { success: v ? `${branch} now tracks ${v}.` : `${branch} no longer tracks a remote branch.` });
}

export async function mergeInto(source: string, target: string) {
  const here = app.headBranch;
  const m = await app.act(`Merging ${source}`, () => (target === here ? api.merge(P(), source) : api.mergeInto(P(), source, target)), {
    success: (m) => mergeMessage(m, source),
  });
  afterMerge(m);
}

export async function rebaseOnto(branch: string, onto: string) {
  const here = app.headBranch;
  await app.act(`Rebasing ${branch}`, () => (branch === here ? api.rebase(P(), onto) : api.rebaseBranch(P(), branch, onto)), {
    success: (r) => (r.result === "up-to-date" ? `${branch} already contains ${onto}.` : `Rebased ${branch} onto ${onto}.`),
  });
}

export function branchMenu(e: MouseEvent, name: string) {
  const head = app.headBranch;
  const b = app.refs?.local.find((x) => x.name === name);
  const items: MenuItem[] = [
    { label: `Check out ${name}`, disabled: name === head, action: () => checkout(name) },
    { separator: true },
    { label: `Merge ${name} into ${head}`, disabled: !head || name === head, action: () => mergeInto(name, head!) },
    { label: `Rebase ${head} onto ${name}`, disabled: !head || name === head, action: () => rebaseOnto(head!, name) },
    { separator: true },
    { label: "Push", action: () => push(name) },
    { label: "Track remote branch…", action: () => setUpstream(name) },
    { separator: true },
    { label: "New branch from here…", action: () => newBranch(name, name) },
    { label: "Check out in a new worktree…", disabled: name === head, action: () => addWorktree(name) },
    { label: "Rename…", action: () => renameBranch(name) },
    { label: "Show history", action: () => showHistory(name) },
    { label: `Compare with ${head ?? "HEAD"}`, disabled: name === head, action: () => compareWith(name) },
    { label: "Browse files", action: () => browseAt(name, name) },
    { label: "Export as ZIP…", action: () => exportZip(name, name) },
    { separator: true },
    { label: "Delete…", danger: true, disabled: name === head, action: () => deleteBranch(name) },
  ];
  if (b?.upstreamGone) items.splice(7, 0, { label: "Upstream was deleted on the remote", disabled: true });
  menu.fromEvent(e, items, name);
}

export function remoteBranchMenu(e: MouseEvent, fullName: string) {
  const head = app.headBranch;
  menu.fromEvent(e, [
    { label: "Check out as local branch", action: () => checkoutRemote(fullName) },
    { separator: true },
    { label: `Merge into ${head}`, disabled: !head, action: () => mergeInto(fullName, head!) },
    { label: `Rebase ${head} onto it`, disabled: !head, action: () => rebaseOnto(head!, fullName) },
    { separator: true },
    { label: "Show history", action: () => showHistory(fullName) },
    { label: `Compare with ${head ?? "HEAD"}`, action: () => compareWith(fullName) },
    { label: "Browse files", action: () => browseAt(fullName, fullName) },
    { label: "Delete on remote…", danger: true, action: () => deleteRemoteBranch(fullName) },
  ], fullName);
}

export function showHistory(scope: string) {
  app.scope = scope;
  app.view = "history";
  app.loadLog();
}

// ------------------------------------------------- compare, browse, export

/** Compare `target` with `base` (the current branch by default). */
export function compareWith(target: string, base?: string) {
  const b = base ?? app.headBranch ?? app.status?.head ?? "HEAD";
  if (b === target) return toasts.push("info", `Pick a branch other than ${target} to compare it with.`);
  app.compare = { base: b, target };
  app.view = "compare";
}

/** Show the repository's files as they were at `rev`. */
export function browseAt(rev: string, label: string) {
  app.browse = { rev, label };
  app.view = "browse";
}

/** Save the files at `rev` as a ZIP archive. */
export async function exportZip(rev: string, label: string) {
  const name = `${app.repo?.name ?? "repository"}-${slug(label)}`;
  const dest = await pickSaveFile({ title: `Export ${label} as ZIP`, defaultPath: `${name}.zip`, filters: [{ name: "ZIP archive", extensions: ["zip"] }] });
  if (!dest) return;
  await app.act("Exporting ZIP", () => api.exportZip(P(), rev, dest, name), {
    success: (n) => `Exported ${n} file${n === 1 ? "" : "s"} to ${fileName(dest)}.`,
  });
}

// -------------------------------------------------------------------- tags

export async function newTag(target: string, label: string) {
  const r = await dialog.show({
    title: "New tag",
    body: `Tags ${label}.`,
    fields: [
      { key: "name", label: "Name", placeholder: "v1.2.0" },
      { key: "message", label: "Message (optional — makes an annotated tag)", type: "textarea" },
    ],
    confirm: "Create tag",
    validate: (v) => (String(v.name ?? "").trim() ? null : "Enter a tag name."),
  });
  if (r.button !== "confirm") return;
  const name = String(r.values.name).trim();
  const msg = String(r.values.message ?? "").trim() || null;
  await app.act("Creating tag", () => api.createTag(P(), name, target, msg), { success: `Created tag ${name}.` });
}

export function tagMenu(e: MouseEvent, name: string, oid: string) {
  const remotes = app.refs?.remotes ?? [];
  menu.fromEvent(e, [
    { label: "Show commit", action: () => selectCommit(oid) },
    { label: "Check out", action: () => app.act("Checking out", () => api.checkoutCommit(P(), oid)) },
    { label: "Browse files", action: () => browseAt(oid, name) },
    { label: "Export as ZIP…", action: () => exportZip(oid, name) },
    { label: `Compare with ${app.headBranch ?? "HEAD"}`, action: () => compareWith(name) },
    ...remotes.map((r) => ({ label: `Push to ${r.name}`, action: () => app.act("Pushing tag", (c) => api.pushTag(P(), name, r.name, c), { success: `Pushed ${name} to ${r.name}.` }) })),
    { separator: true },
    { label: "Delete", danger: true, action: () => app.act("Deleting tag", () => api.deleteTag(P(), name), { success: `Deleted tag ${name}.` }) },
  ], name);
}

// ----------------------------------------------------------------- remotes

export async function addRemote() {
  const r = await dialog.show({
    title: "Add remote",
    fields: [
      { key: "name", label: "Name", value: app.refs?.remotes.length ? "" : "origin" },
      { key: "url", label: "URL", placeholder: "git@github.com:you/project.git" },
    ],
    confirm: "Add remote",
    validate: (v) => (!String(v.name ?? "").trim() ? "Enter a name." : !String(v.url ?? "").trim() ? "Enter a URL." : null),
  });
  if (r.button !== "confirm") return;
  const name = String(r.values.name).trim();
  await app.act("Adding remote", () => api.addRemote(P(), name, String(r.values.url).trim()), { success: `Added ${name}.` });
  await app.act("Fetching", (c) => api.fetch(P(), name, false, c));
}

export function remoteMenu(e: MouseEvent, name: string) {
  menu.fromEvent(e, [
    { label: `Fetch ${name}`, action: () => app.act("Fetching", (c) => api.fetch(P(), name, true, c), { success: `Fetched ${name}.` }) },
    { separator: true },
    { label: "Remove remote", danger: true, action: async () => {
      if (await dialog.confirm(`Remove ${name}?`, "Its remote-tracking branches are removed too. Undo restores the remote.", "Remove", true))
        await app.act("Removing remote", () => api.removeRemote(P(), name), { success: `Removed ${name}.` });
    } },
  ], name);
}

// ----------------------------------------------------------------- commits

export function selectCommit(oid: string) {
  app.view = "history";
  app.selectedCommit = oid;
  if (!app.log.some((r) => r.oid === oid)) app.loadLog();
}

/**
 * The commits on the current branch that can be rewritten, oldest first:
 * the first-parent chain from HEAD back to (not including) the first merge
 * commit. Commits already pushed are allowed but confirmed first.
 */
export function rewritable(): CommitRow[] {
  const head = app.status?.head;
  if (!head) return [];
  const byId = new Map(app.log.map((r) => [r.oid, r]));
  const out: CommitRow[] = [];
  let cur = byId.get(head);
  while (cur) {
    if (cur.parents.length > 1) break;
    out.push(cur);
    cur = cur.parents[0] ? byId.get(cur.parents[0]) : undefined;
  }
  return out.reverse();
}

/** Commits already pushed to the upstream (rewriting them needs a force push). */
export function pushedSet(): Set<string> {
  const up = app.status?.upstream;
  const set = new Set<string>();
  if (!up) return set;
  const start = app.log.find((r) => r.refs.some((l) => l.kind === "remote" && l.name === up));
  if (!start) return set;
  const byId = new Map(app.log.map((r) => [r.oid, r]));
  const stack = [start.oid];
  while (stack.length) {
    const id = stack.pop()!;
    if (set.has(id)) continue;
    set.add(id);
    const r = byId.get(id);
    if (r) stack.push(...r.parents);
  }
  return set;
}

async function confirmRewritePushed(oids: string[]): Promise<boolean> {
  const pushed = pushedSet();
  if (!oids.some((o) => pushed.has(o))) return true;
  return dialog.confirm(
    "Rewrite pushed commits?",
    "Some of these commits are already on the remote. After rewriting them you'll need to force push, which affects anyone else using this branch.",
    "Rewrite anyway",
    true,
  );
}

/** Run an interactive rebase over all rewritable commits, changing only `edits`. */
export async function rewrite(label: string, edit: (steps: Step[]) => Step[] | null, touched: string[], done?: string) {
  const commits = rewritable();
  if (!commits.length) return;
  const base = commits[0].parents[0] ?? null;
  let steps: Step[] = commits.map((c) => ({ oid: c.oid, action: "pick" as RebaseAction }));
  const edited = edit(steps);
  if (!edited) return;
  steps = edited;
  if (!(await confirmRewritePushed(touched))) return;
  await app.act(label, () => api.interactiveRebase(P(), base, steps, label), { success: done ?? `${label}: done.` });
}

export async function reword(c: CommitRow) {
  const info = await api.commitInfo(P(), c.oid);
  const full = info.body ? `${info.summary}\n\n${info.body}` : info.summary;
  const r = await dialog.show({
    title: "Edit commit message",
    fields: [{ key: "m", label: "Message", type: "textarea", value: full }],
    confirm: "Save message",
    validate: (v) => (String(v.m ?? "").trim() ? null : "The message can't be empty."),
  });
  if (r.button !== "confirm") return;
  const message = String(r.values.m);
  await rewrite(`Edit message of “${c.summary}”`, (steps) => steps.map((s) => (s.oid === c.oid ? { ...s, action: "reword", message } : s)), [c.oid], "Message updated.");
}

export async function combineWithParent(c: CommitRow, action: "squash" | "fixup") {
  const list = rewritable();
  const i = list.findIndex((x) => x.oid === c.oid);
  if (i <= 0) {
    toasts.push("error", "This commit has no earlier commit on the branch to combine with.");
    return;
  }
  await rewrite(`${action === "squash" ? "Squash" : "Fixup"} “${c.summary}” into “${list[i - 1].summary}”`,
    (steps) => steps.map((s) => (s.oid === c.oid ? { ...s, action } : s)), [c.oid, list[i - 1].oid], "Commits combined.");
}

export async function dropCommit(c: CommitRow) {
  const ok = await dialog.confirm("Remove this commit?", `"${c.summary}" and its changes will be removed from the branch. Undo brings it back.`, "Remove commit", true);
  if (!ok) return;
  await rewrite(`Remove “${c.summary}”`, (steps) => steps.map((s) => (s.oid === c.oid ? { ...s, action: "drop" } : s)), [c.oid], "Commit removed.");
}

/** Move commit `oid` to sit directly after (on top of) `afterOid` (null = first). */
export async function moveCommit(oid: string, afterOid: string | null) {
  const list = rewritable();
  const from = list.findIndex((c) => c.oid === oid);
  if (from < 0) return;
  const order = list.map((c) => c.oid).filter((o) => o !== oid);
  const at = afterOid === null ? 0 : order.indexOf(afterOid) + 1;
  order.splice(at, 0, oid);
  if (order.every((o, i) => o === list[i].oid)) return;
  const touched = list.slice(Math.min(from, at)).map((c) => c.oid);
  const moved = list[from];
  await rewrite(`Move “${moved.summary}”`, () => order.map((o) => ({ oid: o, action: "pick" as RebaseAction })), touched, "Commit moved.");
}

export async function resetTo(c: Pick<CommitRow, "oid" | "short">) {
  const r = await dialog.show({
    title: `Reset ${app.headBranch ?? "HEAD"} to ${c.short}`,
    body: "Moves the branch to this commit. Choose what happens to the changes in the commits after it.",
    confirm: "Keep changes",
    extra: [
      { id: "soft", label: "Keep changes staged" },
      { id: "hard", label: "Discard everything", danger: true },
    ],
  });
  const mode = r.button === "confirm" ? "mixed" : r.button === "soft" ? "soft" : r.button === "hard" ? "hard" : null;
  if (!mode) return;
  if (mode === "hard" && !(await dialog.confirm("Discard all changes?", "Uncommitted changes in your working copy are lost too, and only the branch move can be undone.", "Discard", true))) return;
  await app.act("Resetting", () => api.reset(P(), c.oid, mode), { success: `Reset to ${c.short}.` });
}

export function commitMenu(e: MouseEvent, c: CommitRow, openRebase: (from: CommitRow) => void) {
  const head = app.headBranch;
  const list = rewritable();
  const canRewrite = list.some((x) => x.oid === c.oid);
  const isFirst = list[0]?.oid === c.oid;
  menu.fromEvent(e, [
    { label: "Check out this commit", action: () => app.act("Checking out", () => api.checkoutCommit(P(), c.oid), { success: `Checked out ${c.short} (detached).` }) },
    { label: "New branch here…", action: () => newBranch(c.oid, c.short) },
    { label: "New tag here…", action: () => newTag(c.oid, c.short) },
    { separator: true },
    { label: `Cherry-pick into ${head ?? "HEAD"}`, disabled: c.oid === app.status?.head, action: () => app.act("Cherry-picking", () => api.cherryPick(P(), c.oid), { success: (r) => `Cherry-picked as ${r.short}.` }) },
    { label: "Revert commit", action: () => app.act("Reverting", () => api.revert(P(), c.oid), { success: (r) => `Reverted in ${r.short}.` }) },
    { label: `Reset ${head ?? "HEAD"} to here…`, action: () => resetTo(c) },
    { separator: true },
    { label: "Edit message…", disabled: !canRewrite, action: () => reword(c) },
    { label: "Squash into parent", disabled: !canRewrite || isFirst, action: () => combineWithParent(c, "squash") },
    { label: "Fixup into parent", hint: "drops this message", disabled: !canRewrite || isFirst, action: () => combineWithParent(c, "fixup") },
    { label: "Remove commit…", danger: true, disabled: !canRewrite, action: () => dropCommit(c) },
    { label: "Rewrite from here…", disabled: !canRewrite, action: () => openRebase(c) },
    { separator: true },
    { label: "Browse files at this commit", action: () => browseAt(c.oid, c.short) },
    { label: "Export as ZIP…", action: () => exportZip(c.oid, c.short) },
    { label: "Save as patch…", disabled: c.parents.length > 1, action: () => saveCommitPatch(c) },
    { label: "Copy commit ID", action: () => navigator.clipboard.writeText(c.oid).then(() => toasts.push("info", "Copied commit ID.")) },
  ], `${c.short} ${c.summary}`);
}

// --------------------------------------------------------- files & patches

const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 50) || "changes";

/** Escape gitignore's special characters so a file name matches only itself. */
const literal = (s: string) => s.replace(/[\\*?[\]]/g, (c) => `\\${c}`).replace(/^([#!])/, "\\$1").replace(/ $/, "\\ ");

/** "Ignore…" menu entries for a working-copy file. */
export function ignoreItems(path: string): MenuItem[] {
  const cut = path.lastIndexOf("/");
  const base = path.slice(cut + 1);
  const dir = cut > 0 ? path.slice(0, cut) : "";
  const ext = /[^.]\.([A-Za-z0-9_-]{1,12})$/.exec(base)?.[1];
  return [
    { label: `Ignore ${base}`, action: () => ignore(`/${literal(path)}`) },
    ...(ext ? [{ label: `Ignore all .${ext} files`, action: () => ignore(`*.${ext}`) }] : []),
    ...(dir ? [{ label: `Ignore the folder ${dir}/`, action: () => ignore(`/${literal(dir)}/`) }] : []),
  ];
}

export async function ignore(pattern: string) {
  const added = await app.act("Updating .gitignore", () => api.ignorePattern(P(), pattern));
  if (added !== undefined) toasts.push(added ? "success" : "info", added ? `Added ${pattern} to .gitignore.` : `${pattern} is already in .gitignore.`);
}

export async function untrack(paths: string[]) {
  const what = paths.length === 1 ? paths[0] : `${paths.length} files`;
  await app.act("Stopping tracking", () => api.untrackFiles(P(), paths), {
    success: `Git no longer tracks ${what}; the files stay on disk. Commit to record it, and add them to .gitignore to keep them out.`,
  });
}

/** Put a file back to how it was at `rev` (e.g. a commit, or "<commit>^" for the version before it). */
export async function restoreFile(rev: string, path: string, when: string) {
  const base = path.slice(path.lastIndexOf("/") + 1);
  await app.act("Restoring", () => api.restoreFile(P(), rev, path), {
    success: `Restored ${base} ${when}. It's an ordinary change in your working copy now.`,
  });
}

const fileName = (p: string) => p.split(/[\\/]/).pop();

export async function saveCommitPatch(c: { oid: string; short: string; summary: string }) {
  const dest = await pickSaveFile({ title: "Save commit as patch", defaultPath: `${c.short}-${slug(c.summary)}.patch`, filters: [{ name: "Patch", extensions: ["patch", "diff"] }] });
  if (!dest) return;
  await app.act("Saving patch", () => api.saveCommitPatch(P(), c.oid, dest), { success: `Saved ${c.short} as ${fileName(dest)}.` });
}

export async function saveChangesPatch() {
  const st = app.status;
  const staged = st?.staged.length ?? 0, unstaged = st?.unstaged.length ?? 0;
  if (!staged && !unstaged) return toasts.push("info", "There are no uncommitted changes to save.");
  let useStaged = staged > 0 && !unstaged;
  if (staged && unstaged) {
    const r = await dialog.show({
      title: "Save changes as a patch",
      body: "Which changes should go into the patch?",
      confirm: `Staged (${staged} file${staged === 1 ? "" : "s"})`,
      extra: [{ id: "unstaged", label: `Not staged (${unstaged})` }],
    });
    if (r.button === "cancel") return;
    useStaged = r.button === "confirm";
  }
  const name = app.headBranch ? `${slug(app.headBranch)}-changes.patch` : "changes.patch";
  const dest = await pickSaveFile({ title: "Save changes as patch", defaultPath: name, filters: [{ name: "Patch", extensions: ["patch", "diff"] }] });
  if (!dest) return;
  await app.act("Saving patch", () => api.saveChangesPatch(P(), useStaged, dest), {
    success: (n) => `Saved ${n} file${n === 1 ? "" : "s"} to ${fileName(dest)}.`,
  });
}

/** Apply a .patch file to the working copy; an email patch's message is put in the commit composer. */
export async function applyPatch() {
  const file = await pickFile({ title: "Apply a patch", multiple: false, directory: false, filters: [{ name: "Patch", extensions: ["patch", "diff", "eml", "txt"] }] });
  if (typeof file !== "string") return;
  const r = await app.act("Applying patch", () => api.applyPatch(P(), file));
  if (!r) return;
  app.view = "changes";
  const files = `${r.files} file${r.files === 1 ? "" : "s"}`;
  if (r.subject) app.prefillMessage = { subject: r.subject, body: r.body ?? "", at: Date.now() };
  toasts.push("success", r.subject ? `Applied the patch to ${files}. Its message is ready in the commit box.` : `Applied the patch to ${files}.`, {
    label: "Undo",
    run: () => app.undo(),
  });
}

// ------------------------------------------------------------------- stash

export async function stashChanges() {
  const s = app.status;
  if (s && !s.staged.length && !s.unstaged.length) {
    toasts.push("info", "There are no local changes to stash.");
    return;
  }
  const r = await dialog.show({
    title: "Stash changes",
    body: "Saves your changes and cleans the working copy.",
    fields: [
      { key: "message", label: "Description (optional)", placeholder: "What were you working on?" },
      { key: "untracked", label: "Include new files", type: "checkbox", checked: true },
      { key: "keep", label: "Keep staged changes in the working copy", type: "checkbox", checked: false },
    ],
    confirm: "Stash",
  });
  if (r.button !== "confirm") return;
  await app.act("Stashing", () => api.stashSave(P(), String(r.values.message ?? "").trim() || null, !!r.values.untracked, !!r.values.keep), {
    success: "Changes stashed.",
  });
}

export async function applyStash(index: number, pop: boolean) {
  await app.act(pop ? "Popping stash" : "Applying stash", () => (pop ? api.stashPop(P(), index) : api.stashApply(P(), index)), {
    success: pop ? "Stash applied and removed." : "Stash applied.",
  });
}

/** Delete one or more stashes (by list index) after a confirmation. Resolves
 *  to true when they were deleted. */
export async function dropStashes(indexes: number[]): Promise<boolean> {
  const items = app.stashes.filter((s) => indexes.includes(s.index));
  if (!items.length) return false;
  const one = items.length === 1;
  const ok = await dialog.show({
    title: one ? "Delete stash?" : `Delete ${items.length} stashes?`,
    body: one
      ? keyText(`"${items[0].message}" and its changes will be removed from the stash list. Undo (Ctrl+Z) brings it back.`)
      : keyText(`These stashes and their changes will be removed from the stash list. Undo (Ctrl+Z) brings them all back.`),
    items: one ? undefined : items.map((s) => ({ text: s.message, note: `${s.branch ? `on ${s.branch} · ` : ""}${relTime(s.time)}` })),
    confirm: one ? "Delete stash" : `Delete ${items.length} stashes`,
    danger: true,
  });
  if (ok.button !== "confirm") return false;
  const n = await app.act("Deleting stashes", () => api.stashDropMany(P(), items.map((s) => s.oid)), {
    success: one ? "Stash deleted. Undo brings it back." : `Deleted ${items.length} stashes. Undo brings them back.`,
  });
  return n !== undefined;
}

export const dropStash = (index: number) => dropStashes([index]);

export async function stashToBranch(index: number) {
  const name = await dialog.prompt("New branch from stash", "Branch name", { confirm: "Create branch", validate: validBranch });
  if (!name) return;
  await app.act("Creating branch", () => api.stashBranch(P(), index, name), { success: `Created ${name} with the stash applied.` });
}

// ---------------------------------------------------------- drag and drop

/** What happens when something is dropped on branch `target`. */
export function dropOnBranch(p: Payload, target: string, e: PointerEvent) {
  const head = app.headBranch;
  if (p.kind === "commit") {
    menu.open(e.clientX, e.clientY, [
      { label: `Cherry-pick into ${target}`, disabled: target !== head, hint: target !== head ? "check it out first" : undefined,
        action: () => app.act("Cherry-picking", () => api.cherryPick(P(), p.oid), { success: (r) => `Cherry-picked as ${r.short}.` }) },
    ], p.summary);
    return;
  }
  const src = p.kind === "branch" ? p.name : p.kind === "remote-branch" ? p.fullName : "";
  if (!src || src === target) return;
  menu.open(e.clientX, e.clientY, [
    { label: `Merge ${src} into ${target}`, action: () => mergeInto(src, target) },
    { label: `Rebase ${target} onto ${src}`, disabled: target.includes("/") && !app.refs?.local.some((b) => b.name === target), action: () => rebaseOnto(target, src) },
  ], `${src} → ${target}`);
}

export const acceptsOnBranch = (target: string, isHead: boolean) => (p: Payload) =>
  (p.kind === "branch" && p.name !== target) || (p.kind === "remote-branch" && p.fullName !== target) || (p.kind === "commit" && isHead);

// ---- Worktrees and submodules ----

/** The folder a new worktree goes in by default: next to the repository, "<repo>-<branch>". */
function worktreeFolder(branch: string): string {
  const main = app.path.replace(/[\\/]+$/, "");
  const sep = main.includes("\\") && !main.includes("/") ? "\\" : "/";
  const cut = Math.max(main.lastIndexOf("/"), main.lastIndexOf("\\"));
  const parent = main.slice(0, cut), name = main.slice(cut + 1);
  return `${parent}${sep}${name}-${branch.replace(/[^\w.-]+/g, "-")}`;
}

/** Check out a branch (an existing one, or a new one from HEAD) in its own folder. */
export async function addWorktree(branch?: string) {
  const r = await dialog.show({
    title: "New worktree",
    body: "Check out a branch in a separate folder, so you can work on it without switching branches here.",
    fields: [
      { key: "branch", label: "Branch", value: branch ?? "", placeholder: "An existing branch, or a new name" },
      { key: "folder", label: "Folder", placeholder: branch ? worktreeFolder(branch) : "Next to this repository, named after the branch", hint: "Leave empty for the suggested folder." },
      { key: "open", label: "Open it in a new tab", type: "checkbox", checked: true },
    ],
    confirm: "Create worktree",
    validate: (v) => {
      const b = String(v.branch ?? "").trim();
      if (!b) return "Name a branch.";
      return app.refs?.local.some((x) => x.name === b) ? null : validBranch(b);
    },
  });
  if (r.button !== "confirm") return;
  const name = String(r.values.branch).trim();
  const exists = !!app.refs?.local.some((x) => x.name === name);
  const folder = String(r.values.folder ?? "").trim() || worktreeFolder(name);
  const made = await app.act("Creating worktree", () => api.addWorktree(P(), folder, name, !exists, null), {
    success: `${exists ? "Checked out" : "Created"} ${name} in ${folder}.`,
  });
  if (made && r.values.open) await app.open(made);
}

export async function removeWorktree(w: { name: string | null; path: string; branch: string | null; changes: number; locked: string | null }) {
  if (!w.name) return;
  const ok = await dialog.show({
    title: "Remove worktree?",
    body: `Deletes the folder ${w.path}.${w.branch ? ` The branch ${w.branch} and its commits stay.` : ""}`,
    items: w.changes || w.locked != null ? [
      ...(w.changes ? [{ text: `${w.changes} uncommitted change${w.changes === 1 ? "" : "s"} will be lost`, warn: true }] : []),
      ...(w.locked != null ? [{ text: `It's locked${w.locked ? `: ${w.locked}` : ""}`, warn: true }] : []),
    ] : undefined,
    confirm: "Remove worktree",
    danger: true,
  });
  if (ok.button !== "confirm") return;
  const tab = app.tabs.findIndex((t) => t.path.replace(/[\\/]+$/, "") === w.path);
  if (tab >= 0) app.closeTab(tab);
  await app.act("Removing worktree", () => api.removeWorktree(P(), w.name!, true), { success: `Removed ${w.path}.` });
}

export async function updateSubmodules(paths: string[], label: string) {
  await app.act(`Updating ${label}`, (creds) => api.updateSubmodules(P(), paths, creds), {
    success: (n) => `Updated ${n} submodule${n === 1 ? "" : "s"}.`,
  });
}
