<script lang="ts">
  import { keyLabel } from "../lib/platform";
  import { api } from "../lib/api";
  import { app } from "../lib/state.svelte";
  import { toasts } from "../lib/ui.svelte";

  let subject = $state("");
  let body = $state("");
  let amend = $state(false);
  let skipHooks = $state(false);
  let saved: { subject: string; body: string } | null = null;
  let loadedFor = "";

  const st = $derived(app.status);
  const draftKey = $derived(`arbor.draft.${app.path}`);

  // Restore the draft for this repository.
  $effect(() => {
    if (loadedFor === app.path) return;
    loadedFor = app.path;
    amend = false;
    try {
      const d = JSON.parse(localStorage.getItem(draftKey) || "null");
      subject = d?.subject ?? "";
      body = d?.body ?? "";
    } catch {
      subject = body = "";
    }
  });

  $effect(() => {
    const d = { subject, body };
    if (!amend) {
      try {
        localStorage.setItem(draftKey, JSON.stringify(d));
      } catch {}
    }
  });

  // Prefill git's prepared message when a merge stopped on conflicts.
  $effect(() => {
    if (st?.state === "merge" && !subject && st.mergeMessage) {
      const [s, ...rest] = st.mergeMessage.split("\n");
      subject = s;
      body = rest.join("\n").trim();
    }
  });

  async function toggleAmend() {
    amend = !amend;
    if (amend) {
      saved = { subject, body };
      const m = (await api.headMessage(app.path)) ?? "";
      const [s, ...rest] = m.split("\n");
      subject = s;
      body = rest.join("\n").trim();
    } else if (saved) {
      subject = saved.subject;
      body = saved.body;
      saved = null;
    }
  }

  const staged = $derived(st?.staged.length ?? 0);
  const merging = $derived(st?.state === "merge");
  const canCommit = $derived(
    !!subject.trim() && !app.busy && !st?.conflicted.length && (staged > 0 || amend || merging),
  );
  const label = $derived(
    amend ? "Amend last commit" : merging ? "Commit merge" : staged ? `Commit ${staged} file${staged === 1 ? "" : "s"}` : "Commit",
  );

  async function commit() {
    if (!canCommit) return;
    const message = body.trim() ? `${subject.trim()}\n\n${body.trim()}` : subject.trim();
    const r = await app.act(amend ? "Amending" : "Committing", () => api.commit(app.path, message, amend, skipHooks));
    if (r) {
      toasts.push("success", `${amend ? "Amended" : "Committed"} ${r.short}.`, { label: "Undo", run: () => app.undo() });
      subject = "";
      body = "";
      amend = false;
      saved = null;
      try {
        localStorage.removeItem(draftKey);
      } catch {}
    }
  }

  function key(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      commit();
    }
  }

  const over = $derived(subject.length > 72);

  // Commit and focus requests from keyboard shortcuts.
  let summaryEl = $state<HTMLInputElement>();
  const fresh = (t: number) => Date.now() - t < 1500;
  let seenCommit = 0;
  let seenFocus = 0;
  $effect(() => {
    const t = app.commitRequest;
    if (t > seenCommit && fresh(t) && summaryEl) {
      seenCommit = t;
      if (canCommit) commit();
      else summaryEl.focus();
    }
  });
  $effect(() => {
    const t = app.focusComposer;
    if (t > seenFocus && fresh(t) && summaryEl) {
      seenFocus = t;
      summaryEl.focus();
    }
  });
</script>

<div class="composer">
  <div class="subject">
    <input bind:this={summaryEl} placeholder={amend ? "Message of the amended commit" : "Summary"} bind:value={subject} onkeydown={key} spellcheck="true" aria-label="Commit summary" />
    <span class="len" class:over title="Keep the summary under 72 characters">{subject.length || ""}</span>
  </div>
  <textarea rows="3" placeholder="Description" bind:value={body} onkeydown={key} spellcheck="true" aria-label="Commit description"></textarea>
  <div class="row">
    <label title="Replace the last commit with these staged changes and message"><input type="checkbox" checked={amend} onchange={toggleAmend} disabled={!st?.head || merging} /> Amend</label>
    <label title="Don't run the pre-commit and commit-msg hooks"><input type="checkbox" bind:checked={skipHooks} /> Skip hooks</label>
    <span class="sp"></span>
    <button class="btn primary" disabled={!canCommit} onclick={commit} title={keyLabel("Ctrl+Enter")}>{label}</button>
  </div>
</div>

<style>
  .composer { border-top: 1px solid var(--line); padding: 10px; display: flex; flex-direction: column; gap: 6px; background: var(--surface-2); }
  .subject { position: relative; }
  .subject input { font-weight: 600; padding-right: 36px; }
  .len { position: absolute; right: 9px; top: 50%; translate: 0 -50%; font-size: calc(var(--fs) * 11); color: var(--ink-3); font-variant-numeric: tabular-nums; }
  .len.over { color: var(--danger); font-weight: 700; }
  .row { display: flex; align-items: center; gap: 12px; font-size: calc(var(--fs) * 12); color: var(--ink-2); }
  label { display: flex; align-items: center; gap: 5px; }
  .sp { flex: 1; }
</style>
