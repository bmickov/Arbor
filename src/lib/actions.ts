// User-facing operations that need confirmation, naming or follow-up
// choices. Menus, buttons and drag-and-drop all call into these.

import { open as pickFile, save as pickSaveFile } from "@tauri-apps/plugin-dialog";
import { api, type CommitRow, type MergeOutcome, type RebaseAction, type Step } from "./api";
import type { Payload } from "./drag.svelte";
import { relTime } from "./format";
import { branchUrl, commitUrl, fileUrl, hostOf, newPullRequestUrl, pullRequestsUrl, type Host } from "./hosting";
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

function afterMerge(m: { result?: string; conflicts: string[] } | undefined, what = "merge") {
  if (m?.result === "conflicts" || (m && what !== "merge" && m.conflicts.length)) {
    app.view = "changes";
    const n = m.conflicts.length;
    const then = what === "rebase" ? "choose Continue" : "commit";
    toasts.push("error", `The ${what} stopped on ${n} conflicted file${n === 1 ? "" : "s"}. Resolve them, then ${then}.`);
  }
}

/** Cherry-pick a commit onto the current branch; a conflict stops it to be resolved. */
export async function cherryPick(oid: string) {
  if (onHead(oid)) return toasts.push("info", `That commit is already on ${app.headBranch ?? "the current branch"}.`);
  const r = await app.act("Cherry-picking", () => api.cherryPick(P(), oid), { success: (r) => (r.conflicts.length ? null : `Cherry-picked as ${r.short}.`) });
  afterMerge(r, "cherry-pick");
}

export async function revertCommit(oid: string) {
  const r = await app.act("Reverting", () => api.revert(P(), oid), { success: (r) => (r.conflicts.length ? null : `Reverted in ${r.short}.`) });
  afterMerge(r, "revert");
}

/** Carry on with a paused rebase (after resolving), skip its commit, or abort it. */
export async function rebaseContinue() {
  const r = await app.act("Continuing rebase", () => api.rebaseContinue(P()), { success: (r) => (r.result === "conflicts" ? null : "Rebase finished. Undo reverts all of it.") });
  afterMerge(r, "rebase");
}
export async function rebaseSkip() {
  const p = app.status?.pausedRebase;
  if (p && !(await dialog.confirm("Skip this commit?", `${p.commit} “${p.summary}” will be left out of the rebased branch.`, "Skip commit", true))) return;
  const r = await app.act("Skipping commit", () => api.rebaseSkip(P()), { success: (r) => (r.result === "conflicts" ? null : "Rebase finished.") });
  afterMerge(r, "rebase");
}
export async function rebaseAbort() {
  await app.act("Aborting rebase", () => api.rebaseAbort(P()), { success: "Rebase aborted. Everything is as it was." });
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
  if (r?.result === "conflicts") afterMerge({ result: "conflicts", conflicts: r.conflicts }, app.settings.pullRebase ? "rebase" : "merge");
  else if (r && r.result !== "up-to-date" && app.lfs?.used && app.lfs.installed) await app.act("Downloading LFS files", () => api.lfsPull(P()));
  return !!r && r.result !== "conflicts";
}

