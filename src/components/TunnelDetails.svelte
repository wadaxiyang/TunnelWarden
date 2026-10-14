<script lang="ts">
  import { t as translateText, locale } from '../i18n';
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
        points = appendPoint(points, { at: performance.now(), time: new Date().toLocaleTimeString($locale), rate });
      } catch (e) {
        if (!disposed) { sampleError = errorText(e); sample = null; rate = null; previous = null; points = appendPoint(points, { at: performance.now(), time: new Date().toLocaleTimeString($locale), rate: null }); }
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
  <button class="text-button back-button" onclick={onback}>{$translateText("← All tunnels")}</button>
  <header class="detail-header">
    <div class="grow"><div class="inline"><span class="mode-badge">{tunnel.kind[0]}</span><h1 bind:this={heading} tabindex="-1" title={tunnel.name}>{tunnel.name}</h1></div><p class="hint">{$translateText(`${tunnel.kind} forwarding`)} · {host?.name ?? $translateText("Missing SSH host")}</p></div>
    <span class="status {status}"><span class="status-dot"></span>{$translateText(status)}</span>
    <div class="actions"><button onclick={onedit} disabled={busy}>{$translateText("Edit")}</button>{#if status === 'running'}<button onclick={() => onaction('restart_tunnel')} disabled={busy}>{$translateText("Restart")}</button>{/if}<button class:primary={['stopped','error'].includes(status)} disabled={busy} onclick={() => onaction(['running','connecting','retrying'].includes(status) ? 'stop_tunnel' : 'start_tunnel')}>{busy ? $translateText("Working…") : ['running','connecting','retrying'].includes(status) ? $translateText("Stop") : $translateText("Start")}</button></div>
  </header>
  <div class="detail-tabs" role="tablist" aria-label={$translateText("Tunnel details")}>
    {#each tabs as tab}<button id={`detail-tab-${tab}`} role="tab" aria-label={$translateText(tab)} aria-selected={activeTab === tab} aria-controls={`detail-panel-${tab}`} tabindex={activeTab === tab ? 0 : -1} onclick={() => activeTab = tab} onkeydown={tabKey}>{$translateText(tab)}{#if tab === 'History' && activity.some(e => e.level === 'error')}<span class="tab-error" title={$translateText("Contains errors")} aria-hidden="true">•</span>{/if}</button>{/each}
  </div>
  {#each tabs as tab}
    <div id={`detail-panel-${tab}`} class="detail-tabpanel" class:traffic-panel={tab === 'Traffic'} role="tabpanel" aria-labelledby={`detail-tab-${tab}`} tabindex="0" hidden={activeTab !== tab}>
      {#if tab === 'Overview'}
{#if runtimeState?.last_error}<div class="error-banner" role="alert">{$translateText(runtimeState.last_error)}</div>{/if}
  {#if runtimeState?.next_retry_at && status === 'retrying'}<p class="notice">{$translateText("Next retry around {time}. Stop cancels the retry.", { time: new Date(runtimeState.next_retry_at).toLocaleTimeString($locale) })}</p>{/if}
  {#if tunnel.kind === 'Remote'}<p class="notice">{status === 'running' ? $translateText("Forward established.") + " " : ''}{$translateText("External reachability is not verified. The server controls remote binding; GatewayPorts, forwarding policy and firewalls can restrict access.")}</p>{/if}
<section class="detail-section" aria-labelledby="config-heading"><h2 id="config-heading">{$translateText("Configuration")}</h2><dl>
    <dt>{tunnel.kind === 'Remote' ? $translateText("Remote listener") : tunnel.kind === 'Dynamic' ? $translateText("Local SOCKS5 listener") : $translateText("Local listener")}</dt><dd><code>{endpoint(tunnel.listen_address, tunnel.listen_port)}</code></dd>
    <dt>{$translateText("SSH host")}</dt><dd>{host ? `${host.username}@${endpoint(host.hostname, host.ssh_port)}` : $translateText("Missing host")}</dd>
    {#if tunnel.kind !== 'Dynamic'}<dt>{tunnel.kind === 'Remote' ? $translateText("Local target") : $translateText("Remote target")}</dt><dd><code>{endpoint(tunnel.target_host, tunnel.target_port)}</code></dd>{/if}
    <dt>{$translateText("Configured jump route")}</dt><dd>{host?.jump_chain.length ? host.jump_chain.map(id => hosts.find(h => h.id === id)?.name ?? $translateText("Missing host")).join(' → ') : $translateText("Direct connection")}</dd>
    <dt>{$translateText("Auto-start")}</dt><dd>{tunnel.auto_start ? $translateText("Enabled at app startup") : $translateText("Disabled")}</dd>
  </dl></section>
      {:else if tab === 'Traffic'}
<section class="detail-section" aria-labelledby="traffic-heading">
    <div class="section-heading"><h2 id="traffic-heading">{$translateText("Live traffic")}</h2><span class="muted">{paused ? $translateText("Paused while hidden") : sampleError ? $translateText("Sample unavailable") : !ready ? $translateText("Loading…") : status !== 'running' ? $translateText("No active forward") : !rate ? $translateText("Taking first sample…") : $translateText("Updates every second")}</span></div>
    {#if sampleError}<p class="row-error" role="alert">{$translateText("Could not read traffic: {$translateText(error)}", { error: $translateText(sampleError) })}</p>{/if}
    <div class="traffic-metrics">
      <div><span class="metric-label upload">{$translateText("↑ Upload")}</span><strong data-metric="upload" title={currentRate ? $translateText("{count} bytes/s", { count: currentRate.up.toFixed(2) }) : undefined}>{formatRate(currentRate?.up)}</strong><small>{$translateText("This PC → SSH")}</small></div>
      <div><span class="metric-label download">{$translateText("↓ Download")}</span><strong data-metric="download" title={currentRate ? $translateText("{count} bytes/s", { count: currentRate.down.toFixed(2) }) : undefined}>{formatRate(currentRate?.down)}</strong><small>{$translateText("SSH → this PC")}</small></div>
      <div><span class="metric-label" title={$translateText("Bytes sent in the current connection")}>{$translateText("Sent total")}</span><strong data-metric="sent" title={current ? $translateText("{count} bytes", { count: current.sent_bytes }) : undefined}>{current ? formatBytes(current.sent_bytes) : '—'}</strong><small>{current ? $translateText("{count} bytes", { count: current.sent_bytes }) : $translateText("Resets on reconnect")}</small></div>
      <div><span class="metric-label" title={$translateText("Bytes received in the current connection")}>{$translateText("Received total")}</span><strong data-metric="received" title={current ? $translateText("{count} bytes", { count: current.received_bytes }) : undefined}>{current ? formatBytes(current.received_bytes) : '—'}</strong><small>{current ? $translateText("{count} bytes", { count: current.received_bytes }) : $translateText("Resets on reconnect")}</small></div>
    </div>
    <div class="chart-toolbar"><div class="chart-legend"><span class="upload">{$translateText("━ Upload")}</span><span class="download">{$translateText("┄ Download")}</span></div><label>{$translateText("Time window")}<select bind:value={windowSeconds}><option value={60}>{$translateText("Last minute")}</option><option value={300}>{$translateText("Last 5 minutes")}</option></select></label></div>
    <svg class="traffic-chart" viewBox="0 0 720 210" preserveAspectRatio="none" role="img" aria-labelledby="chart-title chart-description">
      <title id="chart-title">{$translateText("Upload and download rate over time")}</title><desc id="chart-description">{measured ? $translateText("Two payload rate lines. Missing samples and new connections appear as gaps. Exact recent values are available using View exact samples.") : $translateText("Waiting for two samples from an active forward. No traffic has been invented for the period before opening this page.")}</desc>
      {#each [0, 0.5, 1] as fraction}<line x1="90" x2="700" y1={174 - fraction * 150} y2={174 - fraction * 150} class="chart-grid"/><text x="82" y={178 - fraction * 150} text-anchor="end">{formatBytes(scale * fraction)}</text>{/each}
      <text x="90" y="202">−{windowSeconds}s</text><text x="380" y="202" text-anchor="middle">−{windowSeconds / 2}s</text><text x="700" y="202" text-anchor="end">{$translateText("Latest sample")}</text>
      <path data-series="upload" d={upPath} class="upload-line"/><path data-series="download" d={downPath} class="download-line"/>
      {#if !measured}<text x="380" y="95" text-anchor="middle">{status === 'running' ? $translateText("Waiting for rate samples…") : $translateText("Start this tunnel to measure traffic")}</text>{/if}
    </svg>

    <div class="traffic-caption"><span class="muted">{$translateText("Payload only · resets on reconnect")}{#if current} · {$translateText("{seconds} s connected", { seconds: Math.floor(current.elapsed_ms / 1000).toLocaleString($locale) })}{/if}</span><button class="text-button" onclick={() => showSamples = true}>{$translateText("View exact samples")}</button></div>

  </section>
      {:else}
<section class="detail-section" aria-labelledby="history-heading"><div class="section-heading"><h2 id="history-heading">{$translateText("State & activity history")}</h2><label>{$translateText("Show")}<select bind:value={historyLevel} onchange={() => historyPage = 0}><option value="all">{$translateText("All activity")}</option><option value="error">{$translateText("Errors only")}</option></select></label></div>
    <p class="hint">{$translateText("This app session only · drawn from the latest 1,000 application log entries. Reopening the interface retains activity; quitting the app clears it. Newest first.")}</p>
    {#if !filteredActivity.length}<p class="history-empty">{historyLevel === 'error' ? $translateText("No retained errors for this tunnel.") : $translateText("No retained activity for this tunnel yet.")}</p>{:else}<ol class="activity-timeline">{#each filteredActivity.slice(historyPage * historySize, (historyPage + 1) * historySize) as entry (entry.id)}<li><time datetime={entry.timestamp}>{new Date(entry.timestamp).toLocaleString($locale)}</time><span class="log-level" class:error-level={entry.level === 'error'}>{$translateText(entry.level)}</span><p>{$translateText(entry.message)}</p></li>{/each}</ol><div class="history-pagination"><button disabled={historyPage === 0} onclick={() => historyPage--}>{$translateText("Newer")}</button><span class="muted">{$translateText("Page {page} of {pages}", { page: historyPage + 1, pages: Math.max(1, Math.ceil(filteredActivity.length / historySize)) })}</span><button disabled={(historyPage + 1) * historySize >= filteredActivity.length} onclick={() => historyPage++}>{$translateText("Older")}</button></div>{/if}
  </section>
      {/if}
    </div>
  {/each}
</div>
{#if showSamples}<Modal title={$translateText("Recent traffic samples")} onclose={() => showSamples = false}>
<p class="hint">{$translateText("Payload bytes per second; protocol overhead excluded. Collected only while this detail page is visible, up to 5 minutes. Gaps mean no measurement. Counters reset on reconnect; they do not prove final delivery or public reachability.")}{#if current}{' '}{$translateText("Current connection age: {seconds} s.", { seconds: Math.floor(current.elapsed_ms / 1000).toLocaleString($locale) })}{/if}</p>
<div class="sample-table"><table><caption>{$translateText("Latest 20 measurements; — means unavailable")}</caption><thead><tr><th scope="col">{$translateText("Time")}</th><th scope="col">{$translateText("Upload B/s")}</th><th scope="col">{$translateText("Download B/s")}</th></tr></thead><tbody>{#each points.slice(-20).reverse() as p}<tr><td>{p.time}</td><td>{p.rate?.up.toFixed(2) ?? '—'}</td><td>{p.rate?.down.toFixed(2) ?? '—'}</td></tr>{/each}</tbody></table></div>
  <footer><button onclick={() => showSamples = false}>{$translateText("Close samples")}</button></footer>
</Modal>{/if}
