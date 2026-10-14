<script lang="ts">
  import { t as translateText, locale, resolveLanguage, detectLanguage, type Locale } from './i18n';
  import { version } from '../package.json';
  import { onMount } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import HostEditor from './components/HostEditor.svelte';
  import TunnelEditor from './components/TunnelEditor.svelte';
  import TunnelDetails from './components/TunnelDetails.svelte';
  import Modal from './components/Modal.svelte';
  import KeyUnlock from './components/KeyUnlock.svelte';
  import { endpoint, errorText, type Host, type HostInput, type Tunnel, type Snapshot, type Settings, type LogEntry, type HostKeyPrompt, type KeyPassphrasePrompt, type HostTestResult } from './types';
  import license from '../LICENSE?raw';
  import notice from '../NOTICE.ORYXIS?raw';
  const pages = ['Tunnels', 'SSH Hosts', 'Logs', 'Settings', 'About'] as const;
  let detailId = $state<string | null>(null);
  let page = $state<(typeof pages)[number]>('Tunnels');
  let hosts = $state<Host[]>([]); let tunnels = $state<Tunnel[]>([]);
  let runtime = $state<Snapshot>({ tunnels: [] }); let logs = $state<LogEntry[]>([]);
  let settings = $state<Settings>({ language: 'system', auto_reconnect: true, launch_at_login: false, close_to_tray: true });
  let questions = $state<HostKeyPrompt[]>([]);
  let keyQuestions = $state<KeyPassphrasePrompt[]>([]);
  let hostTests = $state<Record<string, { running: boolean; error: boolean; message: string }>>({});
  let exportPath = $state('');
  let closeConfirmation = $state(false); let savedSettings = $state('');
  let loading = $state(true); let error = $state(''); let busy = $state<string | null>(null);
  let tunnelEditor = $state(false); let hostEditor = $state(false);
  let editingTunnel = $state<Tunnel | null>(null); let editingHost = $state<Host | null>(null);
  let deletion = $state<{ kind: 'host' | 'tunnel'; id: string; name: string } | null>(null);
  let search = $state(''); let logLevel = $state('all'); let settingsSaved = $state(false);
  const native = isTauri();
  let systemLanguage = $state<Locale>(detectLanguage(navigator.language));
  $effect(() => {
    const selected = resolveLanguage(settings.language, systemLanguage);
    locale.set(selected);
    document.documentElement.lang = selected;
  });
  let selectedTunnel = $derived(tunnels.find(t => t.id === detailId));
  let statusById = $derived(new Map(runtime.tunnels.map(r => [r.id, r])));
  let running = $derived(runtime.tunnels.filter(r => r.status === 'running').length);
  let visibleTunnels = $derived(tunnels.filter(t => `${t.name} ${hosts.find(h => h.id === t.host_id)?.name ?? ''} ${t.target_host}`.toLowerCase().includes(search.toLowerCase())));
  let visibleLogs = $derived(logs.filter(l => logLevel === 'all' || l.level === logLevel).reverse());
  async function refresh() {
    [hosts, tunnels, runtime] = await Promise.all([invoke<Host[]>('list_hosts'), invoke<Tunnel[]>('list_tunnels'), invoke<Snapshot>('get_runtime_snapshot')]);
  }
  onMount(() => {
    const unlisten: UnlistenFn[] = []; let disposed = false;
    async function boot() {
      if (!native) { loading = false; return; }
      try {
        const handlers = await Promise.all([
          listen<Snapshot>('tunnelwarden://runtime', e => runtime = e.payload),
          listen<LogEntry>('tunnelwarden://log', e => logs = [...logs, e.payload].slice(-1000)),
          listen<HostKeyPrompt[]>('tunnelwarden://host-key', e => questions = e.payload),
          listen<KeyPassphrasePrompt[]>('tunnelwarden://key-passphrase', e => keyQuestions = e.payload),
          listen('tunnelwarden://close-requested', () => requestClose()),
        ]);
        if (disposed) { handlers.forEach(f => f()); return; } unlisten.push(...handlers);
        systemLanguage = await invoke<Locale>('get_system_language');
        await refresh();
        [settings, logs, questions, keyQuestions] = await Promise.all([invoke<Settings>('get_settings'), invoke<LogEntry[]>('get_logs'), invoke<HostKeyPrompt[]>('get_host_key_prompts'), invoke<KeyPassphrasePrompt[]>('get_key_passphrase_prompts')]);
        savedSettings = JSON.stringify(settings);
        await invoke('ui_ready');
      } catch (e) { error = errorText(e); } finally { loading = false; }
    }
    void boot(); return () => { disposed = true; unlisten.forEach(f => f()); };
  });
  async function testHost(id: string) {
    hostTests[id] = { running: true, error: false, message: 'Connecting and authenticating…' };
    try {
      const result = await invoke<HostTestResult>('test_host', { hostId: id });
      hostTests[id] = { running: false, error: false, message: `${result.message} (${result.elapsed_ms} ms)` };
    } catch (e) { hostTests[id] = { running: false, error: true, message: errorText(e) }; }
  }
  function requestClose() {
    if (hostEditor || tunnelEditor || deletion || busy || questions.length || keyQuestions.length || Object.values(hostTests).some(t => t.running) || JSON.stringify(settings) !== savedSettings) {
      closeConfirmation = true;
    } else { void closeUi(); }
  }
  async function closeUi() {
    try { await invoke('close_ui'); } catch (e) { error = errorText(e); closeConfirmation = false; }
  }
  async function cancelTest(id: string) {
    try { await invoke('cancel_host_test', { hostId: id }); } catch (e) { error = errorText(e); }
  }
  async function exportLogs() {
    busy = 'export'; error = ''; exportPath = '';
    try { exportPath = await invoke<string | null>('export_error_logs') ?? ''; }
    catch (e) { error = errorText(e); } finally { busy = null; }
  }
  async function action(id: string, command: string) {
    busy = id; error = '';
    try { await invoke(command, { tunnelId: id }); runtime = await invoke<Snapshot>('get_runtime_snapshot'); }
    catch (e) { error = errorText(e); } finally { busy = null; }
  }
  async function saveHost(host: HostInput) { await invoke('save_host', { host }); await refresh(); hostEditor = false; }
  async function saveTunnel(tunnel: Tunnel) { await invoke('save_tunnel', { tunnel }); await refresh(); tunnelEditor = false; }
  async function remove() {
    if (!deletion) return; busy = deletion.id; error = '';
    try { await invoke(deletion.kind === 'host' ? 'delete_host' : 'delete_tunnel', deletion.kind === 'host' ? { hostId: deletion.id } : { tunnelId: deletion.id }); await refresh(); deletion = null; }
    catch (e) { error = errorText(e); } finally { busy = null; }
  }
  async function saveSettings() {
    busy = 'settings'; error = ''; settingsSaved = false;
    try { await invoke('set_settings', { settings }); settingsSaved = true; savedSettings = JSON.stringify(settings); }
    catch (e) { error = errorText(e); } finally { busy = null; }
  }
  async function respond(question: HostKeyPrompt, trust: boolean) {
    busy = question.id; error = '';
    try { await invoke('respond_host_key', { promptId: question.id, trust }); questions = await invoke<HostKeyPrompt[]>('get_host_key_prompts'); }
    catch (e) { error = errorText(e); } finally { busy = null; }
  }
