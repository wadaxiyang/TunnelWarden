// Real Tauri + loopback SSH acceptance. Requires tests/ssh_fixture.py running,
// stdout at .upstream/ui-fixture.json; only disposable smoke-test profiles.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createConnection, createServer } from 'node:net';
const binary = process.env.TUNNELWARDEN_AGENT_BROWSER ?? resolve('.upstream/tooling/node_modules/agent-browser/bin/agent-browser-win32-x64.exe');
function run(...command) {
  const r = spawnSync(binary, ['--session', 'tunnelwarden-native', '--cdp', '9223', ...command], { encoding: 'utf8', timeout: 30000, windowsHide: true });
  assert.equal(r.status, 0, r.stderr || r.stdout); assert(!/^✗/m.test(r.stdout), r.stdout); return r.stdout;
}
const click = name => run('find', 'role', 'button', 'click', '--name', name, '--exact');
const fill = (label, value) => {
  const line = snapshot().split('\n').find(l => /- (textbox|spinbutton) /.test(l) && l.includes(`"${label}`));
  const ref = line?.match(/ref=([^\]]+)\]/)?.[1]; assert(ref, `Missing input: ${label}`);
  return run('fill', `@${ref}`, String(value));
};
const snapshot = () => run('snapshot', '-i');
const body = () => run('get', 'text', 'body');
async function waitText(text) {
  const deadline = Date.now() + 15000;
  while (!body().includes(text)) { assert(Date.now() < deadline, `Missing text: ${text}`); await new Promise(r => setTimeout(r, 200)); }
}
async function banner(port) {
  await new Promise((resolve, reject) => {
    const socket = createConnection({ host: '127.0.0.1', port }); let data = '';
    socket.setTimeout(5000, () => socket.destroy(new Error('SSH banner timeout')));
    socket.on('data', b => { data += b; if (data.startsWith('SSH-2.0')) { socket.destroy(); resolve(); } });
    socket.on('error', reject);
  });
}
const statePath = resolve('.upstream/ui-v02-state.json');
if (process.argv[2] === 'reopen') {
  const state = JSON.parse(readFileSync(statePath, 'utf8'));
  await waitText('running'); assert(body().includes(state.name)); await banner(state.port);
  click('SSH Hosts'); snapshot(); click('Test connection'); await waitText('SSH connection and authentication succeeded');
  assert(!body().includes('Private-key passphrase'));
  click('Logs'); snapshot(); assert(body().includes('Export error logs'));
  console.log('PASS: recreated UI reads live runtime and in-memory unlock survives WebView destruction.');
} else if (process.argv[2] === 'cleanup') {
  run('eval', `(async () => { const i=window.__TAURI_INTERNALS__.invoke; for(const h of await i('list_hosts')) if(h.name.startsWith('v0.2 acceptance ')) { for(const t of await i('list_tunnels')) if(t.host_id===h.id) await i('delete_tunnel',{tunnelId:t.id}); await i('delete_host',{hostId:h.id}); } })()`);
  console.log('Disposable UI profiles cleaned.');
} else {
  const readyDeadline = Date.now() + 10000;
  while (!readFileSync('.upstream/ui-fixture.json', 'utf8').trim()) { assert(Date.now() < readyDeadline, 'SSH fixture did not start'); await new Promise(r => setTimeout(r, 100)); }
  const fixture = JSON.parse(readFileSync('.upstream/ui-fixture.json', 'utf8').replace(/^\uFEFF/, ''));
  const keyPath = resolve('.upstream/ui-encrypted.key'); writeFileSync(keyPath, fixture.encrypted_private_key);
  const server = createServer(); await new Promise(r => server.listen(0, '127.0.0.1', r)); const port = server.address().port; await new Promise(r => server.close(r));
  const name = `v0.2 acceptance ${Date.now()}`;
  click('SSH Hosts'); snapshot(); click('+ New host');
  fill('Name', name); fill('Hostname / IP', '127.0.0.1'); fill('SSH port', fixture.port); fill('Username', 'tester');
  const authRef = snapshot().match(/combobox "Authentication method" \[[^\]]*ref=([^\]]+)\]/)?.[1];
  assert(authRef); run('select', `@${authRef}`, 'Private key file'); snapshot(); fill('Private key file', keyPath); click('Save host');
  await waitText('Test connection');
  snapshot(); click('Test connection'); await waitText('Unlock private key');
  fill('Private-key passphrase', 'wrong-test-passphrase'); click('Unlock and connect'); await waitText('Incorrect passphrase');
  fill('Private-key passphrase', 'fixture-key-passphrase'); click('Unlock and connect'); await waitText('Verify SSH host key');
  assert(body().includes(fixture.fingerprint)); click('Trust and connect'); await waitText('SSH connection and authentication succeeded');
  click('Tunnels'); snapshot(); click('+ New tunnel'); fill('Name', name);
  click('Remote (-R)'); snapshot(); fill('Remote listen address', '0.0.0.0'); assert(body().includes('GatewayPorts'));
  click('Local (-L)'); snapshot(); fill('Local listen address', '127.0.0.1'); fill('Local listen port', port); fill('Remote target host', '127.0.0.1'); fill('Remote target port', fixture.port);
  click('Save tunnel'); snapshot(); click('Start'); await waitText('running'); await banner(port);
  writeFileSync(statePath, JSON.stringify({name, port}));
  run('screenshot', '--screenshot-dir', resolve('.upstream/ui-evidence'));
  assert(!run('errors').trim());
  console.log('PASS: actual host-test UI, incorrect/correct key passphrase, verified fingerprint, remote guidance, live tunnel.');
}
