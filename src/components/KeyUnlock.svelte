<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import Modal from './Modal.svelte';
  import { errorText, type KeyPassphrasePrompt } from '../types';
  let { question }: { question: KeyPassphrasePrompt } = $props();
  let passphrase = $state(''); let busy = $state(false); let error = $state('');
  async function respond(cancel = false) {
    if (busy) return;
    busy = true; error = '';
    const value = cancel ? null : passphrase;
    passphrase = '';
    try { await invoke('respond_key_passphrase', { promptId: question.id, passphrase: value }); }
    catch (e) { error = errorText(e); } finally { busy = false; }
  }
</script>
<Modal title="Unlock private key" onclose={() => void respond(true)}>
  <form onsubmit={e => { e.preventDefault(); void respond(); }}>
    <p>Enter the private-key passphrase for <strong>{question.host_name}</strong>.</p>
    <p class="fingerprint">{question.identity_file}</p>
    <label>Private-key passphrase<input type="password" bind:value={passphrase} autocomplete="off" required disabled={busy} /></label>
    <p class="hint">The passphrase is never saved. The unlocked key stays in memory until TunnelWarden quits, so background reconnect can continue. Closing the interface does not lock it.</p>
    {#if question.error}<p class="error" role="alert">{question.error}</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <footer><button type="button" disabled={busy} onclick={() => respond(true)}>Cancel connection</button><button class="primary" disabled={busy || !passphrase}>{busy ? 'Unlocking…' : 'Unlock and connect'}</button></footer>
  </form>
</Modal>
