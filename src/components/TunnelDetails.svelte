<script lang="ts">
  import { onMount } from 'svelte';
  import Modal from './Modal.svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { endpoint, errorText, type Host, type Tunnel, type RuntimeTunnel, type LogEntry, type TrafficSample } from '../types';
  import { appendPoint, rateBetween, formatBytes, formatRate, linePath, type Rate, type TrafficPoint } from '../traffic';

  let { tunnel, hosts, runtimeState, logs, busy, onback, onedit, onaction }: {
    tunnel: Tunnel; hosts: Host[]; runtimeState: RuntimeTunnel | undefined; logs: LogEntry[];
    busy: boolean; onback: () => void; onedit: () => void; onaction: (command: string) => void;
  } = $props();
  let host = $derived(hosts.find(h => h.id === tunnel.host_id));
  let status = $derived(runtimeState?.status ?? 'stopped');
  let activity = $derived(logs.filter(e => e.tunnel_id === tunnel.id).reverse());
  let sample = $state<TrafficSample | null>(null);
  let rate = $state<Rate | null>(null);
  let points = $state<TrafficPoint[]>([]);
  let sampleError = $state(''); let paused = $state(false); let ready = $state(false);
  const tabs = ['Overview', 'Traffic', 'History'] as const;
  let activeTab = $state<(typeof tabs)[number]>('Overview');
  let showSamples = $state(false);
  let windowSeconds = $state(60); let historyLevel = $state('all'); let historyPage = $state(0);
  const historySize = 10;
  function tabKey(event: KeyboardEvent) {
    const index = tabs.indexOf(activeTab);
    const next = event.key === 'ArrowRight' ? (index + 1) % tabs.length : event.key === 'ArrowLeft' ? (index + tabs.length - 1) % tabs.length : event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : -1;
    if (next < 0) return;
    event.preventDefault(); activeTab = tabs[next];
    document.getElementById(`detail-tab-${activeTab}`)?.focus();
  }
  let filteredActivity = $derived(activity.filter(e => historyLevel === 'all' || e.level === 'error'));
  $effect(() => { historyPage = Math.min(historyPage, Math.max(0, Math.ceil(filteredActivity.length / historySize) - 1)); });
  let latest = $derived(points.at(-1)?.at ?? 0);
  let chartPoints = $derived(points.filter(p => p.at >= latest - windowSeconds * 1000));
  let scale = $derived(Math.max(1, ...chartPoints.flatMap(p => p.rate ? [p.rate.up, p.rate.down] : [])) * 1.1);
  let start = $derived(latest - windowSeconds * 1000);
  let upPath = $derived(linePath(chartPoints, 'up', start, windowSeconds * 1000, scale));
  let downPath = $derived(linePath(chartPoints, 'down', start, windowSeconds * 1000, scale));
  let current = $derived(status === 'running' && !sampleError && !paused ? sample : null);
  let currentRate = $derived(current ? rate : null);
  let measured = $derived(chartPoints.some(p => p.rate !== null));
  let heading: HTMLHeadingElement;

  onMount(() => {
    heading.focus();
    let disposed = false; let pending = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let previous: TrafficSample | null = null;
    async function poll() {
      if (disposed || pending || document.hidden) return;
      pending = true;
      try {
        const next = await invoke<TrafficSample | null>('get_tunnel_traffic', { tunnelId: tunnel.id });
        if (disposed || document.hidden) return;
        rate = next ? rateBetween(previous, next) : null;
        sample = next; previous = next; sampleError = ''; ready = true;
        // No data is different from zero. Gaps/reconnects break the SVG paths.
        points = appendPoint(points, { at: performance.now(), time: new Date().toLocaleTimeString(), rate });
      } catch (e) {
        if (!disposed) { sampleError = errorText(e); sample = null; rate = null; previous = null; points = appendPoint(points, { at: performance.now(), time: new Date().toLocaleTimeString(), rate: null }); }
      } finally {
        pending = false;
        if (!disposed && !document.hidden) timer = setTimeout(poll, 1000);
      }
    }
    function visibility() {
      clearTimeout(timer); paused = document.hidden; previous = null; rate = null;
      if (!paused) void poll();
    }
    document.addEventListener('visibilitychange', visibility);
    visibility();
    return () => { disposed = true; clearTimeout(timer); document.removeEventListener('visibilitychange', visibility); };
  });
</script>

