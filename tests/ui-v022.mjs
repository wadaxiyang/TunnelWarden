// Run after ui-v02.mjs against the isolated real Tauri application. Uses an
// actual loopback echo target through Oryxis; never substitutes an IPC backend.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createServer, createConnection } from 'node:net';
const binary = process.env.TUNNELWARDEN_AGENT_BROWSER ?? resolve('.upstream/tooling/node_modules/agent-browser/bin/agent-browser-win32-x64.exe');
function run(...command) {
  const r = spawnSync(binary, ['--session', 'tunnelwarden-native', '--cdp', '9223', ...command], { encoding: 'utf8', timeout: 30000, windowsHide: true });
  assert.equal(r.status, 0, r.stderr || r.stdout); assert(!/^✗/m.test(r.stdout), r.stdout); return r.stdout;
}
const snapshot = () => run('snapshot', '-i');
const click = name => { snapshot(); return run('find', 'role', 'button', 'click', '--name', name, '--exact'); };
const body = () => run('get', 'text', 'body');
const tab = name => { snapshot(); return run('find', 'role', 'tab', 'click', '--name', name, '--exact'); };
const wait = ms => new Promise(r => setTimeout(r, ms));
async function waitText(text) {
  const until = Date.now() + 15000;
  while (!body().includes(text)) { assert(Date.now() < until, `Missing: ${text}`); await wait(150); }
}
function evaluate(script) { return JSON.parse(run('eval', script)); }
const saved = JSON.parse(readFileSync('.upstream/ui-v02-state.json', 'utf8'));
const echo = createServer(socket => { socket.on('error', () => {}); socket.pipe(socket); });
await new Promise(r => echo.listen(0, '127.0.0.1', r));
const echoPort = echo.address().port;
let socket;
try {
  const original = evaluate(`(async()=> (await window.__TAURI_INTERNALS__.invoke('list_tunnels')).find(t=>t.name===${JSON.stringify(saved.name)}))()`);
  assert(original);
  original.target_port = JSON.parse(readFileSync(".upstream/ui-fixture.json", "utf8").replace(/^\uFEFF/, "")).port;
  const adapted = { ...original, target_port: echoPort };
  evaluate(`(async()=>{const i=window.__TAURI_INTERNALS__.invoke; await i('save_tunnel',{tunnel:${JSON.stringify(adapted)}}); await i('start_tunnel',{tunnelId:${JSON.stringify(original.id)}}); return true;})()`);
  run('reload'); await waitText(saved.name);
  evaluate(`(()=>{const original=window.__TAURI_INTERNALS__.invoke; window.detailTestCalls=0; window.__TAURI_INTERNALS__.invoke=(cmd,...args)=>{if(cmd==='get_tunnel_traffic')window.detailTestCalls++;return original(cmd,...args)};return true;})()`);
  click(`View details for ${saved.name}`); await waitText('Configuration');
  assert(!body().includes('Live traffic'));
  assert(!body().includes('State & activity history'));
  assert(body().includes(`127.0.0.1:${echoPort}`));
  assert.equal(evaluate('document.activeElement.tagName'), 'H1');
  tab('Overview'); run('press', 'ArrowRight');
  assert.equal(evaluate('document.activeElement.getAttribute("aria-selected")'), 'true');
  assert.equal(evaluate('document.activeElement.textContent.trim()'), 'Traffic');
  await waitText('Live traffic');
  assert(!body().includes('Configuration'));
  assert.equal(evaluate('document.documentElement.scrollHeight > innerHeight'), false);
  run('press', 'End'); assert(body().includes('State & activity history'));
  run('press', 'Home'); assert(body().includes('Configuration'));
  tab('Traffic');
  socket = createConnection({ host: '127.0.0.1', port: saved.port });
  await new Promise((resolve, reject) => { socket.once('connect', resolve); socket.once('error', reject); });
  let received = 0; socket.on('data', bytes => received += bytes.length);
  for (let n = 0; n < 50; n++) { socket.write(Buffer.alloc(65536, n)); await wait(100); }
  const until = Date.now() + 5000;
  while (received < 50 * 65536) { assert(Date.now() < until); await wait(50); }
  const counters = evaluate(`(async()=>await window.__TAURI_INTERNALS__.invoke('get_tunnel_traffic',{tunnelId:${JSON.stringify(original.id)}}))()`);
  assert.equal(counters.sent_bytes, String(50 * 65536)); assert.equal(counters.received_bytes, counters.sent_bytes);
  assert(evaluate(`document.querySelector('[data-series="upload"]').getAttribute('d')`).includes('L'));
  assert(evaluate(`document.querySelector('[data-series="download"]').getAttribute('d')`).includes('L'));
  await waitText('3276800 bytes');
  assert(evaluate(`document.querySelector('[data-metric="upload"]').title`).includes('bytes/s'));
  assert.equal(evaluate('document.documentElement.scrollWidth > window.innerWidth'), false);
  run('screenshot', '--full', '--screenshot-dir', resolve('.upstream/ui-v022-evidence'));
  click('View exact samples'); await waitText('Recent traffic samples');
  click('Close samples');
  click('Restart'); await waitText('Taking first sample');
  await wait(1300);
  const restarted = evaluate(`(async()=>await window.__TAURI_INTERNALS__.invoke('get_tunnel_traffic',{tunnelId:${JSON.stringify(original.id)}}))()`);
  assert.notEqual(restarted.attachment, counters.attachment); assert.equal(restarted.sent_bytes, '0');
  click('Stop'); await waitText('No active forward');
  assert.equal(evaluate(`document.querySelector('[data-metric="upload"]').textContent`), '—');
  assert(body().includes('Stopped'));
  // Real bind conflict must show an authoritative error and history, not Running.
  const occupied = createServer(); await new Promise(r => occupied.listen(saved.port, '127.0.0.1', r));
  try { click('Start'); await waitText('Error'); tab('Overview'); assert(evaluate("document.querySelector('.error-banner')?.textContent.length > 0")); }
  finally { await new Promise(r => occupied.close(r)); }
  click('← All tunnels');
  assert.equal(evaluate('document.activeElement.getAttribute("aria-label")'), `View details for ${saved.name}`);
  await wait(300); const calls = evaluate('window.detailTestCalls'); await wait(2200);
  assert.equal(evaluate('window.detailTestCalls'), calls, 'Leaving details must stop polling');
  click(`View details for ${saved.name}`); tab('History'); await waitText('State & activity history');
  assert(body().includes('Running')); assert(body().includes('Stopped'));
  click('SSH Hosts'); await wait(300); const left = evaluate('window.detailTestCalls'); await wait(1300);
  assert.equal(evaluate('window.detailTestCalls'), left, 'Changing navigation must stop polling');
  evaluate(`(async()=>{const i=window.__TAURI_INTERNALS__.invoke; await i('save_tunnel',{tunnel:${JSON.stringify(original)}}); await i('start_tunnel',{tunnelId:${JSON.stringify(original.id)}});return true;})()`);
  run('reload'); await waitText(saved.name);
  assert(!run('errors').trim());
  console.log('PASS: real detail tabs/arrows/Home/End/focus, no outer page scroll, bidirectional exact counters and charts, restart/reset, stop, bind-error history, retained activity, polling cleanup.');
} finally {
  socket?.destroy(); echo.close();
}
