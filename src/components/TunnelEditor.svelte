<script lang="ts">
  import { t as translateText } from '../i18n';
  import { untrack } from 'svelte';
  import Modal from './Modal.svelte';
  import { errorText, type Host, type Tunnel, type Mode } from '../types';
  let { tunnel, hosts, onsave, onclose }: { tunnel: Tunnel | null; hosts: Host[]; onsave: (draft: Tunnel) => Promise<void>; onclose: () => void } = $props();
  let draft = $state<Tunnel>(untrack(() => tunnel ? structuredClone($state.snapshot(tunnel)) : { id: crypto.randomUUID(), name: '', kind: 'Local', host_id: hosts[0]?.id ?? '', listen_address: '127.0.0.1', listen_port: 15432, target_host: '', target_port: 5432, auto_start: false }));
  let error = $state(''); let busy = $state(false);
  const modes: { value: Mode; label: string }[] = [{ value: 'Local', label: 'Local (-L)' }, { value: 'Remote', label: 'Remote (-R)' }, { value: 'Dynamic', label: 'Dynamic (-D)' }];
  async function save(event: SubmitEvent) {
    event.preventDefault(); error = ''; busy = true;
    try { await onsave({ ...draft, target_host: draft.kind === 'Dynamic' ? '' : draft.target_host, target_port: draft.kind === 'Dynamic' ? 0 : draft.target_port }); }
    catch (e) { error = errorText(e); } finally { busy = false; }
  }
</script>
<Modal title={tunnel ? $translateText("Edit tunnel") : $translateText("New tunnel")} {onclose}>
  <form onsubmit={save}>
    <div class="segmented" aria-label={$translateText("Forwarding mode")}>
      {#each modes as mode}<button type="button" class:selected={draft.kind === mode.value} aria-pressed={draft.kind === mode.value} onclick={() => draft.kind = mode.value}>{$translateText(mode.label)}</button>{/each}
    </div>
    <div class="form-grid">
      <label class="full">{$translateText("Name")}<input bind:value={draft.name} required maxlength="100" placeholder={$translateText("Production database")} /></label>
      <label class="full">{$translateText("SSH host profile")}<select bind:value={draft.host_id} required><option value="" disabled>{$translateText("Select an SSH host")}</option>{#each hosts as host}<option value={host.id}>{host.name} · {host.hostname}:{host.ssh_port}</option>{/each}</select></label>
      <div class="form-section full"><h3>{draft.kind === 'Remote' ? $translateText("Remote listener") : draft.kind === 'Dynamic' ? $translateText("Local SOCKS listener") : $translateText("Local listener")}</h3><p>{draft.kind === 'Remote' ? $translateText("The SSH server listens here and forwards traffic back to this computer.") : $translateText("Applications on this computer connect to this address.")}</p></div>
      <label>{draft.kind === 'Remote' ? $translateText("Remote listen address") : $translateText("Local listen address")}<input bind:value={draft.listen_address} required placeholder="127.0.0.1" /></label>
      <label>{draft.kind === 'Remote' ? $translateText("Remote listen port") : draft.kind === 'Dynamic' ? $translateText("Local SOCKS port") : $translateText("Local listen port")}<input type="number" min="1" max="65535" step="1" required bind:value={draft.listen_port} /></label>
      {#if draft.kind === 'Remote'}
        <div class="notice full"><strong>{$translateText("Remote listener requirements")}</strong><p>{$translateText("The SSH server must permit remote TCP forwarding (AllowTcpForwarding and PermitListen; account/key restrictions also apply).")}</p>
          {#if !['localhost', '::1', '[::1]'].includes(draft.listen_address.trim().toLowerCase()) && !/^127\./.test(draft.listen_address.trim())}
            <p>{$translateText("Listening beyond loopback requires server GatewayPorts policy. With OpenSSH, clientspecified permits the requested bind address; yes can force a wildcard bind. The server may refuse or override your requested address.")}</p>
          {/if}
          <p>{$translateText("A successful forward does not prove external or public access. Check the actual server bind address, firewall/security group, routing/NAT and target service, then connect from a separate external machine.")}</p>
        </div>
      {/if}
      {#if draft.kind !== 'Dynamic'}
        <div class="form-section full"><h3>{draft.kind === 'Remote' ? $translateText("Local target") : $translateText("Remote target")}</h3><p>{draft.kind === 'Remote' ? $translateText("The destination reached from this computer.") : $translateText("The destination reached from the SSH server.")}</p></div>
        <label>{draft.kind === 'Remote' ? $translateText("Local target host") : $translateText("Remote target host")}<input bind:value={draft.target_host} required placeholder={draft.kind === 'Remote' ? '127.0.0.1' : 'db.internal'} /></label>
        <label>{draft.kind === 'Remote' ? $translateText("Local target port") : $translateText("Remote target port")}<input type="number" min="1" max="65535" step="1" required bind:value={draft.target_port} /></label>
      {/if}
      <label class="check full"><input type="checkbox" bind:checked={draft.auto_start} /><span>{$translateText("Auto-start when TunnelWarden starts")}<small>{$translateText("Requires a verified host key. Encrypted key files need manual unlock after each app restart, or use an external SSH agent.")}</small></span></label>
    </div>
    {#if error}<p class="error" role="alert">{$translateText(error)}</p>{/if}
    {#if tunnel}<p class="hint">{$translateText("Saving an edit stops this tunnel. Start it again when you are ready.")}</p>{/if}
    <footer><button type="button" onclick={onclose} disabled={busy}>{$translateText("Cancel")}</button><button class="primary" disabled={busy || !hosts.length}>{busy ? $translateText("Saving…") : $translateText("Save tunnel")}</button></footer>
  </form>
</Modal>
