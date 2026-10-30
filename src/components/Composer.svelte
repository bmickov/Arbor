<script lang="ts">
  import { keyLabel } from "../lib/platform";
  import { api, type Identity } from "../lib/api";
  import { app } from "../lib/state.svelte";
  import { dialog, menu, toasts, type MenuItem } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";

  let subject = $state("");
  let body = $state("");
  let amend = $state(false);
  let skipHooks = $state(false);
  // Sign this commit; starts from the repository's commit.gpgsign.
  let sign = $state(false);
  $effect(() => {
    void app.path;
    app.loadSigning();
  });
  $effect(() => {
    sign = app.signing?.enabled ?? false;
  });
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

  // A message handed over by "Apply patch" (from an email patch).
  let seenPrefill = 0;
  $effect(() => {
    const p = app.prefillMessage;
    if (!p || p.at <= seenPrefill) return;
    seenPrefill = p.at;
    if (subject.trim() || body.trim()) {
      dialog.confirm("Use the patch's message?", `Replace your message with “${p.subject}”?`, "Replace").then((ok) => ok && ((subject = p.subject), (body = p.body)));
    } else {
      subject = p.subject;
      body = p.body;
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
    const r = await app.act(amend ? "Amending" : "Committing", () => api.commit(app.path, message, amend, skipHooks, sign));
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

  // ------------------------------------------------ who commits (profiles)
  let who = $state<Identity | null>(null);
  $effect(() => {
    const path = app.path;
    void app.version;
    api.getIdentity(path).then((i) => app.path === path && (who = i)).catch(() => (who = null));
  });

  async function setWho(name: string | null, email: string | null) {
    await app.act("Changing author", () => api.setIdentity(app.path, name, email), {
      success: name ? `You now commit as ${name} in this repository.` : "This repository now uses your global name and email.",
    });
    who = await api.getIdentity(app.path).catch(() => null);
  }

  async function customWho() {
    const r = await dialog.show({
      title: "Commit as…",
      body: "Used for commits in this repository only.",
      fields: [
        { key: "name", label: "Name", value: who?.name ?? "" },
        { key: "email", label: "Email", value: who?.email ?? "" },
        { key: "save", label: "Also save as a profile", type: "checkbox", checked: true },
      ],
      confirm: "Use",
      validate: (v) => (!String(v.name).trim() ? "Enter a name." : !/^\S+@\S+$/.test(String(v.email).trim()) ? "Enter an email address." : null),
    });
    if (r.button !== "confirm") return;
    const name = String(r.values.name).trim(), email = String(r.values.email).trim();
    if (r.values.save && !app.settings.profiles.some((p) => p.email === email && p.name === name)) {
      app.settings.profiles.push({ name, email });
      app.saveSettings();
    }
    await setWho(name, email);
  }

  function whoMenu(e: MouseEvent) {
    const profiles = app.settings.profiles.filter((p) => p.name.trim() && p.email.trim());
    const items: MenuItem[] = [
      ...profiles.map((p) => ({
        label: p.name,
        hint: p.email + (who?.email === p.email && who?.name === p.name ? "  ✓" : ""),
        action: () => setWho(p.name, p.email),
      })),
      ...(profiles.length ? [{ separator: true } as MenuItem] : []),
      { label: "Someone else…", action: customWho },
      { label: "Use my global name and email", disabled: !who?.local, action: () => setWho(null, null) },
      { separator: true },
      { label: "Edit profiles…", action: () => (app.prefs = "commits") },
    ];
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu.open(r.left, r.bottom + 4, items, "Commit as");
  }

  // ----------------------------------------------------------- templates
  async function useTemplate(text: string) {
    if ((subject.trim() || body.trim()) && !(await dialog.confirm("Replace the message?", "The template replaces what you've written so far.", "Replace"))) return;
    // Like git, lines starting with # are instructions, not part of the message.
    const lines = text.replace(/\r/g, "").split("\n").filter((l) => !l.startsWith("#"));
    subject = (lines.shift() ?? "").trimEnd();
    body = lines.join("\n").replace(/^\n+/, "").trimEnd();
    if (body) body += "\n";
    summaryEl?.focus();
  }

  async function templateMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const own = await api.commitTemplate(app.path).catch(() => null);
    const saved = app.settings.commitTemplates.filter((t) => t.text.trim());
    const items: MenuItem[] = [
      ...saved.map((t) => ({ label: t.name.trim() || t.text.split("\n")[0], action: () => useTemplate(t.text) })),
      ...(own ? [{ label: "This repository's template", hint: "commit.template", action: () => useTemplate(own) }] : []),
      ...(saved.length || own ? [{ separator: true } as MenuItem] : [{ label: "No templates yet", disabled: true } as MenuItem]),
      { label: saved.length ? "Edit templates…" : "Add a template…", action: () => (app.prefs = "commits") },
    ];
    menu.open(r.left, r.bottom + 4, items, "Commit templates");
  }

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
  <div class="meta">
    <button class="who" class:missing={who && !who.email} onclick={whoMenu} title="Who this commit is by. Click to switch, per repository.">
      <Icon name="commit" size={12} />
      {#if who?.email}
        <span class="wname">{who.name ?? "(no name)"}</span><span class="wmail">{who.email}</span>
      {:else}
        <span class="wname">Choose who you commit as</span>
      {/if}
      <Icon name="down" size={11} />
    </button>
    <span class="sp"></span>
    <button class="tplbtn" onclick={templateMenu}>Template <Icon name="down" size={11} /></button>
  </div>
  <div class="subject">
    <input bind:this={summaryEl} placeholder={amend ? "Message of the amended commit" : "Summary"} bind:value={subject} onkeydown={key} spellcheck="true" aria-label="Commit summary" />
    <span class="len" class:over title="Keep the summary under 72 characters">{subject.length || ""}</span>
  </div>
  <textarea rows="3" placeholder="Description" bind:value={body} onkeydown={key} spellcheck="true" aria-label="Commit description"></textarea>
  <div class="row">
    <label title="Replace the last commit with these staged changes and message"><input type="checkbox" checked={amend} onchange={toggleAmend} disabled={!st?.head || merging} /> Amend</label>
    <label title="Don't run the pre-commit and commit-msg hooks"><input type="checkbox" bind:checked={skipHooks} /> Skip hooks</label>
    {#if app.signing?.enabled || app.signing?.key}
      <label title={sign ? `Sign with ${app.signing.format === "ssh" ? "SSH" : app.signing.format === "x509" ? "X.509" : "GPG"}${app.signing.key ? ` key ${app.signing.key}` : ""}. Set up in Preferences → Commits.` : "Commit without a signature"}>
        <input type="checkbox" bind:checked={sign} /> Sign</label>
    {/if}
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
  .row { display: flex; align-items: center; gap: 6px 10px; flex-wrap: wrap; font-size: calc(var(--fs) * 12); color: var(--ink-2); }
  label { display: flex; align-items: center; gap: 4px; white-space: nowrap; }
  .row .btn { margin-left: auto; }
  .sp { flex: 1; }
  .meta { display: flex; align-items: center; gap: 8px; font-size: calc(var(--fs) * 11.5); color: var(--ink-3); margin: -2px 0 0; }
  .who, .tplbtn { display: inline-flex; align-items: center; gap: 5px; min-width: 0; padding: 2px 6px; margin-left: -6px; border-radius: 5px; color: var(--ink-3); }
  .tplbtn { margin: 0 -6px 0 0; flex: none; }
  .who:hover, .tplbtn:hover { background: var(--surface-3); color: var(--ink-2); }
  .who.missing { color: var(--warn); }
  .wname { font-weight: 600; color: var(--ink-2); white-space: nowrap; }
  .wmail { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
