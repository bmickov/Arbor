<script lang="ts">
  import { app, CODE_SIZES, UI_SIZES } from "../lib/state.svelte";
  import {
    bindings, COMMANDS, commandFor, comboFromEvent, FIXED, GROUPS, prettyKeys, recording, resetBindings, setBinding,
  } from "../lib/shortcuts.svelte";
  import { THEMES, resolveTheme, type Theme, type ThemeId } from "../lib/themes";
  import Icon from "./Icon.svelte";

  type Section = "general" | "appearance" | "diffs" | "keyboard";
  const SECTIONS: { id: Section; label: string; icon: string }[] = [
    { id: "general", label: "General", icon: "history" },
    { id: "appearance", label: "Appearance", icon: "settings" },
    { id: "diffs", label: "Diffs & Git", icon: "split" },
    { id: "keyboard", label: "Keyboard", icon: "grid" },
  ];

  const s = app.settings;
  const save = () => app.saveSettings();

  // ------------------------------------------------------------ appearance
  const THEME_CHOICES: { id: ThemeId; name: string; description: string }[] = [
    { id: "system", name: "System", description: "Follows your desktop's light or dark setting." },
    ...THEMES.map((t) => ({ id: t.id as ThemeId, name: t.name, description: t.description })),
  ];

  function setTheme(id: ThemeId) {
    s.theme = id;
    save();
  }

  function swatch(t: Theme) {
    const k = t.tokens;
    return `--p-bg:${k.bg};--p-surface:${k.surface};--p-line:${k.line};--p-ink:${k["ink-3"]};--p-accent:${k.accent};--p-sel:${k.sel};--p-l0:${k["lane-0"]};--p-l1:${k["lane-1"]};--p-l2:${k["lane-2"]};--p-add:${k["add-bg-strong"]};--p-del:${k["del-bg-strong"]}`;
  }

  // -------------------------------------------------------------- keyboard
  let filter = $state("");
  let recordingId = $state<string | null>(null);
  let note = $state<{ id: string; text: string } | null>(null);

  const fq = $derived(filter.trim().toLowerCase());
  const visibleGroups = $derived(
    GROUPS.map((g) => ({
      group: g,
      cmds: COMMANDS.filter((c) => c.group === g && (!fq || c.label.toLowerCase().includes(fq) || bindings(c.id).some((b) => b.toLowerCase().includes(fq)))),
    })).filter((g) => g.cmds.length),
  );
  const customized = $derived(Object.keys(s.shortcuts).length > 0);

  function startRecording(id: string) {
    recordingId = id;
    recording.active = true;
    note = null;
  }

  function stopRecording() {
    recordingId = null;
    recording.active = false;
  }

  // Capture the next key combination while recording.
  function onKeyCapture(e: KeyboardEvent) {
    if (!recordingId) {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey) return stopRecording();
    if ((e.key === "Backspace" || e.key === "Delete") && !e.ctrlKey && !e.altKey && !e.shiftKey) {
      setBinding(recordingId, []);
      note = { id: recordingId, text: "No shortcut." };
      return stopRecording();
    }
    const combo = comboFromEvent(e);
    if (!combo) return; // a modifier on its own: keep waiting
    if (/^Alt\+[1-9]$/.test(combo)) {
      note = { id: recordingId, text: `${combo} is reserved for switching tabs.` };
      return stopRecording();
    }
    const id = recordingId;
    const other = commandFor(combo, id);
    if (other) {
      setBinding(other.id, bindings(other.id).filter((b) => b !== combo));
      note = { id, text: `Removed from “${other.label}”.` };
    }
    setBinding(id, [combo]);
    stopRecording();
  }

  function resetOne(id: string) {
    const def = COMMANDS.find((c) => c.id === id)!.keys;
    // Take the default keys back from any command now using them.
    for (const k of def) {
      const other = commandFor(k, id);
      if (other) setBinding(other.id, bindings(other.id).filter((b) => b !== k));
    }
    setBinding(id, def);
    note = null;
  }

  function close() {
    stopRecording();
    app.prefs = null;
  }

  $effect(() => () => {
    recording.active = false;
  });
