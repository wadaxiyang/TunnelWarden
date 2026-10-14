<script lang="ts">
  import { t as translateText } from '../i18n';
  import { untrack } from 'svelte';
  import Modal from './Modal.svelte';
  import { errorText, type Host, type HostInput } from '../types';
  let { host, hosts, onsave, onclose }: { host: Host | null; hosts: Host[]; onsave: (input: HostInput) => Promise<void>; onclose: () => void } = $props();
  let draft = $state<Host>(untrack(() => host ? structuredClone($state.snapshot(host)) : { id: crypto.randomUUID(), name: '', hostname: '', ssh_port: 22, username: '', auth_method: 'Agent', identity_file: '', jump_chain: [], keepalive_seconds: 30, has_password: false }));
  let password = $state(''); let clearPassword = $state(false); let hop = $state('');
  let busy = $state(false); let error = $state('');
  let candidates = $derived(hosts.filter(h => h.id !== draft.id && !draft.jump_chain.includes(h.id)));
  function move(index: number, delta: number) { const chain = [...draft.jump_chain]; [chain[index], chain[index + delta]] = [chain[index + delta], chain[index]]; draft.jump_chain = chain; }
  async function save(event: SubmitEvent) {
    event.preventDefault(); error = ''; busy = true;
    try { await onsave({ profile: draft, password: password || null, clear_password: clearPassword }); password = ''; }
    catch (e) { error = errorText(e); } finally { busy = false; }
  }
</script>
<Modal title={host ? $translateText("Edit SSH host") : $translateText("New SSH host")} {onclose}>
  <form onsubmit={save}>
    <div class="form-grid">
      <label class="full">{$translateText("Name")}<input bind:value={draft.name} required maxlength="100" placeholder={$translateText("Production gateway")} /></label>
      <label>{$translateText("Hostname / IP")}<input bind:value={draft.hostname} required placeholder="server.example.com" /></label>
      <label>{$translateText("SSH port")}<input type="number" min="1" max="65535" step="1" required bind:value={draft.ssh_port} /></label>
      <label>{$translateText("Username")}<input bind:value={draft.username} required autocomplete="off" placeholder="deploy" /></label>
      <label>{$translateText("Authentication method")}<select bind:value={draft.auth_method}><option value="Agent">{$translateText("SSH agent")}</option><option value="Key">{$translateText("Private key file")}</option><option value="Password">{$translateText("Password")}</option><option value="Auto">{$translateText("Automatic (key / agent / password)")}</option></select></label>
      {#if draft.auth_method === 'Key' || draft.auth_method === 'Auto'}
        <label class="full">{$translateText("Private key file")}<input bind:value={draft.identity_file} required={draft.auth_method === 'Key'} placeholder="C:\Users\you\.ssh\id_ed25519" /><small>{$translateText("OpenSSH / PEM / PuTTY PPK file. Encrypted keys ask for a passphrase when you test or connect. Unlock lasts until the app quits; the passphrase is never saved.")}</small></label>
      {/if}
      {#if draft.auth_method !== 'Agent'}
        <label class="full">{draft.has_password ? $translateText("Replace stored password (optional)") : $translateText("Password")}<input type="password" bind:value={password} autocomplete="new-password" disabled={clearPassword} required={draft.auth_method === 'Password' && !draft.has_password && !clearPassword} /><small>{draft.has_password ? $translateText("Leave blank to keep the password saved in Windows Credential Manager.") : $translateText("Stored in Windows Credential Manager, never in the configuration file.")}</small></label>
        {#if draft.has_password}<label class="check full"><input type="checkbox" bind:checked={clearPassword} />{$translateText("Remove the stored password")}</label>{/if}
      {:else}<p class="hint full">{$translateText("Uses the Windows OpenSSH agent or an available Pageant / KeePassXC agent.")}</p>{/if}
      <div class="form-section full"><h3>{$translateText("Jump route")}</h3><p>{$translateText("Optional hops, in connection order. Each hop uses its own saved credentials.")}</p></div>
      <div class="full hops">
        {#each draft.jump_chain as id, index}<div class="hop"><span>{index + 1}. {hosts.find(h => h.id === id)?.name ?? $translateText("Missing host")}</span><button type="button" disabled={index === 0} onclick={() => move(index, -1)} aria-label={$translateText("Move hop up")}>↑</button><button type="button" disabled={index === draft.jump_chain.length - 1} onclick={() => move(index, 1)} aria-label={$translateText("Move hop down")}>↓</button><button type="button" onclick={() => draft.jump_chain = draft.jump_chain.filter(h => h !== id)} aria-label={$translateText("Remove hop")}>×</button></div>{/each}
        <div class="inline"><label class="grow">{$translateText("Add jump host")}<select bind:value={hop}><option value="">{candidates.length ? $translateText("Choose a saved host") : $translateText("No other hosts available")}</option>{#each candidates as candidate}<option value={candidate.id}>{candidate.name}</option>{/each}</select></label><button type="button" disabled={!hop} onclick={() => { draft.jump_chain = [...draft.jump_chain, hop]; hop = ''; }}>{$translateText("Add hop")}</button></div>
      </div>
      <label class="full">{$translateText("Keepalive interval (seconds)")}<input type="number" min="0" max="3600" step="1" required bind:value={draft.keepalive_seconds} /><small>{$translateText("0 disables SSH keepalive. Default: 30 seconds.")}</small></label>
    </div>
    {#if error}<p class="error" role="alert">{$translateText(error)}</p>{/if}
    {#if host}<p class="hint">{$translateText("Stop tunnels using this host or its jump route before saving changes.")}</p>{/if}
    <footer><button type="button" disabled={busy} onclick={onclose}>{$translateText("Cancel")}</button><button class="primary" disabled={busy}>{busy ? $translateText("Saving…") : $translateText("Save host")}</button></footer>
  </form>
</Modal>