/** Push; resolves to true when the branch ended up on the remote. */
export async function push(branch?: string, remote?: string, forPullRequest = false): Promise<boolean> {
  const target = branch ?? app.headBranch ?? undefined;
  // Publishing a branch for the first time: offer to open a pull request.
  const first = !!target && !app.refs?.local.find((b) => b.name === target)?.upstream;
  const host = first && !forPullRequest ? hostFor(remote ?? remoteOf(target)) : null;
  try {
    await app.run("Pushing", (c) => api.push(P(), { branch: target, remote }, c), {
      quiet: true,
      success: (dest) => {
        if (!host || !target) return `Pushed to ${dest}.`;
        toasts.push("success", `Published ${target} to ${dest.split("/")[0]}.`, { label: `Create ${host.pr}`, run: () => createPullRequest(target) }, 10000);
        return null;
      },
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
    if (r.result === "conflicts") return afterMerge({ result: "conflicts", conflicts: r.conflicts }, app.settings.pullRebase ? "rebase" : "merge");
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
  // A new branch starting at a local branch can be stacked on it.
  const local = (b: string | null) => (b && app.refs?.local.some((x) => x.name === b) ? b : null);
  const base = from ? local(from) : local(app.headBranch);
  const r = await dialog.show({
    title: "New branch",
    body: fromLabel ? `Starts at ${fromLabel}.` : "Starts at the current commit.",
    fields: [
      { key: "name", label: "Name", placeholder: "feature/short-description" },
      { key: "checkout", label: "Check it out", type: "checkbox", checked: true },
      ...(base ? [{ key: "stack", label: `Stack it on ${base}`, type: "checkbox" as const, checked: false, hint: `Keep it on top of ${base}: when ${base} changes, Restack replays this branch's commits on it.` }] : []),
    ],
    confirm: "Create branch",
    validate: (v) => validBranch(String(v.name ?? "").trim()),
  });
  if (r.button !== "confirm") return;
  const name = String(r.values.name).trim();
  const made = await app.act("Creating branch", async () => {
    await api.createBranch(P(), name, from, !!r.values.checkout);
    if (base && r.values.stack) await api.stackSetParent(P(), name, base);
    return true;
  }, { success: base && r.values.stack ? `Created ${name}, stacked on ${base}.` : `Created ${name}.` });
  void made;
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

/** Warn before a merge (or rebase) that's going to conflict. Resolves false to cancel. */
async function warnConflicts(source: string, target: string, what: "merge" | "rebase"): Promise<boolean> {
  let p;
  try {
    p = await api.predictMerge(P(), source, target);
  } catch {
    return true; // Can't tell; let the operation itself report problems.
  }
  if (p.result !== "conflicts") return true;
  const n = p.conflicts.length;
  const r = await dialog.show({
    title: what === "merge" ? `Merging ${source} will conflict` : `Rebasing will probably stop on conflicts`,
    body: `${source} and ${target} made conflicting changes to ${n} file${n === 1 ? "" : "s"}. You can go ahead and resolve ${n === 1 ? "it" : "them"} in the conflict wizard, or cancel.`,
    items: p.conflicts.slice(0, 50).map((c) => ({ text: c, warn: true })),
    confirm: what === "merge" ? "Merge anyway" : "Rebase anyway",
  });
  return r.button === "confirm";
}

export async function mergeInto(source: string, target: string) {
  const here = app.headBranch;
  if (!(await warnConflicts(source, target, "merge"))) return;
  const m = await app.act(`Merging ${source}`, () => (target === here ? api.merge(P(), source) : api.mergeInto(P(), source, target)), {
    success: (m) => mergeMessage(m, source),
  });
  afterMerge(m);
}

export async function rebaseOnto(branch: string, onto: string) {
  const here = app.headBranch;
  if (!(await warnConflicts(onto, branch, "rebase"))) return;
  const r = await app.act(`Rebasing ${branch}`, () => (branch === here ? api.rebase(P(), onto) : api.rebaseBranch(P(), branch, onto)), {
    success: (r) => (r.result === "conflicts" ? null : r.result === "up-to-date" ? `${branch} already contains ${onto}.` : `Rebased ${branch} onto ${onto}.`),
  });
  afterMerge(r, "rebase");
}

/** Stacked-branch items for a branch's menu (from the sidebar's stack list). */
let stacks: { branch: string; parent: string; behind: number }[] = [];
export function setStacks(s: typeof stacks) {
  stacks = s;
}
function stackItems(name: string): MenuItem[] {
  const me = stacks.find((s) => s.branch === name);
  const kids = stacks.filter((s) => s.parent === name).length;
  if (!me) return [{ label: "Stack on another branch…", action: () => stackOn(name) }];
  return [
    { label: `Restack onto ${me.parent}`, hint: me.behind ? `${me.behind} new` : "up to date", action: () => restack(name) },
    ...(kids ? [{ label: `Restack ${name} and branches on it`, action: () => restack(name, true) }] : []),
    { label: "Stack on another branch…", action: () => stackOn(name) },
    { label: "Remove from stack", action: () => unstack(name) },
  ];
}

export function branchMenu(e: MouseEvent, name: string) {
  const head = app.headBranch;
  const b = app.refs?.local.find((x) => x.name === name);
  const current = name === head;
  // The current branch can't be merged into, rebased onto or compared with
  // itself: leave those out rather than show "Merge main into main".
  const items: MenuItem[] = [
    { label: current ? `${name} is checked out` : `Check out ${name}`, disabled: current, action: () => checkout(name) },
    { separator: true },
    ...(current || !head ? [] : [
      { label: `Merge ${name} into ${head}`, action: () => mergeInto(name, head) },
      { label: `Rebase ${head} onto ${name}`, action: () => rebaseOnto(head, name) },
      { separator: true },
    ] as MenuItem[]),
    { label: "Push", action: () => push(name) },
    { label: "Track remote branch…", action: () => setUpstream(name) },
    { separator: true },
    { label: "New branch from here…", action: () => newBranch(name, name) },
    { label: "Check out in a new worktree…", disabled: name === head, action: () => addWorktree(name) },
    { label: pinnedBranches().includes(name) ? "Unpin" : "Pin to top", action: () => togglePin(name) },
    ...stackItems(name),
    { label: "Rename…", action: () => renameBranch(name) },
    { label: "Show history", action: () => showHistory(name) },
    ...(current ? [] : [{ label: `Compare with ${head ?? "HEAD"}`, action: () => compareWith(name) }]),
    { label: "Browse files", action: () => browseAt(name, name) },
    { label: "Export as ZIP…", action: () => exportZip(name, name) },
    ...branchHostItems(name),
    { separator: true },
    { label: "Delete…", danger: true, disabled: name === head, action: () => deleteBranch(name) },
  ];
  if (b?.upstreamGone) items.splice(items.findIndex((i) => "label" in i && i.label === "Track remote branch…") + 1, 0, { label: "Upstream was deleted on the remote", disabled: true });
  menu.fromEvent(e, items, name);
}

export function remoteBranchMenu(e: MouseEvent, fullName: string) {
  const head = app.headBranch;
  menu.fromEvent(e, [
    { label: "Check out as local branch", action: () => checkoutRemote(fullName) },
    { label: pinnedBranches().includes(fullName) ? "Unpin" : "Pin to top", action: () => togglePin(fullName) },
    { separator: true },
    { label: `Merge into ${head}`, disabled: !head, action: () => mergeInto(fullName, head!) },
    { label: `Rebase ${head} onto it`, disabled: !head, action: () => rebaseOnto(head!, fullName) },
    { separator: true },
    { label: "Show history", action: () => showHistory(fullName) },
    { label: `Compare with ${head ?? "HEAD"}`, action: () => compareWith(fullName) },
    { label: "Browse files", action: () => browseAt(fullName, fullName) },
    ...branchHostItems(fullName),
    { separator: true },
    { label: "Delete on remote…", danger: true, action: () => deleteRemoteBranch(fullName) },
  ], fullName);
}

export function showHistory(scope: string) {
  app.scope = scope;
  app.view = "history";
  app.loadLog();
}

// ----------------------------------------------- hosting: links and PRs

/** The remote a branch lives on: its upstream's, else origin, else the first. */
function remoteOf(branch?: string | null): string | null {
  const remotes = app.refs?.remotes ?? [];
  const local = branch ? app.refs?.local.find((b) => b.name === branch) : null;
  const up = local?.upstream?.split("/")[0];
  const byPrefix = branch ? remotes.find((r) => branch.startsWith(`${r.name}/`))?.name : null;
  return byPrefix ?? (up && remotes.some((r) => r.name === up) ? up : null) ?? remotes.find((r) => r.name === "origin")?.name ?? remotes[0]?.name ?? null;
}

/** The hosting service for a remote (GitHub, GitLab…), if it's one we know. */
export function hostFor(remote?: string | null): Host | null {
  const r = app.refs?.remotes.find((x) => x.name === (remote ?? remoteOf(null)));
  return r ? hostOf(r.url) : null;
}

function openLink(url: string) {
  api.openExternal(url).catch((e) => toasts.push("error", String(e)));
}

/** The branch a pull request from `branch` should target on `remote`. */
function prBase(branch: string, remote: string): string | null {
  const parent = stacks.find((s) => s.branch === branch)?.parent;
  const onRemote = new Set((app.refs?.remotes.find((r) => r.name === remote)?.branches ?? []).map((b) => b.name));
  if (parent) {
    const p = parent.startsWith(`${remote}/`) ? parent.slice(remote.length + 1) : parent;
    if (onRemote.has(p)) return p;
  }
  return ["main", "master", "trunk", "develop"].find((b) => b !== branch && onRemote.has(b)) ?? null;
}

/** Open the page that starts a pull request for a local or remote branch, pushing it first if it isn't on the remote. */
export async function createPullRequest(branch: string) {
  const remote = remoteOf(branch);
  const host = hostFor(remote);
  if (!remote || !host) return toasts.push("info", "This repository isn't on GitHub, GitLab, Bitbucket, Codeberg, Gitea or Azure DevOps.");
  let name = branch.startsWith(`${remote}/`) ? branch.slice(remote.length + 1) : branch;
  const local = app.refs?.local.find((b) => b.name === branch);
  if (local) {
    const up = local.upstream;
    if (up) name = up.slice(up.indexOf("/") + 1);
    // Not on the remote yet, or has commits that aren't: push first.
    if (!up || local.ahead > 0) {
      const ok = await dialog.confirm(
        `Push ${branch} first?`,
        up ? `${local.ahead} commit${local.ahead === 1 ? " isn't" : "s aren't"} on ${remote} yet. The ${host.pr} would leave ${local.ahead === 1 ? "it" : "them"} out.` : `${branch} isn't on ${remote} yet, so it has to be pushed before you can open a ${host.pr}.`,
        "Push",
      );
      if (!ok) return;
      if (!(await push(branch, remote, true))) return;
    }
  }
  openLink(newPullRequestUrl(host, name, prBase(name, remote)));
}

/** "Open on GitHub"-style items for a branch's menu (local or remote). */
function branchHostItems(branch: string): MenuItem[] {
  const remote = remoteOf(branch);
  const host = hostFor(remote);
  if (!host || !remote) return [];
  const local = app.refs?.local.find((b) => b.name === branch);
  const name = local ? local.upstream?.slice(local.upstream.indexOf("/") + 1) : branch.slice(remote.length + 1);
  return [
    { separator: true },
    { label: `Create ${host.pr}…`, hint: host.name, action: () => createPullRequest(branch) },
    { label: `Open on ${host.name}`, disabled: !name, hint: name ? undefined : "not pushed", action: () => openLink(branchUrl(host, name!)) },
  ];
}

/** Items for the repository menu. */
export function repoHostItems(): MenuItem[] {
  const host = hostFor(null);
  if (!host) return [];
  return [
    { label: `Open on ${host.name}`, action: () => openLink(host.web) },
    { label: host.kind === "gitlab" ? "Merge requests" : "Pull requests", hint: host.name, action: () => openLink(pullRequestsUrl(host)) },
  ];
}

/** Open a commit, or a file at a commit, on the hosting site. Null when there's nowhere to open it. */
export function hostLink(oid: string, path?: string, line?: number): { label: string; open: () => void; pushed: boolean } | null {
  const host = hostFor(null);
  if (!host) return null;
  return {
    label: `Open on ${host.name}`,
    pushed: onAnyRemote(oid) ?? true,
    open: () => openLink(path ? fileUrl(host, oid, path, line) : commitUrl(host, oid)),
  };
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

/** Every loaded commit reachable from `starts` (themselves included). */
function reachableFrom(starts: string[]): Set<string> {
  const set = new Set<string>();
  const byId = new Map(app.log.map((r) => [r.oid, r]));
  const stack = [...starts];
  while (stack.length) {
    const id = stack.pop()!;
    if (set.has(id)) continue;
    set.add(id);
    const r = byId.get(id);
    if (r) stack.push(...r.parents);
  }
  return set;
}

/** Commits already pushed to the upstream (rewriting them needs a force push). */
export function pushedSet(): Set<string> {
  const up = app.status?.upstream;
  if (!up) return new Set();
  const start = app.log.find((r) => r.refs.some((l) => l.kind === "remote" && l.name === up));
  return start ? reachableFrom([start.oid]) : new Set();
}

/** Whether a commit is on some remote branch, or null when the loaded history can't tell. */
function onAnyRemote(oid: string): boolean | null {
  const starts = app.log.filter((r) => r.refs.some((l) => l.kind === "remote")).map((r) => r.oid);
  return starts.length ? reachableFrom(starts).has(oid) : null;
}

/** Whether a commit is already part of the checked-out branch (null: can't tell). */
function onHead(oid: string): boolean | null {
  const head = app.status?.head;
  return head && app.log.some((r) => r.oid === head) ? reachableFrom([head]).has(oid) : null;
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
  const r = await app.act(label, () => api.interactiveRebase(P(), base, steps, label), { success: (r) => (r.result === "conflicts" ? null : done ?? `${label}: done.`) });
  afterMerge(r, "rebase");
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

/** Fold the staged changes into an earlier commit of this branch (Tower's fixup). */
export async function foldInto(c: CommitRow) {
  const staged = app.status?.staged.length ?? 0;
  if (!staged) return toasts.push("info", "Stage the changes you want to fold into this commit first.");
  const list = rewritable();
  const i = list.findIndex((x) => x.oid === c.oid);
  const after = i >= 0 ? list.length - 1 - i : 0;
  const ok = await dialog.confirm(
    `Fold staged changes into “${c.summary}”?`,
    `The ${staged === 1 ? "staged file's changes become" : `changes in ${staged} staged files become`} part of that commit` +
      (after ? `, and the ${after === 1 ? "commit" : `${after} commits`} after it ${after === 1 ? "is" : "are"} replayed on top.` : ".") +
      " Unstaged changes and new files stay as they are. Undo puts everything back.",
    "Fold in",
  );
  if (!ok) return;
  if (!(await confirmRewritePushed(i >= 0 ? list.slice(i).map((x) => x.oid) : [c.oid]))) return;
  await app.act(`Folding changes into “${c.summary}”`, () => api.foldInto(P(), c.oid), { success: `Folded into “${c.summary}”.` });
}

/** Save every current change as a stash, leaving the working copy as it is. */
export async function snapshot() {
  const s = app.status;
  if (s && !s.staged.length && !s.unstaged.length) return toasts.push("info", "There are no changes to snapshot.");
  const r = await dialog.show({
    title: "Save a snapshot",
    body: "Keeps a copy of all your current changes, including new files, in the stash list. Your working copy stays exactly as it is, so you can carry on and come back to this point later.",
    fields: [{ key: "name", label: "Name (optional)", placeholder: "Before trying the other approach" }],
    confirm: "Save snapshot",
  });
  if (r.button !== "confirm") return;
  await app.act("Saving snapshot", () => api.stashSnapshot(P(), String(r.values.name ?? "").trim() || null), {
    success: (label) => `${label} saved. Find it under Stashes.`,
  });
}

/** Make the working copy what a stash (usually a snapshot) holds, keeping today's state as a new snapshot. */
export async function goBackToStash(oid: string, message: string) {
  const s = app.status;
  const dirty = !!s && (s.staged.length > 0 || s.unstaged.length > 0);
  const ok = await dialog.confirm(
    "Go back to this point?",
    `Your working copy becomes what “${message}” holds, with its staged changes staged again.` +
      (dirty ? " Your current changes are saved as a new snapshot first, so you can come back to them." : ""),
    "Go back",
  );
  if (!ok) return;
  await app.act("Going back", () => api.stashRestoreSnapshot(P(), oid), {
    success: (saved) => (saved ? `Restored. Your previous changes are in “${saved}”.` : "Restored."),
  });
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
  const link = hostLink(c.oid);
  menu.fromEvent(e, [
    { label: "Check out this commit", action: () => app.act("Checking out", () => api.checkoutCommit(P(), c.oid), { success: `Checked out ${c.short} (detached).` }) },
    { label: "New branch here…", action: () => newBranch(c.oid, c.short) },
    { label: "New tag here…", action: () => newTag(c.oid, c.short) },
    { separator: true },
    { label: `Cherry-pick into ${head ?? "HEAD"}`, disabled: !!onHead(c.oid), hint: onHead(c.oid) ? "already on it" : undefined, action: () => cherryPick(c.oid) },
    { label: "Revert commit", action: () => revertCommit(c.oid) },
    { label: "Find the bad commit after this…", hint: "bisect", disabled: !!app.status?.bisect || c.oid === app.status?.head, action: () => bisectStart(c) },
    { label: `Reset ${head ?? "HEAD"} to here…`, action: () => resetTo(c) },
    { separator: true },
    { label: "Edit message…", disabled: !canRewrite, action: () => reword(c) },
    { label: "Squash into parent", disabled: !canRewrite || isFirst, action: () => combineWithParent(c, "squash") },
    { label: "Fixup into parent", hint: "drops this message", disabled: !canRewrite || isFirst, action: () => combineWithParent(c, "fixup") },
    { label: "Fold staged changes into this commit…", hint: app.status?.staged.length ? undefined : "nothing staged", disabled: !canRewrite || !app.status?.staged.length, action: () => foldInto(c) },
    { label: "Remove commit…", danger: true, disabled: !canRewrite, action: () => dropCommit(c) },
    { label: "Rewrite from here…", disabled: !canRewrite, action: () => openRebase(c) },
    { separator: true },
    { label: "Browse files at this commit", action: () => browseAt(c.oid, c.short) },
    { label: "Export as ZIP…", action: () => exportZip(c.oid, c.short) },
    { label: "Save as patch…", disabled: c.parents.length > 1, action: () => saveCommitPatch(c) },
    { label: "Copy commit ID", action: () => navigator.clipboard.writeText(c.oid).then(() => toasts.push("info", "Copied commit ID.")) },
    ...(link ? [{ label: link.label, hint: link.pushed ? undefined : "not pushed yet", disabled: !link.pushed, action: link.open }] : []),
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

/** Stash all changes, or only those in `paths`. */
export async function stashChanges(paths?: string[]) {
  const s = app.status;
  if (s && !s.staged.length && !s.unstaged.length) {
    toasts.push("info", "There are no local changes to stash.");
    return;
  }
  const partial = !!paths?.length;
  const untrackedChosen = partial && paths!.some((p) => s?.unstaged.some((f) => f.path === p && f.status === "untracked"));
  const r = await dialog.show({
    title: partial ? `Stash ${paths!.length === 1 ? paths![0].split("/").pop() : `${paths!.length} files`}` : "Stash changes",
    body: partial ? "Saves the changes to just these files and undoes them in the working copy. Everything else stays as it is." : "Saves your changes and cleans the working copy.",
    items: partial && paths!.length > 1 ? paths!.map((p) => ({ text: p })) : undefined,
    fields: partial ? [
      { key: "message", label: "Description (optional)", placeholder: "What were you working on?" },
      { key: "untracked", label: "Include new files", type: "checkbox", checked: true },
    ] : [
      { key: "message", label: "Description (optional)", placeholder: "What were you working on?" },
      { key: "untracked", label: "Include new files", type: "checkbox", checked: true },
      { key: "keep", label: "Keep staged changes in the working copy", type: "checkbox", checked: false },
    ],
    confirm: "Stash",
  });
  if (r.button !== "confirm") return;
  const untracked = partial ? untrackedChosen && !!r.values.untracked : !!r.values.untracked;
  await app.act("Stashing", () => api.stashSave(P(), String(r.values.message ?? "").trim() || null, untracked, !!r.values.keep, false, paths), {
    success: partial ? "Those changes are stashed." : "Changes stashed.",
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
        action: () => cherryPick(p.oid) },
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

// ---- Git-flow ----

export async function flowSetup() {
  const c = (await api.flowConfig(P())) ?? (await api.flowSuggest(P()));
  const r = await dialog.show({
    title: "Set up git-flow",
    body: "Features and releases branch off the development branch; hotfixes off the production branch. These are git-flow's own settings, so the git-flow command line agrees.",
    fields: [
      { key: "main", label: "Production branch", value: c.main },
      { key: "develop", label: "Development branch", value: c.develop, hint: "Created from the production branch if it doesn't exist." },
      { key: "feature", label: "Feature prefix", value: c.feature },
      { key: "release", label: "Release prefix", value: c.release },
      { key: "hotfix", label: "Hotfix prefix", value: c.hotfix },
      { key: "versionTag", label: "Version tag prefix", value: c.versionTag, placeholder: "e.g. v (optional)" },
    ],
    confirm: "Set up",
    validate: (v) => (!String(v.main).trim() || !String(v.develop).trim() ? "Name both branches." : null),
  });
  if (r.button !== "confirm") return false;
  const v = r.values;
  const config = { main: String(v.main).trim(), develop: String(v.develop).trim(), feature: String(v.feature), release: String(v.release), hotfix: String(v.hotfix), versionTag: String(v.versionTag ?? "") };
  await app.act("Setting up git-flow", () => api.flowInit(P(), config), { success: "Git-flow is set up." });
  return true;
}

const FLOW_NOUN: Record<string, string> = { feature: "feature", release: "release", hotfix: "hotfix" };

export async function flowStart(kind: "feature" | "release" | "hotfix") {
  const c = await api.flowConfig(P());
  if (!c) return flowSetup();
  const prefix = kind === "feature" ? c.feature : kind === "release" ? c.release : c.hotfix;
  const name = await dialog.prompt(`Start ${FLOW_NOUN[kind]}`, kind === "feature" ? "Feature name" : "Version", {
    confirm: `Start ${FLOW_NOUN[kind]}`,
    placeholder: kind === "feature" ? "login-form" : "1.4.0",
    body: `Creates ${prefix}… from ${kind === "hotfix" ? c.main : c.develop} and checks it out.`,
    validate: (v) => (!v.trim() ? "Give it a name." : validBranch(prefix + v.trim())),
  });
  if (!name) return;
  await app.act(`Starting ${FLOW_NOUN[kind]}`, () => api.flowStart(P(), kind, name), { success: (b) => `Started ${b}.` });
}

export async function flowFinish(branch: string) {
  const c = await api.flowConfig(P());
  if (!c) return;
  const kind = branch.startsWith(c.feature) ? "feature" : branch.startsWith(c.release) ? "release" : "hotfix";
  const name = branch.slice((kind === "feature" ? c.feature : kind === "release" ? c.release : c.hotfix).length);
  const tagged = kind !== "feature";
  const r = await dialog.show({
    title: `Finish ${branch}`,
    body: tagged
      ? `Merges ${branch} into ${c.main}, tags it ${c.versionTag}${name}, and merges ${c.main} back into ${c.develop}.`
      : `Merges ${branch} into ${c.develop} (with a merge commit).`,
    fields: [
      ...(tagged ? [{ key: "tag", label: "Tag message", value: `${kind === "release" ? "Release" : "Hotfix"} ${name}` }] : []),
      { key: "keep", label: "Keep the branch afterwards", type: "checkbox" as const, checked: false },
    ],
    confirm: "Finish",
  });
  if (r.button !== "confirm") return;
  const out = await app.act(`Finishing ${branch}`, () => api.flowFinish(P(), branch, tagged ? String(r.values.tag ?? "") : null, !!r.values.keep), {
    success: (o) => (o.result === "finished" ? `Finished ${branch}${o.tag ? `, tagged ${o.tag}` : ""}.` : null),
  });
  if (out?.result === "conflicts") {
    app.view = "changes";
    toasts.push("error", `Merging into ${out.mergingInto} stopped on ${out.conflicts.length} conflicted file${out.conflicts.length === 1 ? "" : "s"}. Resolve them and commit, then finish ${branch} again.`);
  }
}

export async function flowMenu(e: MouseEvent) {
  const c = await api.flowConfig(P());
  if (!c) {
    menu.fromEvent(e, [{ label: "Set up git-flow…", action: () => flowSetup() }], "Git-flow");
    return;
  }
  const open = (app.refs?.local ?? []).map((b) => b.name).filter((n) => n.startsWith(c.feature) || n.startsWith(c.release) || n.startsWith(c.hotfix));
  const head = app.headBranch;
  menu.fromEvent(e, [
    { label: "Start feature…", action: () => flowStart("feature") },
    { label: "Start release…", action: () => flowStart("release") },
    { label: "Start hotfix…", action: () => flowStart("hotfix") },
    ...(open.length ? [{ separator: true } as MenuItem] : []),
    ...open.map((b) => ({ label: `Finish ${b}…`, hint: b === head ? "current" : undefined, action: () => flowFinish(b) })),
    { separator: true },
    { label: "Git-flow settings…", action: () => flowSetup() },
  ], "Git-flow");
}

// ---- Stacked branches ----

export async function stackOn(branch: string) {
  const choices = (app.refs?.local ?? []).map((b) => b.name).filter((n) => n !== branch);
  const r = await dialog.show({
    title: `Stack ${branch} on…`,
    body: `When that branch changes, "Restack" replays ${branch}'s own commits on top of it.`,
    fields: [{ key: "parent", label: "Parent branch", placeholder: choices.slice(0, 3).join(", "), value: app.status?.branch !== branch ? app.status?.branch ?? "" : "" }],
    confirm: "Stack",
    validate: (v) => (choices.includes(String(v.parent).trim()) ? null : "Pick one of your local branches."),
  });
  if (r.button !== "confirm") return;
  const parent = String(r.values.parent).trim();
  await app.act("Stacking", () => api.stackSetParent(P(), branch, parent), { success: `${branch} is stacked on ${parent}.` });
}

export async function unstack(branch: string) {
  await app.act("Unstacking", () => api.stackSetParent(P(), branch, null), { success: `${branch} isn't stacked any more.` });
}

/** Restack a branch, then the branches stacked on it, stopping at a conflict. */
export async function restack(branch: string, withChildren = false) {
  const chain = withChildren ? [branch, ...(await api.stackDescendants(P(), branch))] : [branch];
  for (const b of chain) {
    const r = await app.act(`Restacking ${b}`, () => api.stackRestack(P(), b));
    if (!r) return;
    if (r.result === "conflicts") {
      app.view = "changes";
      const rest = chain.slice(chain.indexOf(b) + 1);
      toasts.push("error", `Restacking ${b} stopped on a conflict. Resolve it and Continue${rest.length ? `, then restack ${rest.join(", ")}` : ""}.`);
      return;
    }
  }
  toasts.push("success", chain.length > 1 ? `Restacked ${chain.length} branches.` : `Restacked ${branch}.`);
}

// ---- Bisect ----

export async function bisectStart(good: { oid: string; short: string; summary: string }) {
  const head = app.status?.head?.slice(0, 7);
  const ok = await dialog.show({
    title: "Find the commit that broke it",
    body: `Bisect checks out commits between ${good.short} (works) and ${head} (broken, what's checked out now). Test each one and mark it good or bad; it narrows down to the first bad commit in a few steps.`,
    items: [{ text: `Good: ${good.short} ${good.summary}` }, { text: `Bad: ${head} ${app.status?.headSummary ?? ""}` }],
    confirm: "Start bisect",
  });
  if (ok.button !== "confirm") return;
  await app.act("Starting bisect", () => api.bisectStart(P(), null, good.oid), { success: (s) => `Testing ${s.current?.slice(0, 7)} · about ${s.steps} step${s.steps === 1 ? "" : "s"} left.` });
  app.view = "history";
}

export async function bisectMark(verdict: "good" | "bad" | "skip") {
  await app.act(`Marking ${verdict}`, () => api.bisectMark(P(), verdict), {
    success: (s) => (s.found ? `Found it: ${s.found.slice(0, 7)} is the first bad commit.` : `Now testing ${s.current?.slice(0, 7)}.`),
  });
}

export async function bisectStop() {
  await app.act("Stopping bisect", () => api.bisectReset(P()), { success: "Bisect stopped; back where you started." });
}

// ---- External tools ----

export function hasDiffTool() {
  return !!app.settings.diffTool;
}

/** Open a file's change in the external diff tool. area: unstaged | staged | a commit id. */
export async function openDiffTool(path: string, area: string) {
  const s = app.settings;
  if (!s.diffTool) {
    toasts.push("info", "Pick a diff tool in Preferences → Diffs & Git first.");
    app.prefs = "diffs";
    return;
  }
  try {
    await api.diffTool(P(), path, area, s.diffTool, s.diffCommand);
  } catch (e) {
    toasts.push("error", String(e));
  }
}

/** Resolve a conflicted file in the external merge tool; offer to mark it resolved after. */
export async function openMergeTool(path: string) {
  const s = app.settings;
  if (!s.mergeTool) {
    toasts.push("info", "Pick a merge tool in Preferences → Diffs & Git first.");
    app.prefs = "diffs";
    return;
  }
  const r = await app.act(`Waiting for the merge tool (${path.split("/").pop()})`, () => api.mergeTool(P(), path, s.mergeTool, s.mergeCommand));
  if (!r) return;
  if (r.markersLeft) {
    toasts.push("info", `${path.split("/").pop()} still has conflict markers.`);
    return;
  }
  if (await dialog.confirm("Mark as resolved?", `The merge tool closed and ${path} has no conflict markers left.`, "Mark resolved")) {
    await app.act("Resolving", () => api.stageFiles(P(), [path]), { success: `Resolved ${path.split("/").pop()}.` });
  }
}

// ---- Pinned branches ----

export function pinnedBranches(): string[] {
  return app.settings.pinned[app.path] ?? [];
}

export function togglePin(name: string) {
  const list = pinnedBranches();
  app.settings.pinned[app.path] = list.includes(name) ? list.filter((n) => n !== name) : [...list, name];
  app.saveSettings();
}

// ---- Git LFS ----

export async function lfsTrack(pattern?: string) {
  const p = pattern ?? (await dialog.prompt("Track files with Git LFS", "File pattern", {
    confirm: "Track",
    placeholder: "*.psd",
    body: "Matching files are stored with LFS from now on (in .gitattributes, which you then commit). Files already committed stay as they are.",
    validate: (v) => (!v.trim() ? "Enter a pattern." : null),
  }));
  if (!p) return;
  await app.act("Tracking with LFS", () => api.lfsTrack(P(), p), { success: `New ${p} files are stored with LFS. Commit .gitattributes to share that.` });
  await app.loadLfs();
}

export async function lfsUntrack(pattern: string) {
  await app.act("Untracking", () => api.lfsTrack(P(), pattern, true), { success: `${pattern} isn't stored with LFS any more.` });
  await app.loadLfs();
}

export async function lfsPull() {
  await app.act("Downloading LFS files", () => api.lfsPull(P()), { success: "LFS files are up to date." });
  await app.loadLfs();
}

export async function lfsLock(path: string, unlock = false) {
  await app.act(unlock ? "Unlocking" : "Locking", () => api.lfsLock(P(), path, unlock), { success: unlock ? `Unlocked ${path}.` : `Locked ${path}. Others can't push changes to it until you unlock it.` });
}

// ---- Undo history ----

/** The undo steps, newest first; pick one to undo back to (and including) it. */
export async function undoMenu(e: MouseEvent) {
  e.preventDefault();
  const list = await api.undoList(P()).catch(() => []); // newest first
  if (!list.length) {
    toasts.push("info", "Nothing to undo.");
    return;
  }
  menu.fromEvent(e, list.slice(0, 20).map((u, i) => ({
    label: u.label,
    hint: relTime(u.time),
    action: async () => {
      if (i > 0 && !(await dialog.confirm(`Undo ${i + 1} actions?`, `Undoes everything from “${list[0].label}” back to “${u.label}”, newest first.`, `Undo ${i + 1}`))) return;
      for (let k = 0; k <= i; k++) {
        if (!app.undoTop) break;
        await app.undo();
      }
    },
  })), "Undo history");
}