</script>

<svelte:window onkeydowncapture={onKeyCapture} />

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="backdrop" onclick={(e) => e.target === e.currentTarget && close()}>
  <div class="prefs" role="dialog" aria-modal="true" aria-labelledby="prefs-title">
    <nav>
      <h2 id="prefs-title">Preferences</h2>
      {#each SECTIONS as sec}
        <button class:on={app.prefs === sec.id} onclick={() => { stopRecording(); app.prefs = sec.id; }}>
          <Icon name={sec.icon} size={15} /> {sec.label}
        </button>
      {/each}
    </nav>

    <div class="content">
      <header>
        <h3>{SECTIONS.find((x) => x.id === app.prefs)?.label}</h3>
        <button class="close" onclick={close} aria-label="Close preferences"><Icon name="x" /></button>
      </header>

      <div class="scroll">
        {#if app.prefs === "general"}
          <section>
            <h4>Startup</h4>
            <label class="toggle">
              <input type="checkbox" checked={s.restoreSession}
                onchange={() => {
                  s.restoreSession = !s.restoreSession;
                  save();
                  if (s.restoreSession) app.saveSession();
                  else app.forgetSession();
                }} />
              <span>
                <b>Pick up where you left off</b>
                <small>
                  When Arbor starts, reopen your tabs and bring back each one's view, selected commit or file,
                  search and scroll position, plus the panel layout and window size.
                  Turn this off to start on the repository list every time.
                </small>
              </span>
            </label>
          </section>
        {:else if app.prefs === "appearance"}
          <section>
            <h4>Theme</h4>
            <div class="themes" role="radiogroup" aria-label="Theme">
              {#each THEME_CHOICES as t}
                {@const preview = t.id === "system" ? null : resolveTheme(t.id)}
                <button class="theme" class:on={s.theme === t.id} role="radio" aria-checked={s.theme === t.id} onclick={() => setTheme(t.id)}>
                  {#if preview}
                    <span class="swatch" style={swatch(preview)}>
                      <span class="sw-top"></span>
                      <span class="sw-side"></span>
                      <span class="sw-main">
                        <span class="sw-row sel"><i style="background:var(--p-l0)"></i><b></b></span>
                        <span class="sw-row"><i style="background:var(--p-l1)"></i><b></b></span>
                        <span class="sw-row"><i style="background:var(--p-l2)"></i><b></b></span>
                        <span class="sw-diff"><em class="add"></em><em class="del"></em></span>
                      </span>
                    </span>
                  {:else}
                    <span class="swatch split">
                      <span class="half" style={swatch(resolveTheme("light"))}><span class="sw-top"></span><span class="sw-main"><span class="sw-row sel"><i style="background:var(--p-l0)"></i><b></b></span></span></span>
                      <span class="half" style={swatch(resolveTheme("dark"))}><span class="sw-top"></span><span class="sw-main"><span class="sw-row sel"><i style="background:var(--p-l0)"></i><b></b></span></span></span>
                    </span>
                  {/if}
                  <span class="tname">{t.name}{#if s.theme === t.id}<Icon name="check" size={13} />{/if}</span>
                  <span class="tdesc">{t.description}</span>
                </button>
              {/each}
            </div>
            {#if s.theme === "ubuntu-dark" && s.uiFont !== "system"}
              <p class="tip">
                Tip: Ubuntu Dark looks most at home with the system font.
                <button class="link" onclick={() => { s.uiFont = "system"; s.codeFont = "system"; save(); }}>Use system fonts</button>
              </p>
            {/if}
          </section>

          <section>
            <h4>Interface text</h4>
            <div class="row">
              <span class="label">Font</span>
              <div class="seg" role="radiogroup" aria-label="Interface font">
                <button role="radio" aria-checked={s.uiFont === "atkinson"} class:on={s.uiFont === "atkinson"} onclick={() => { s.uiFont = "atkinson"; save(); }}>Atkinson Hyperlegible</button>
                <button role="radio" aria-checked={s.uiFont === "system"} class:on={s.uiFont === "system"} onclick={() => { s.uiFont = "system"; save(); }}>System font</button>
              </div>
            </div>
            <div class="row">
              <span class="label">Size</span>
              <div class="size">
                <button class="step" aria-label="Smaller" disabled={s.uiSize <= UI_SIZES.min} onclick={() => app.setUiSize(s.uiSize - 1)}><Icon name="minus" size={13} /></button>
                <input type="range" min={UI_SIZES.min} max={UI_SIZES.max} step="1" value={s.uiSize} aria-label="Interface text size"
                  oninput={(e) => app.setUiSize(Number((e.currentTarget as HTMLInputElement).value))} />
                <button class="step" aria-label="Larger" disabled={s.uiSize >= UI_SIZES.max} onclick={() => app.setUiSize(s.uiSize + 1)}><Icon name="plus" size={13} /></button>
                <span class="val">{s.uiSize} px</span>
                <button class="link" disabled={s.uiSize === UI_SIZES.default} onclick={() => app.setUiSize(UI_SIZES.default)}>Reset</button>
              </div>
            </div>
            <p class="hint">Also with <span class="kbd">Ctrl</span> <span class="kbd">=</span> and <span class="kbd">Ctrl</span> <span class="kbd">-</span> anywhere in Arbor.</p>
          </section>

          <section>
            <h4>Code and diffs</h4>
            <div class="row">
              <span class="label">Font</span>
              <div class="seg" role="radiogroup" aria-label="Code font">
                <button role="radio" aria-checked={s.codeFont === "commit"} class:on={s.codeFont === "commit"} onclick={() => { s.codeFont = "commit"; save(); }}>Commit Mono</button>
                <button role="radio" aria-checked={s.codeFont === "system"} class:on={s.codeFont === "system"} onclick={() => { s.codeFont = "system"; save(); }}>System monospace</button>
              </div>
            </div>
            <div class="row">
              <span class="label">Size</span>
              <div class="size">
                <button class="step" aria-label="Smaller" disabled={s.codeSize <= CODE_SIZES.min} onclick={() => app.setCodeSize(s.codeSize - 1)}><Icon name="minus" size={13} /></button>
                <input type="range" min={CODE_SIZES.min} max={CODE_SIZES.max} step="1" value={s.codeSize} aria-label="Code text size"
                  oninput={(e) => app.setCodeSize(Number((e.currentTarget as HTMLInputElement).value))} />
                <button class="step" aria-label="Larger" disabled={s.codeSize >= CODE_SIZES.max} onclick={() => app.setCodeSize(s.codeSize + 1)}><Icon name="plus" size={13} /></button>
                <span class="val">{s.codeSize} px</span>
                <button class="link" disabled={s.codeSize === CODE_SIZES.default} onclick={() => app.setCodeSize(CODE_SIZES.default)}>Reset</button>
              </div>
            </div>
            <div class="sample mono" aria-hidden="true">
              <div class="sl"><span class="n">12</span><span class="c">  const total = items.reduce((a, b) =&gt; a + b, 0);</span></div>
              <div class="sl del"><span class="n">13</span><span class="c">- return total;</span></div>
              <div class="sl add"><span class="n">13</span><span class="c">+ return Math.round(total * 100) / 100;</span></div>
            </div>
          </section>
        {:else if app.prefs === "diffs"}
          <section>
            <h4>Diffs</h4>
            <div class="row">
              <span class="label">Context lines</span>
              <div class="seg" role="radiogroup" aria-label="Context lines">
                {#each [1, 3, 6, 10] as n}
                  <button role="radio" aria-checked={s.context === n} class:on={s.context === n} onclick={() => { s.context = n; save(); }}>{n}</button>
                {/each}
              </div>
            </div>
            <label class="toggle">
              <input type="checkbox" checked={s.ignoreWhitespace} onchange={() => { s.ignoreWhitespace = !s.ignoreWhitespace; save(); }} />
              <span><b>Ignore whitespace changes</b><small>Hide lines that only differ in indentation or spacing.</small></span>
            </label>
            <label class="toggle">
              <input type="checkbox" checked={s.splitDiff} onchange={() => { s.splitDiff = !s.splitDiff; save(); }} />
              <span><b>Side-by-side diffs in history</b><small>Show old and new versions in two columns when viewing commits.</small></span>
            </label>
          </section>
          <section>
            <h4>Git</h4>
            <label class="toggle">
              <input type="checkbox" checked={s.pullRebase} onchange={() => { s.pullRebase = !s.pullRebase; save(); }} />
              <span><b>Pull with rebase</b><small>Replay your local commits on top of the remote ones instead of creating a merge commit.</small></span>
            </label>
          </section>
        {:else}
          <section class="kb">
            <div class="kbtools">
              <div class="search">
                <Icon name="search" size={14} />
                <input placeholder="Filter by action or key" bind:value={filter} spellcheck="false" />
              </div>
              <button class="btn small" disabled={!customized} onclick={() => { resetBindings(); note = null; }}>Reset all</button>
            </div>
            <p class="hint">Click a shortcut, then press the keys you want. <span class="kbd">Esc</span> cancels, <span class="kbd">⌫</span> removes it.</p>
            {#each visibleGroups as g (g.group)}
              <h4>{g.group}</h4>
              <div class="cmds">
                {#each g.cmds as c (c.id)}
                  {@const keys = bindings(c.id)}
                  {@const custom = !!s.shortcuts[c.id]}
                  <div class="cmd">
                    <span class="clabel">
                      {c.label}
                      {#if note?.id === c.id}<small class="note">{note.text}</small>{/if}
                    </span>
                    <button class="keys" class:rec={recordingId === c.id} onclick={() => (recordingId === c.id ? stopRecording() : startRecording(c.id))}
                      aria-label="Change shortcut for {c.label}">
                      {#if recordingId === c.id}
                        <span class="press">Press keys…</span>
                      {:else if keys.length}
                        {#each keys as k, i}
                          {#if i > 0}<span class="or">or</span>{/if}
                          <span class="combo">{#each prettyKeys(k) as part}<span class="kbd">{part}</span>{/each}</span>
                        {/each}
                      {:else}
                        <span class="none">None</span>
                      {/if}
                    </button>
                    <button class="reset" class:show={custom} disabled={!custom} title="Reset to default" aria-label="Reset {c.label} to default" onclick={() => resetOne(c.id)}><Icon name="undo" size={13} /></button>
                  </div>
                {/each}
              </div>
            {/each}
            {#if !fq}
              <h4>Built in</h4>
              <div class="cmds">
                {#each FIXED as f}
                  <div class="cmd fixed"><span class="clabel">{f.label}</span><span class="keys static"><span class="kbd">{f.keys}</span></span><span class="reset"></span></div>
                {/each}
              </div>
            {/if}
          </section>
        {/if}
      </div>

      <footer>
        <span class="faint">Changes apply right away.</span>
        <button class="btn primary" onclick={close}>Done</button>
      </footer>
    </div>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 100; background: var(--backdrop); display: grid; place-items: center; padding: 20px; }
  .prefs {
    width: min(860px, 100%); height: min(640px, 100%); display: grid; grid-template-columns: 200px 1fr;
    background: var(--surface); border: 1px solid var(--line); border-radius: 12px; box-shadow: var(--shadow-pop); overflow: hidden;
    animation: rise 0.14s ease-out;
  }
  @keyframes rise { from { translate: 0 6px; opacity: 0; } }
  nav { background: var(--bg); border-right: 1px solid var(--line); padding: 16px 10px; display: flex; flex-direction: column; gap: 2px; }
  nav h2 { margin: 0 8px 12px; font-size: calc(var(--fs) * 16); font-weight: 700; }
  nav button { display: flex; align-items: center; gap: 9px; padding: 7px 10px; border-radius: 7px; color: var(--ink-2); font-weight: 600; text-align: left; }
  nav button:hover { background: var(--surface-3); color: var(--ink); }
  nav button.on { background: var(--sel); color: var(--ink); }

  .content { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  header { display: flex; align-items: center; justify-content: space-between; padding: 14px 16px 10px 24px; }
  h3 { margin: 0; font-size: calc(var(--fs) * 17); font-weight: 700; }
  .close { display: grid; place-items: center; width: 30px; height: 30px; border-radius: 7px; color: var(--ink-2); }
  .close:hover { background: var(--surface-3); color: var(--ink); }
  .scroll { flex: 1; overflow-y: auto; padding: 0 24px 20px; }
  footer { display: flex; align-items: center; justify-content: space-between; padding: 10px 16px 12px 24px; border-top: 1px solid var(--line-soft); font-size: calc(var(--fs) * 12); }

  section { padding: 10px 0 14px; border-bottom: 1px solid var(--line-soft); }
  section:last-child { border-bottom: 0; }
  h4 { margin: 8px 0 10px; font-size: calc(var(--fs) * 13); font-weight: 700; color: var(--ink-2); }

  .themes { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; }
  .theme {
    display: flex; flex-direction: column; gap: 4px; padding: 8px; border-radius: 10px; text-align: left;
    border: 1px solid var(--line); background: var(--surface);
  }
  .theme:hover { border-color: var(--ink-3); }
  .theme.on { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-soft); }
  .tname { display: flex; align-items: center; justify-content: space-between; font-weight: 700; margin-top: 4px; }
  .tname :global(svg) { color: var(--accent); }
  .tdesc { font-size: calc(var(--fs) * 11.5); color: var(--ink-3); line-height: 1.35; }

  .swatch {
    position: relative; display: grid; grid-template-columns: 22% 1fr; grid-template-rows: 10px 1fr;
    height: 74px; border-radius: 6px; overflow: hidden; background: var(--p-surface); border: 1px solid var(--p-line);
  }
  .swatch.split { display: grid; grid-template-columns: 1fr 1fr; grid-template-rows: 1fr; background: none; }
  .half { display: grid; grid-template-rows: 10px 1fr; background: var(--p-surface); }
  .sw-top { grid-column: 1 / -1; background: var(--p-bg); border-bottom: 1px solid var(--p-line); }
  .sw-side { background: var(--p-bg); border-right: 1px solid var(--p-line); }
  .sw-main { display: flex; flex-direction: column; gap: 4px; padding: 6px; }
  .sw-row { display: flex; align-items: center; gap: 5px; height: 9px; padding: 0 3px; border-radius: 3px; }
  .sw-row.sel { background: var(--p-sel); }
  .sw-row i { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  .sw-row b { height: 3px; flex: 1; border-radius: 2px; background: var(--p-ink); opacity: 0.6; }
  .sw-diff { display: flex; flex-direction: column; gap: 2px; margin-top: 2px; }
  .sw-diff em { height: 5px; border-radius: 2px; }
  .sw-diff .add { background: var(--p-add); }
  .sw-diff .del { background: var(--p-del); width: 70%; }

  .row { display: flex; align-items: center; gap: 16px; margin: 8px 0; }
  .label { width: 110px; color: var(--ink-2); flex: none; }
  .seg { display: inline-flex; background: var(--surface-3); border-radius: 8px; padding: 2px; flex-wrap: wrap; }
  .seg button { padding: 4px 12px; border-radius: 6px; font-weight: 600; color: var(--ink-2); }
  .seg button.on { background: var(--surface); color: var(--ink); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.15); }
  .size { display: flex; align-items: center; gap: 8px; }
  .size input[type="range"] { width: 180px; accent-color: var(--accent); }
  .step { display: grid; place-items: center; width: 26px; height: 26px; border-radius: 6px; border: 1px solid var(--line); background: var(--btn-bg); }
  .step:hover:not(:disabled) { background: var(--btn-hover); }
  .val { width: 44px; font-variant-numeric: tabular-nums; color: var(--ink-2); }
  .link { color: var(--accent); font-weight: 600; }
  .link:disabled { color: var(--ink-3); }
  .hint { margin: 6px 0 0; font-size: calc(var(--fs) * 12); color: var(--ink-3); }
  .tip { margin: 12px 0 0; padding: 8px 12px; border-radius: 8px; background: var(--accent-soft); font-size: calc(var(--fs) * 12); display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }

  .sample { margin-top: 12px; border: 1px solid var(--line); border-radius: 8px; overflow: hidden; background: var(--surface); line-height: 1.6; }
  .sl { display: grid; grid-template-columns: 40px 1fr; white-space: pre; }
  .sl .n { text-align: right; padding-right: 10px; color: var(--ink-3); font-size: calc(var(--cs) * 11); }
  .sl.add { background: var(--add-bg); }
  .sl.del { background: var(--del-bg); }

  .toggle { display: flex; gap: 12px; align-items: flex-start; padding: 8px 0; }
  .toggle input { margin-top: 3px; }
  .toggle span { display: flex; flex-direction: column; gap: 2px; }
  .toggle small { color: var(--ink-3); font-size: calc(var(--fs) * 12); }

  .kb { padding-top: 4px; }
  .kbtools { display: flex; gap: 10px; align-items: center; }
  .search {
    flex: 1; display: flex; align-items: center; gap: 6px; padding: 0 9px; height: 30px;
    border: 1px solid var(--line); border-radius: var(--radius); color: var(--ink-3); background: var(--input-bg);
  }
  .search input { border: 0; padding: 0; height: 28px; background: transparent; box-shadow: none; }
  .cmds { display: flex; flex-direction: column; }
  .cmd { display: grid; grid-template-columns: 1fr auto 28px; align-items: center; gap: 8px; padding: 3px 0; border-bottom: 1px solid var(--line-soft); }
  .cmd:last-child { border-bottom: 0; }
  .clabel { display: flex; flex-direction: column; }
  .note { color: var(--warn); font-size: calc(var(--fs) * 11.5); }
  .keys {
    display: flex; align-items: center; gap: 6px; justify-content: flex-end; min-width: 150px; min-height: 28px;
    padding: 3px 8px; border-radius: 7px; border: 1px solid transparent;
  }
  .keys:hover:not(.static) { border-color: var(--line); background: var(--surface-2); }
  .keys.rec { border-color: var(--accent); background: var(--accent-soft); }
  .keys.static { cursor: default; }
  .combo { display: inline-flex; gap: 3px; }
  .or { color: var(--ink-3); font-size: calc(var(--fs) * 11); }
  .press { color: var(--accent); font-weight: 600; }
  .none { color: var(--ink-3); }
  .reset { display: grid; place-items: center; width: 26px; height: 26px; border-radius: 6px; color: var(--ink-3); visibility: hidden; }
  .reset.show { visibility: visible; }
  .reset:hover { background: var(--surface-3); color: var(--ink); }
  .kb h4 { margin-top: 18px; }

  @media (max-width: 760px) {
    .prefs { grid-template-columns: 1fr; }
    nav { flex-direction: row; flex-wrap: wrap; border-right: 0; border-bottom: 1px solid var(--line); }
    .themes { grid-template-columns: repeat(2, 1fr); }
  }
</style>
