<script lang="ts">
  // Preferences → Commits → Signing: commit.gpgsign, gpg.format and
  // user.signingkey, for the open repository or for all of them.
  import { api } from "../lib/api";
  import { app } from "../lib/state.svelte";
  import { toasts } from "../lib/ui.svelte";

  let enabled = $state(false);
  let format = $state<"openpgp" | "ssh" | "x509">("openpgp");
  let key = $state("");
  let global = $state(false);
  let loadedFor = "";
  let saving = $state(false);

  $effect(() => {
    if (!app.path) return;
    if (loadedFor !== app.path) app.loadSigning();
  });
  // Take the current settings whenever they're (re)loaded for a new repository.
  $effect(() => {
    const s = app.signing;
    if (!s || loadedFor === app.path) return;
    loadedFor = app.path;
    enabled = s.enabled;
    format = s.format;
    key = s.key;
    global = s.scope === "global";
  });

  const s = $derived(app.signing);
  const dirty = $derived(!!s && (enabled !== s.enabled || (enabled && (format !== s.format || key.trim() !== s.key)) || (global !== (s.scope === "global") && (enabled || s.enabled))));
  const repoName = $derived(app.path.split(/[\\/]/).pop());
  const missing = $derived(s && enabled ? (format === "ssh" ? !s.sshAvailable && "ssh-keygen (OpenSSH)" : !s.gpgAvailable && "GnuPG (gpg)") : false);

  async function apply() {
    saving = true;
    try {
      await api.setSigning(app.path, enabled, format, key, global);
      loadedFor = "";
      await app.loadSigning();
      toasts.push("success", enabled ? `New commits ${global ? "everywhere" : `in ${repoName}`} will be signed.` : "Commits won't be signed.");
    } catch (e) {
      toasts.push("error", String(e));
    } finally {
      saving = false;
    }
  }
</script>

<section>
  <h4>Signing</h4>
  {#if !app.path}
    <p class="hint first">Open a repository to set up commit signing.</p>
  {:else if s}
    <p class="hint first">Signed commits prove they came from you. Arbor uses the same settings and tools as Git, so commits made on the command line are signed the same way. Each commit can opt out with <b>Sign</b> under the message.</p>
    <label class="toggle">
      <input type="checkbox" bind:checked={enabled} />
      <span><b>Sign new commits</b><small>Also merges, cherry-picks, reverts and rewritten commits.</small></span>
    </label>
    {#if enabled}
      <div class="row">
        <span class="label">Sign with</span>
        <div class="seg" role="radiogroup" aria-label="Signature type">
          <button role="radio" aria-checked={format === "openpgp"} class:on={format === "openpgp"} onclick={() => (format = "openpgp")}>GPG key</button>
          <button role="radio" aria-checked={format === "ssh"} class:on={format === "ssh"} onclick={() => (format = "ssh")}>SSH key</button>
          {#if s.format === "x509"}<button role="radio" aria-checked={format === "x509"} class:on={format === "x509"} onclick={() => (format = "x509")}>X.509</button>{/if}
        </div>
      </div>
      <div class="row">
        <span class="label">Key</span>
        <input class="mono" bind:value={key} spellcheck="false" aria-label="Signing key"
          placeholder={format === "ssh" ? "~/.ssh/id_ed25519.pub" : "Key ID or email — blank uses your commit email"} />
      </div>
      {#if format === "ssh"}
        <p class="hint">A key file, or a public key (<code>ssh-ed25519 AAAA…</code>) held by ssh-agent. To mark signatures as verified, list trusted keys in an allowed signers file (<code>gpg.ssh.allowedSignersFile</code>){s.allowedSigners ? `, currently ${s.allowedSigners}` : ""}.</p>
      {:else}
        <p class="hint">Find your key IDs with <code>gpg --list-secret-keys --keyid-format=long</code>. GnuPG asks for the passphrase in its own window when needed.</p>
      {/if}
      {#if missing}<p class="warn">{missing} wasn't found. Install it, or signing will fail.</p>{/if}
    {/if}
    <div class="row">
      <span class="label">Applies to</span>
      <div class="seg" role="radiogroup" aria-label="Applies to">
        <button role="radio" aria-checked={!global} class:on={!global} onclick={() => (global = false)}>{repoName}</button>
        <button role="radio" aria-checked={global} class:on={global} onclick={() => (global = true)}>All repositories</button>
      </div>
    </div>
    <div class="actions">
      <button class="btn small primary" disabled={!dirty || saving} onclick={apply}>Apply</button>
    </div>
  {/if}
</section>

<style>
  /* The same look as the rest of Preferences (whose styles are scoped there). */
  section { padding: 10px 0 14px; border-bottom: 1px solid var(--line-soft); }
  h4 { margin: 8px 0 10px; font-size: calc(var(--fs) * 13); font-weight: 700; color: var(--ink-2); }
  .row { display: flex; align-items: center; gap: 16px; margin: 8px 0; }
  .label { width: 110px; color: var(--ink-2); flex: none; }
  .seg { display: inline-flex; background: var(--surface-3); border-radius: 8px; padding: 2px; flex-wrap: wrap; }
  .seg button { padding: 4px 12px; border-radius: 6px; font-weight: 600; color: var(--ink-2); }
  .seg button.on { background: var(--surface); color: var(--ink); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.15); }
  .hint { margin: 6px 0 0; font-size: calc(var(--fs) * 12); color: var(--ink-3); }
  .hint.first { margin: 0 0 10px; }
  .toggle { display: flex; gap: 12px; align-items: flex-start; padding: 8px 0; }
  .toggle input { margin-top: 3px; }
  .toggle span { display: flex; flex-direction: column; gap: 2px; }
  .toggle small { color: var(--ink-3); font-size: calc(var(--fs) * 12); }
  code { font-family: var(--font-mono); }
  .row input { flex: 1; min-width: 0; }
  .warn { color: var(--warn); font-size: calc(var(--fs) * 12); margin: 4px 0; }
  .actions { display: flex; justify-content: flex-end; margin-top: 6px; }
  code { font-size: calc(var(--cs) * 11.5); }
</style>