</script>

<div class="shell">
  <aside>
    <div class="brand"><span class="brand-mark" aria-hidden="true">↔</span><span>TunnelWarden<small>{$translateText("SSH tunnel manager")}</small></span></div>
    <nav aria-label={$translateText("Main navigation")}>{#each pages as item}<button aria-label={$translateText(item)} class:active={page === item} aria-current={page === item ? 'page' : undefined} onclick={() => { page = item; detailId = null; error = ''; }}>{$translateText(item)}{#if item === 'Tunnels' && tunnels.length}<span class="count">{tunnels.length}</span>{/if}</button>{/each}</nav>
    <div class="sidebar-bottom"><span class="status-dot" class:live={running > 0}></span>{$translateText("{count} running", { count: running })}<small>v{version}</small></div>
  </aside>
  <main class:detail-main={page === 'Tunnels' && !!selectedTunnel}>
    {#if !selectedTunnel || page !== 'Tunnels'}<header class="page-header"><div><h1>{$translateText(page)}</h1><p>{page === 'Tunnels' ? $translateText("Your connections, kept within reach.") : page === 'SSH Hosts' ? $translateText("Connection details and credentials for your tunnels.") : page === 'Logs' ? $translateText("Recent activity from this session.") : page === 'Settings' ? $translateText("Startup and connection behavior.") : $translateText("A focused utility, powered by Oryxis.")}</p></div>
      {#if page === 'Tunnels'}<button class="primary" disabled={!native || loading || !hosts.length} onclick={() => { editingTunnel = null; tunnelEditor = true; }}>{$translateText("+ New tunnel")}</button>{:else if page === 'SSH Hosts'}<button class="primary" disabled={!native || loading} onclick={() => { editingHost = null; hostEditor = true; }}>{$translateText("+ New host")}</button>{/if}
    </header>{/if}
    {#if !native}<div class="notice">{$translateText("Desktop connection unavailable. Open TunnelWarden to configure and run SSH tunnels.")}</div>{/if}
    {#if error && !deletion && !questions.length}<div class="error-banner" role="alert"><span>{$translateText(error)}</span><button class="icon-button" onclick={() => error = ''} aria-label={$translateText("Dismiss error")}>×</button></div>{/if}
    {#if loading}<div class="empty"><strong>{$translateText("Loading configuration…")}</strong></div>
    {:else if page === 'Tunnels' && selectedTunnel}
      {#key selectedTunnel.id}<TunnelDetails tunnel={selectedTunnel} {hosts} runtimeState={statusById.get(selectedTunnel.id)} {logs} busy={busy === selectedTunnel.id} onback={() => { const id = detailId; detailId = null; requestAnimationFrame(() => document.getElementById(`detail-${id}`)?.focus()); }} onedit={() => { editingTunnel = selectedTunnel ?? null; tunnelEditor = true; }} onaction={command => selectedTunnel && void action(selectedTunnel.id, command)} />{/key}
    {:else if page === 'Tunnels'}
      {#if tunnels.length}<div class="toolbar"><label class="search">{$translateText("Search tunnels")}<input bind:value={search} placeholder={$translateText("Name, SSH host or target")} /></label><span class="muted">{$translateText("{running} running · {saved} saved", { running, saved: tunnels.length })}</span></div>{/if}
      {#if !tunnels.length}<div class="empty"><span class="empty-symbol" aria-hidden="true">↔</span><h2>{$translateText("Your first tunnel starts here")}</h2><p>{hosts.length ? $translateText("Connect a local app to a remote service, expose a local service, or open a SOCKS proxy.") : $translateText("Add an SSH host, then choose where your tunnel listens and where it leads.")}</p>{#if !hosts.length}<button class="primary" disabled={!native} onclick={() => { page = 'SSH Hosts'; editingHost = null; hostEditor = true; }}>{$translateText("Add SSH host")}</button>{/if}</div>
      {:else if !visibleTunnels.length}<div class="empty"><h2>{$translateText("No matching tunnels")}</h2><button onclick={() => search = ''}>{$translateText("Clear search")}</button></div>
      {:else}<div class="tunnel-list">{#each visibleTunnels as t (t.id)}
        {@const host = hosts.find(h => h.id === t.host_id)}{@const state = statusById.get(t.id)}{@const status = state?.status ?? $translateText("stopped")}
        <article class="tunnel-row" aria-label={t.name}>
          <div class="row-heading"><div class="inline"><span class="mode-badge">{t.kind[0]}</span><h2><button id={`detail-${t.id}`} class="tunnel-title" aria-label={$translateText("View details for {name}", { name: t.name })} onclick={() => detailId = t.id}>{t.name}<span aria-hidden="true"> ↗</span></button></h2>{#if t.auto_start}<span class="subtle-tag">{$translateText("Auto-start")}</span>{/if}</div><span class="status {status}"><span class="status-dot"></span>{$translateText(status)}</span></div>
          <div class="route" class:dynamic={t.kind === 'Dynamic'}>
            <div><small>{t.kind === 'Remote' ? $translateText("Remote listener") : t.kind === 'Dynamic' ? $translateText("Local SOCKS5 listener") : $translateText("Local listener")}</small><code>{endpoint(t.listen_address, t.listen_port)}</code></div>
            <span class="route-arrow" aria-hidden="true">{t.kind === 'Remote' ? '←' : '→'}</span>
            <div><small>{$translateText("SSH host")} · {host?.name ?? $translateText("Missing host")}</small><code>{host ? endpoint(host.hostname, host.ssh_port) : $translateText("Not configured")}</code></div>
            {#if t.kind !== 'Dynamic'}<span class="route-arrow" aria-hidden="true">{t.kind === 'Remote' ? '←' : '→'}</span><div><small>{t.kind === 'Remote' ? $translateText("Local target") : $translateText("Remote target")}</small><code>{endpoint(t.target_host, t.target_port)}</code></div>{/if}
          </div>
          {#if t.kind === 'Remote' && status === 'running'}<p class="hint">{$translateText("Forward established · External reachability not verified. The server may restrict the bind address; check from a separate machine.")}</p>{/if}
          {#if state?.last_error}<p class="row-error">{$translateText(state.last_error)}</p>{/if}
          <div class="row-footer"><span class="muted">{status === 'retrying' && state?.next_retry_at ? $translateText("Next retry around {time}", { time: new Date(state.next_retry_at).toLocaleTimeString($locale) }) : host?.jump_chain.length ? $translateText("{count} configured jump hops", { count: host.jump_chain.length }) : t.kind === 'Dynamic' ? $translateText("SOCKS5 · TCP CONNECT") : $translateText("SSH encrypted transport")}</span><div class="actions">
            <button class="text-button" onclick={() => { editingTunnel = t; tunnelEditor = true; }} disabled={busy === t.id}>{$translateText("Edit")}</button>
            <button class="text-button" onclick={() => deletion = { kind: 'tunnel', id: t.id, name: t.name }} disabled={busy === t.id}>{$translateText("Delete")}</button>
            {#if status === 'running'}<button onclick={() => action(t.id, 'restart_tunnel')} disabled={busy === t.id}>{$translateText("Restart")}</button>{/if}
            <button class:primary={status === 'stopped' || status === 'error'} onclick={() => action(t.id, ['running','connecting','retrying'].includes(status) ? 'stop_tunnel' : 'start_tunnel')} disabled={busy === t.id}>{busy === t.id ? $translateText("Working…") : ['running','connecting','retrying'].includes(status) ? $translateText("Stop") : $translateText("Start")}</button>
          </div></div>
        </article>
      {/each}</div>{/if}
    {:else if page === 'SSH Hosts'}
      {#if !hosts.length}<div class="empty"><span class="empty-symbol" aria-hidden="true">⌘</span><h2>{$translateText("No SSH hosts yet")}</h2><p>{$translateText("Save your server address and SSH authentication. The SSH port is independent of your tunnel ports.")}</p></div>
      {:else}<div class="host-list">{#each hosts as host}<article class="host-row"><div class="host-avatar" aria-hidden="true">{host.name.slice(0, 1).toUpperCase()}</div><div class="grow"><h2>{host.name}</h2><code>{host.username}@{endpoint(host.hostname, host.ssh_port)}</code><p class="muted">{host.auth_method === 'Key' ? $translateText("Private key file") : host.auth_method === 'Agent' ? $translateText("SSH agent") : host.auth_method === 'Auto' ? $translateText("Automatic authentication") : $translateText("Password")} · {host.jump_chain.length ? $translateText("Via {route}", { route: host.jump_chain.map(id => hosts.find(h => h.id === id)?.name ?? $translateText("Missing host")).join(" → ") }) : $translateText("Direct connection")}</p>{#if hostTests[host.id]}<p class:row-error={hostTests[host.id].error} class="hint" role="status">{$translateText(hostTests[host.id].message)}</p>{/if}</div>{#if hostTests[host.id]?.running}<button onclick={() => cancelTest(host.id)}>{$translateText("Cancel test")}</button>{:else}<button onclick={() => testHost(host.id)}>{$translateText("Test connection")}</button>{/if}<button disabled={hostTests[host.id]?.running} onclick={() => { editingHost = host; hostEditor = true; }}>{$translateText("Edit")}</button><button disabled={hostTests[host.id]?.running} class="text-button" onclick={() => deletion = { kind: 'host', id: host.id, name: host.name }}>{$translateText("Delete")}</button></article>{/each}</div>{/if}
    {:else if page === 'Logs'}
      <div class="toolbar"><label>{$translateText("Level")}<select bind:value={logLevel}><option value="all">{$translateText("All activity")}</option><option value="info">{$translateText("Info")}</option><option value="error">{$translateText("Errors")}</option></select></label><span class="muted">{$translateText("{count} / 1,000 retained", { count: logs.length })}</span><button disabled={!native || busy === 'export'} onclick={exportLogs}>{busy === 'export' ? $translateText("Exporting…") : $translateText("Export error logs")}</button></div>
      <p class="hint">{$translateText("Export includes errors, recent activity and runtime status from this app session. Credentials and private-key contents are excluded; server addresses and error details may remain. Review before sharing.")}</p>
      {#if exportPath}<p class="success" role="status">{$translateText("Report saved to {path}", { path: exportPath })}</p>{/if}
      {#if !visibleLogs.length}<div class="empty"><h2>{$translateText("No activity to show")}</h2><p>{$translateText("Tunnel state changes and errors appear here.")}</p></div>{:else}<div class="log-list" aria-live="polite">{#each visibleLogs as entry (entry.id)}<div class="log-row"><time datetime={entry.timestamp}>{new Date(entry.timestamp).toLocaleTimeString($locale)}</time><span class="log-level" class:error-level={entry.level === 'error'}>{$translateText(entry.level)}</span><div>{#if entry.tunnel_id}<strong>{tunnels.find(t => t.id === entry.tunnel_id)?.name ?? $translateText("Deleted tunnel")} · </strong>{/if}{$translateText(entry.message)}</div></div>{/each}</div>{/if}
    {:else if page === 'Settings'}
      <form class="settings" onsubmit={e => { e.preventDefault(); void saveSettings(); }}>
        <section class="settings-section"><h2>{$translateText("Language")}</h2><label>{$translateText("Display language")}<select bind:value={settings.language} onchange={() => settingsSaved = false}><option value="system">{$translateText("Follow system")}</option><option value="zh-CN">简体中文</option><option value="en">English</option></select><small>{$translateText("Chinese systems use Chinese; all other system languages use English.")}</small></label></section>
        <section class="settings-section"><h2>{$translateText("Startup & window")}</h2><label class="check"><input type="checkbox" bind:checked={settings.launch_at_login} onchange={() => settingsSaved = false} /><span>{$translateText("Launch at Windows login")}<small>{$translateText("Starts the Rust runtime and tray without opening the interface.")}</small></span></label><label class="check"><input type="checkbox" bind:checked={settings.close_to_tray} onchange={() => settingsSaved = false} /><span>{$translateText("Keep running when the window closes")}<small>{$translateText("Releases the interface and its WebView. The tray can recreate it or quit and stop tunnels.")}</small></span></label></section>
        <section class="settings-section"><h2>{$translateText("Recovery")}</h2><label class="check"><input type="checkbox" bind:checked={settings.auto_reconnect} onchange={() => settingsSaved = false} /><span>{$translateText("Reconnect after a connection drops")}<small>{$translateText("Retries at 15, 30, 60, then 120 seconds. Manually stopping a tunnel cancels retry.")}</small></span></label><p class="hint">{$translateText("Auto-start is set per tunnel. Enabled tunnels start when the app process starts and retry failed starts independently of this setting.")}</p></section>
        <footer><span class="success" role="status">{settingsSaved ? $translateText("Settings saved") : ''}</span><button class="primary" disabled={!native || busy === 'settings'}>{busy === 'settings' ? $translateText("Saving…") : $translateText("Save settings")}</button></footer>
      </form>
    {:else}
      <div class="about"><div class="about-title"><span class="brand-mark">↔</span><div><h2>TunnelWarden <span class="muted">{version}</span></h2><p>{$translateText("Persistent SSH tunnels. A small, dedicated desktop utility.")}</p></div></div><p>{$translateText("SSH transport, authentication, jump routes and forwarding are provided by Oryxis. The standalone forwarding runtime is adapted from its application.")}</p><dl><dt>{$translateText("Source & releases")}</dt><dd><a href="https://github.com/wadaxiyang/TunnelWarden" target="_blank" rel="noreferrer" onclick={e => { if (native) { e.preventDefault(); void invoke('open_source').catch(e => error = errorText(e)); } }}>github.com/wadaxiyang/TunnelWarden</a></dd><dt>{$translateText("Oryxis upstream")}</dt><dd><code class="fingerprint">16ad2af83b424780ff60b6420a6c20a3b30c7143</code></dd><dt>{$translateText("License")}</dt><dd>GNU AGPL-3.0-or-later</dd></dl><details><summary>{$translateText("Oryxis attribution")}</summary><pre>{notice}</pre></details><details><summary>{$translateText("GNU Affero General Public License")}</summary><pre>{license}</pre></details></div>
    {/if}
  </main>
</div>

{#if tunnelEditor}<TunnelEditor tunnel={editingTunnel} {hosts} onsave={saveTunnel} onclose={() => tunnelEditor = false} />{/if}
{#if hostEditor}<HostEditor host={editingHost} {hosts} onsave={saveHost} onclose={() => hostEditor = false} />{/if}
{#if deletion}<Modal title={$translateText(deletion.kind === "tunnel" ? "Delete tunnel?" : "Delete host?")} onclose={() => { deletion = null; error = ''; }}><p>{$translateText("Delete {name}?", { name: deletion.name })} {deletion.kind === 'tunnel' ? $translateText("Its active listener and connections will be stopped before removal.") : $translateText("Its saved password will also be removed.")}</p>{#if error}<p class="error" role="alert">{$translateText(error)}</p>{/if}<footer><button onclick={() => deletion = null}>{$translateText("Cancel")}</button><button class="danger" disabled={busy === deletion.id} onclick={remove}>{busy === deletion.id ? $translateText("Deleting…") : $translateText("Delete")}</button></footer></Modal>{/if}
{#if questions.length}{@const q = questions[0]}<Modal title={q.old_fingerprint ? $translateText("SSH host key has changed") : $translateText("Verify SSH host key")} onclose={() => void respond(q, false)}><p>{q.old_fingerprint ? $translateText("The server presented a different host key. Confirm the change with the server administrator before continuing.") : $translateText("Verify this fingerprint with the server administrator or another trusted source before connecting.")}</p><dl><dt>{$translateText("SSH server")}</dt><dd>{endpoint(q.hostname, q.port)}</dd><dt>{$translateText("Algorithm")}</dt><dd>{q.key_type}</dd><dt>{$translateText("Fingerprint")}</dt><dd><code class="fingerprint">{q.fingerprint}</code></dd>{#if q.old_fingerprint}<dt>{$translateText("Previously trusted")}</dt><dd><code class="fingerprint">{q.old_fingerprint}</code></dd>{/if}</dl>{#if error}<p class="error" role="alert">{$translateText(error)}</p>{/if}<footer><button disabled={busy === q.id} onclick={() => respond(q, false)}>{$translateText("Reject")}</button><button class:danger={!!q.old_fingerprint} class:primary={!q.old_fingerprint} disabled={busy === q.id} onclick={() => respond(q, true)}>{$translateText("Trust and connect")}</button></footer></Modal>{/if}

{#if keyQuestions.length}{#key keyQuestions[0].id}<KeyUnlock question={keyQuestions[0]} />{/key}{/if}
{#if closeConfirmation}<Modal title={$translateText("Close the interface?")} onclose={() => closeConfirmation = false}><p>{$translateText("Unsaved edits will be discarded. Connection tests and unanswered authentication questions will be cancelled.")}</p><p>{(savedSettings && JSON.parse(savedSettings).close_to_tray) ? $translateText("Running tunnels continue in the background. Open the tray icon to create the interface again.") : $translateText("The app will quit and stop tunnels, according to your saved setting.")}</p><footer><button onclick={() => closeConfirmation = false}>{$translateText("Keep editing")}</button><button class="primary" onclick={closeUi}>{$translateText("Close interface")}</button></footer></Modal>{/if}
