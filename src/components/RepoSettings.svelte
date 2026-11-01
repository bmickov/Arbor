<script lang="ts">
  // Repository settings: remotes (rename, URLs), the .gitignore file, and
  // the git config that applies here (this repository's own settings are
  // editable; global and system ones are shown for reference).
  import { api, type ConfigEntry, type RemoteInfo } from "../lib/api";
  import * as act from "../lib/actions";
  import { app } from "../lib/state.svelte";
  import { dialog, toasts } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";

  let { onclose }: { onclose: () => void } = $props();
  type Tab = "remotes" | "ignore" | "config";
  let tab = $state<Tab>("remotes");

  // ---- Remotes
  interface Row extends RemoteInfo { newName: string; newUrl: string; newPush: string }
  let remotes = $state<Row[]>([]);
  async function loadRemotes() {
    const rs = await api.remotesInfo(app.path).catch(() => []);
    remotes = rs.map((r) => ({ ...r, newName: r.name, newUrl: r.url, newPush: r.pushUrl ?? "" }));
  }
  const dirty = (r: Row) => r.newName !== r.name || r.newUrl !== r.url || r.newPush !== (r.pushUrl ?? "");
  async function saveRemote(r: Row) {
    try {
      await api.editRemote(app.path, r.name, r.newName, r.newUrl, r.newPush || null);
      toasts.push("success", `Saved ${r.newName}.`);
      await loadRemotes();
      await app.refresh();
    } catch (e) {
      toasts.push("error", String(e));
    }
  }
  async function removeRemote(r: Row) {
    if (!(await dialog.confirm(`Remove ${r.name}?`, `Arbor forgets the remote and its remote branches. Nothing on the server changes, and Undo brings it back.`, "Remove", true))) return;
    await app.act("Removing remote", () => api.removeRemote(app.path, r.name), { success: `Removed ${r.name}.` });
    await loadRemotes();
  }

  // ---- .gitignore
  let ignore = $state("");
  let ignoreSaved = $state("");
  async function loadIgnore() {
    ignore = ignoreSaved = await api.readIgnore(app.path).catch(() => "");
  }
  async function saveIgnore() {
    try {
      await api.writeIgnore(app.path, ignore);
      ignoreSaved = ignore;
      toasts.push("success", "Saved .gitignore. Commit it to share it.");
      await app.refresh();
    } catch (e) {
      toasts.push("error", String(e));
    }
  }
  const SNIPPETS: [string, string][] = [
    ["macOS", ".DS_Store"], ["Windows", "Thumbs.db\ndesktop.ini"], ["Editors", ".vscode/\n.idea/\n*.swp"],
    ["Node", "node_modules/\ndist/\n.env"], ["Rust", "target/"], ["Python", "__pycache__/\n*.pyc\n.venv/"], ["PHP", "vendor/\n.phpunit.result.cache"],
  ];
  function addSnippet(text: string) {
    const have = new Set(ignore.split("\n").map((l) => l.trim()));
    const add = text.split("\n").filter((l) => !have.has(l));
    if (!add.length) return;
    ignore = (ignore && !ignore.endsWith("\n") ? ignore + "\n" : ignore) + add.join("\n") + "\n";
  }

  // ---- Config
  let config = $state.raw<ConfigEntry[]>([]);
  let cfgFilter = $state("");
  let newKey = $state("");
  let newValue = $state("");
  async function loadConfig() {
    config = await api.repoConfig(app.path).catch(() => []);
  }
  // The value that wins for each key (later entries override earlier ones).
  const effective = $derived.by(() => {
    const m = new Map<string, ConfigEntry>();
    for (const e of config) m.set(e.key, e);
    const q = cfgFilter.trim().toLowerCase();
    return [...m.values()].filter((e) => !q || e.key.includes(q) || e.value.toLowerCase().includes(q)).sort((a, b) => a.key.localeCompare(b.key));
  });
  async function setConfig(key: string, value: string | null) {
    try {
      await api.setRepoConfig(app.path, key, value);
      await loadConfig();
      if (key === newKey) newKey = newValue = "";
    } catch (e) {
      toasts.push("error", String(e));
    }
  }
  const secret = (k: string) => /token|password|secret|credential\.|\.key$/i.test(k);

  $effect(() => {
    if (tab === "remotes") loadRemotes();
    else if (tab === "ignore") loadIgnore();
    else loadConfig();
  });

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      onclose();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="backdrop" onkeydown={key} onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="box" role="dialog" aria-modal="true" aria-labelledby="rs-title">
    <nav>
      <h2 id="rs-title">Repository</h2>
      <p class="faint path" title={app.path}>{app.repo?.name}</p>
      {#each [["remotes", "Remotes", "remote"], ["ignore", "Ignored files", "file"], ["config", "Git config", "settings"]] as [id, label, icon]}
        <button class:on={tab === id} onclick={() => (tab = id as Tab)}><Icon name={icon} size={14} /> {label}</button>
      {/each}
    </nav>
    <div class="content">
      <header>
        <h3>{tab === "remotes" ? "Remotes" : tab === "ignore" ? ".gitignore" : "Git config"}</h3>
        <button class="x" onclick={onclose} aria-label="Close"><Icon name="x" size={14} /></button>
      </header>
      <div class="scroll">
        {#if tab === "remotes"}
          <p class="hint">Where this repository fetches from and pushes to. The push URL is only needed when pushes go somewhere else.</p>
          {#each remotes as r (r.name)}
            <div class="remote">
              <div class="rrow"><span class="lbl">Name</span><input bind:value={r.newName} spellcheck="false" aria-label="Remote name" /></div>
              <div class="rrow"><span class="lbl">URL</span><input class="mono" bind:value={r.newUrl} spellcheck="false" aria-label="Remote URL" /></div>
              <div class="rrow"><span class="lbl">Push URL</span><input class="mono" bind:value={r.newPush} placeholder="Same as URL" spellcheck="false" aria-label="Push URL" /></div>
              <div class="ractions">
                <button class="btn small ghost danger" onclick={() => removeRemote(r)}><Icon name="discard" size={13} /> Remove</button>
                <span class="sp"></span>
                <button class="btn small" disabled={!dirty(r)} onclick={() => { r.newName = r.name; r.newUrl = r.url; r.newPush = r.pushUrl ?? ""; }}>Revert</button>
                <button class="btn small primary" disabled={!dirty(r)} onclick={() => saveRemote(r)}>Save</button>
              </div>
            </div>
          {:else}
            <p class="faint">No remotes yet.</p>
          {/each}
          <button class="btn small" onclick={async () => { await act.addRemote(); await loadRemotes(); }}><Icon name="plus" size={13} /> Add remote</button>
        {:else if tab === "ignore"}
          <p class="hint">Files matching these patterns aren't shown as changes. One pattern per line; <code>#</code> starts a comment. To ignore a single file, right-click it in Changes instead.</p>
          <textarea class="mono ign" bind:value={ignore} spellcheck="false" aria-label=".gitignore" placeholder={"node_modules/\n*.log"}></textarea>
          <div class="snips">
            <span class="faint">Add common patterns:</span>
            {#each SNIPPETS as [name, text]}<button class="btn small" onclick={() => addSnippet(text)}>{name}</button>{/each}
          </div>
          <div class="ractions">
            <span class="sp"></span>
            <button class="btn small" disabled={ignore === ignoreSaved} onclick={() => (ignore = ignoreSaved)}>Revert</button>
            <button class="btn small primary" disabled={ignore === ignoreSaved} onclick={saveIgnore}>Save</button>
          </div>
        {:else}
          <p class="hint">Settings that apply here. Values set for this repository can be changed or removed; global and system ones come from your other git config files.</p>
          <div class="cfgtools">
            <input placeholder="Filter" bind:value={cfgFilter} spellcheck="false" aria-label="Filter settings" />
          </div>
          <div class="cfg">
            {#each effective as e (e.key)}
              <div class="crow">
                <span class="k mono" title={e.key}>{e.key}</span>
                {#if e.level === "local"}
                  <input class="mono v" value={secret(e.key) ? "••••••" : e.value} disabled={secret(e.key)} aria-label={e.key}
                    onchange={(ev) => setConfig(e.key, (ev.currentTarget as HTMLInputElement).value)} />
                  <button class="btn small ghost" title="Remove from this repository" aria-label="Remove {e.key}" onclick={() => setConfig(e.key, null)}><Icon name="x" size={12} /></button>
                {:else}
                  <span class="v mono faint" title={secret(e.key) ? "" : e.value}>{secret(e.key) ? "••••••" : e.value}</span>
                  <span class="lvl">{e.level}</span>
                {/if}
              </div>
            {/each}
          </div>
          <div class="crow add">
            <input class="mono k" placeholder="section.key" bind:value={newKey} spellcheck="false" aria-label="New setting name" />
            <input class="mono v" placeholder="value" bind:value={newValue} spellcheck="false" aria-label="New setting value"
              onkeydown={(e) => e.key === "Enter" && newKey.trim() && setConfig(newKey.trim(), newValue)} />
            <button class="btn small" disabled={!newKey.trim()} onclick={() => setConfig(newKey.trim(), newValue)}>Set</button>
          </div>
        {/if}
      </div>
    </div>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 100; background: var(--backdrop); display: grid; place-items: center; padding: 20px; }
  .box { width: min(820px, 100%); height: min(620px, 100%); display: grid; grid-template-columns: 190px 1fr; background: var(--surface); border: 1px solid var(--line); border-radius: 12px; box-shadow: var(--shadow-pop); overflow: hidden; }
  nav { background: var(--bg); border-right: 1px solid var(--line); padding: 16px 10px; display: flex; flex-direction: column; gap: 2px; }
  nav h2 { margin: 0 8px 2px; font-size: calc(var(--fs) * 16); }
  .path { margin: 0 8px 12px; font-size: calc(var(--fs) * 12); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  nav button { display: flex; align-items: center; gap: 9px; padding: 7px 10px; border-radius: 7px; color: var(--ink-2); font-weight: 600; text-align: left; }
  nav button:hover { background: var(--surface-3); color: var(--ink); }
  nav button.on { background: var(--sel); color: var(--ink); }
  .content { display: flex; flex-direction: column; min-height: 0; }
  header { display: flex; align-items: center; justify-content: space-between; padding: 16px 20px 6px; }
  h3 { margin: 0; font-size: calc(var(--fs) * 16); }
  .x { display: grid; place-items: center; width: 26px; height: 26px; border-radius: 6px; color: var(--ink-3); }
  .x:hover { background: var(--surface-3); color: var(--ink); }
  .scroll { flex: 1; overflow: auto; padding: 4px 20px 20px; display: flex; flex-direction: column; gap: 10px; }
  .hint { margin: 0; color: var(--ink-3); font-size: calc(var(--fs) * 12); }
  code { font-family: var(--font-mono); }
  .remote { border: 1px solid var(--line); border-radius: 8px; padding: 10px; display: flex; flex-direction: column; gap: 6px; }
  .rrow { display: flex; align-items: center; gap: 10px; }
  .lbl { width: 70px; flex: none; color: var(--ink-2); font-size: calc(var(--fs) * 12); }
  .rrow input { flex: 1; min-width: 0; }
  .ractions { display: flex; gap: 6px; align-items: center; }
  .sp { flex: 1; }
  .ign { flex: 1; min-height: 260px; resize: vertical; font-size: calc(var(--cs) * 12.5); line-height: 1.5; }
  .snips { display: flex; gap: 5px; flex-wrap: wrap; align-items: center; font-size: calc(var(--fs) * 12); }
  .cfgtools input { width: 100%; }
  .cfg { display: flex; flex-direction: column; border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
  .crow { display: grid; grid-template-columns: minmax(160px, 38%) 1fr auto; gap: 8px; align-items: center; padding: 4px 8px; border-bottom: 1px solid var(--line-soft); font-size: calc(var(--cs) * 12); min-height: 30px; }
  .crow:last-child { border-bottom: 0; }
  .crow .k { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--ink-2); }
  .crow .v { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .crow input.v { padding: 2px 6px; }
  .lvl { font-family: var(--font-ui); font-size: calc(var(--fs) * 10.5); color: var(--ink-3); border: 1px solid var(--line); border-radius: 4px; padding: 0 4px; }
  .crow.add { border: 1px dashed var(--line); border-radius: 8px; }
</style>