<div class="tunnel-details">
  <button class="text-button back-button" onclick={onback}>← All tunnels</button>
  <header class="detail-header">
    <div class="grow"><div class="inline"><span class="mode-badge">{tunnel.kind[0]}</span><h1 bind:this={heading} tabindex="-1" title={tunnel.name}>{tunnel.name}</h1></div><p class="hint">{tunnel.kind} forwarding · {host?.name ?? 'Missing SSH host'}</p></div>
    <span class="status {status}"><span class="status-dot"></span>{status}</span>
    <div class="actions"><button onclick={onedit} disabled={busy}>Edit</button>{#if status === 'running'}<button onclick={() => onaction('restart_tunnel')} disabled={busy}>Restart</button>{/if}<button class:primary={['stopped','error'].includes(status)} disabled={busy} onclick={() => onaction(['running','connecting','retrying'].includes(status) ? 'stop_tunnel' : 'start_tunnel')}>{busy ? 'Working…' : ['running','connecting','retrying'].includes(status) ? 'Stop' : 'Start'}</button></div>
  </header>
  <div class="detail-tabs" role="tablist" aria-label="Tunnel details">
    {#each tabs as tab}<button id={`detail-tab-${tab}`} role="tab" aria-label={tab} aria-selected={activeTab === tab} aria-controls={`detail-panel-${tab}`} tabindex={activeTab === tab ? 0 : -1} onclick={() => activeTab = tab} onkeydown={tabKey}>{tab}{#if tab === 'History' && activity.some(e => e.level === 'error')}<span class="tab-error" title="Contains errors" aria-hidden="true">•</span>{/if}</button>{/each}
  </div>
  {#each tabs as tab}
    <div id={`detail-panel-${tab}`} class="detail-tabpanel" class:traffic-panel={tab === 'Traffic'} role="tabpanel" aria-labelledby={`detail-tab-${tab}`} tabindex="0" hidden={activeTab !== tab}>
      {#if tab === 'Overview'}
{#if runtimeState?.last_error}<div class="error-banner" role="alert">{runtimeState.last_error}</div>{/if}
  {#if runtimeState?.next_retry_at && status === 'retrying'}<p class="notice">Next retry around {new Date(runtimeState.next_retry_at).toLocaleTimeString()}. Stop cancels the retry.</p>{/if}
  {#if tunnel.kind === 'Remote'}<p class="notice">{status === 'running' ? 'Forward established. ' : ''}External reachability is not verified. The server controls remote binding; GatewayPorts, forwarding policy and firewalls can restrict access.</p>{/if}
<section class="detail-section" aria-labelledby="config-heading"><h2 id="config-heading">Configuration</h2><dl>
    <dt>{tunnel.kind === 'Remote' ? 'Remote listener' : tunnel.kind === 'Dynamic' ? 'Local SOCKS5 listener' : 'Local listener'}</dt><dd><code>{endpoint(tunnel.listen_address, tunnel.listen_port)}</code></dd>
    <dt>SSH host</dt><dd>{host ? `${host.username}@${endpoint(host.hostname, host.ssh_port)}` : 'Missing host'}</dd>
    {#if tunnel.kind !== 'Dynamic'}<dt>{tunnel.kind === 'Remote' ? 'Local target' : 'Remote target'}</dt><dd><code>{endpoint(tunnel.target_host, tunnel.target_port)}</code></dd>{/if}
    <dt>Configured jump route</dt><dd>{host?.jump_chain.length ? host.jump_chain.map(id => hosts.find(h => h.id === id)?.name ?? 'Missing host').join(' → ') : 'Direct connection'}</dd>
    <dt>Auto-start</dt><dd>{tunnel.auto_start ? 'Enabled at app startup' : 'Disabled'}</dd>
  </dl></section>
      {:else if tab === 'Traffic'}
<section class="detail-section" aria-labelledby="traffic-heading">
    <div class="section-heading"><h2 id="traffic-heading">Live traffic</h2><span class="muted">{paused ? 'Paused while hidden' : sampleError ? 'Sample unavailable' : !ready ? 'Loading…' : status !== 'running' ? 'No active forward' : !rate ? 'Taking first sample…' : 'Updates every second'}</span></div>
    {#if sampleError}<p class="row-error" role="alert">Could not read traffic: {sampleError}</p>{/if}
    <div class="traffic-metrics">
      <div><span class="metric-label upload">↑ Upload</span><strong data-metric="upload" title={currentRate ? `${currentRate.up.toFixed(2)} bytes/s` : undefined}>{formatRate(currentRate?.up)}</strong><small>This PC → SSH</small></div>
      <div><span class="metric-label download">↓ Download</span><strong data-metric="download" title={currentRate ? `${currentRate.down.toFixed(2)} bytes/s` : undefined}>{formatRate(currentRate?.down)}</strong><small>SSH → this PC</small></div>
      <div><span class="metric-label" title="Bytes sent in the current connection">Sent total</span><strong data-metric="sent" title={current ? `${current.sent_bytes} bytes` : undefined}>{current ? formatBytes(current.sent_bytes) : '—'}</strong><small>{current ? `${current.sent_bytes} bytes` : 'Resets on reconnect'}</small></div>
      <div><span class="metric-label" title="Bytes received in the current connection">Received total</span><strong data-metric="received" title={current ? `${current.received_bytes} bytes` : undefined}>{current ? formatBytes(current.received_bytes) : '—'}</strong><small>{current ? `${current.received_bytes} bytes` : 'Resets on reconnect'}</small></div>
    </div>
    <div class="chart-toolbar"><div class="chart-legend"><span class="upload">━ Upload</span><span class="download">┄ Download</span></div><label>Time window<select bind:value={windowSeconds}><option value={60}>Last minute</option><option value={300}>Last 5 minutes</option></select></label></div>
    <svg class="traffic-chart" viewBox="0 0 720 210" preserveAspectRatio="none" role="img" aria-labelledby="chart-title chart-description">
      <title id="chart-title">Upload and download rate over time</title><desc id="chart-description">{measured ? 'Two payload rate lines. Missing samples and new connections appear as gaps. Exact recent values are available using View exact samples.' : 'Waiting for two samples from an active forward. No traffic has been invented for the period before opening this page.'}</desc>
      {#each [0, 0.5, 1] as fraction}<line x1="90" x2="700" y1={174 - fraction * 150} y2={174 - fraction * 150} class="chart-grid"/><text x="82" y={178 - fraction * 150} text-anchor="end">{formatBytes(scale * fraction)}</text>{/each}
      <text x="90" y="202">−{windowSeconds}s</text><text x="380" y="202" text-anchor="middle">−{windowSeconds / 2}s</text><text x="700" y="202" text-anchor="end">Latest sample</text>
      <path data-series="upload" d={upPath} class="upload-line"/><path data-series="download" d={downPath} class="download-line"/>
      {#if !measured}<text x="380" y="95" text-anchor="middle">{status === 'running' ? 'Waiting for rate samples…' : 'Start this tunnel to measure traffic'}</text>{/if}
    </svg>

    <div class="traffic-caption"><span class="muted">Payload only · resets on reconnect{#if current} · {Math.floor(current.elapsed_ms / 1000).toLocaleString()} s connected{/if}</span><button class="text-button" onclick={() => showSamples = true}>View exact samples</button></div>

  </section>
      {:else}
<section class="detail-section" aria-labelledby="history-heading"><div class="section-heading"><h2 id="history-heading">State & activity history</h2><label>Show<select bind:value={historyLevel} onchange={() => historyPage = 0}><option value="all">All activity</option><option value="error">Errors only</option></select></label></div>
    <p class="hint">This app session only · drawn from the latest 1,000 application log entries. Reopening the interface retains activity; quitting the app clears it. Newest first.</p>
    {#if !filteredActivity.length}<p class="history-empty">{historyLevel === 'error' ? 'No retained errors for this tunnel.' : 'No retained activity for this tunnel yet.'}</p>{:else}<ol class="activity-timeline">{#each filteredActivity.slice(historyPage * historySize, (historyPage + 1) * historySize) as entry (entry.id)}<li><time datetime={entry.timestamp}>{new Date(entry.timestamp).toLocaleString()}</time><span class="log-level" class:error-level={entry.level === 'error'}>{entry.level}</span><p>{entry.message}</p></li>{/each}</ol><div class="history-pagination"><button disabled={historyPage === 0} onclick={() => historyPage--}>Newer</button><span class="muted">Page {historyPage + 1} of {Math.max(1, Math.ceil(filteredActivity.length / historySize))}</span><button disabled={(historyPage + 1) * historySize >= filteredActivity.length} onclick={() => historyPage++}>Older</button></div>{/if}
  </section>
      {/if}
    </div>
  {/each}
</div>
{#if showSamples}<Modal title="Recent traffic samples" onclose={() => showSamples = false}>
<p class="hint">Payload bytes per second; protocol overhead excluded. Collected only while this detail page is visible, up to 5 minutes. Gaps mean no measurement. Counters reset on reconnect; they do not prove final delivery or public reachability.{#if current}{' '}Current connection age: {Math.floor(current.elapsed_ms / 1000).toLocaleString()} s.{/if}</p>
<div class="sample-table"><table><caption>Latest 20 measurements; — means unavailable</caption><thead><tr><th scope="col">Time</th><th scope="col">Upload B/s</th><th scope="col">Download B/s</th></tr></thead><tbody>{#each points.slice(-20).reverse() as p}<tr><td>{p.time}</td><td>{p.rate?.up.toFixed(2) ?? '—'}</td><td>{p.rate?.down.toFixed(2) ?? '—'}</td></tr>{/each}</tbody></table></div>
  <footer><button onclick={() => showSamples = false}>Close samples</button></footer>
</Modal>{/if}
