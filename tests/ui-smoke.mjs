// Runs against an isolated, real Tauri WebView opened with CDP enabled.
// No browser mock backend and no outbound SSH connections are used here.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
const binary = process.env.TUNNELWARDEN_AGENT_BROWSER ?? resolve('.upstream/tooling/node_modules/agent-browser/bin/agent-browser-win32-x64.exe');
const args = ['--session', 'tunnelwarden-native', '--cdp', process.env.TUNNELWARDEN_CDP ?? '9223'];
function run(...command) {
  const r = spawnSync(binary, [...args, ...command], { encoding: 'utf8', timeout: 30000, windowsHide: true });
  assert.equal(r.status, 0, `${command.join(' ')}: ${r.stderr || r.stdout}`);
  assert(!/^✗/m.test(r.stdout), r.stdout);
  return r.stdout;
}
const click = name => run('find', 'role', 'button', 'click', '--name', name, '--exact');
const fill = (label, value) => run('find', 'label', label, 'fill', String(value));
const snapshot = () => run('snapshot', '-i');
const name = `UI acceptance ${Date.now()}`;
run('eval', `(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const hosts = (await invoke('list_hosts')).filter(h => h.hostname === 'fixture.invalid' && /^UI acceptance [0-9]+$/.test(h.name));
  for (const h of hosts) {
    for (const t of await invoke('list_tunnels')) if (t.host_id === h.id) await invoke('delete_tunnel', {tunnelId:t.id});
    await invoke('delete_host', {hostId:h.id});
  }
})()`);
run('reload');

click('SSH Hosts'); snapshot(); click('+ New host');
assert(snapshot().includes('SSH port'));
fill('Name', name); fill('Hostname / IP', 'fixture.invalid'); fill('SSH port', 2222); fill('Username', 'tester');
click('Save host');
assert(snapshot().includes(name));
click('Tunnels'); snapshot(); click('+ New tunnel');
fill('Name', name);
click('Remote (-R)');
let s = snapshot();
assert(s.includes('Remote listen port')); assert(s.includes('Local target port')); assert(!s.includes('Remote target port'));
fill('Remote listen port', 48873); fill('Local target host', '127.0.0.1'); fill('Local target port', 3000);
click('Dynamic (-D)'); s = snapshot();
assert(s.includes('Local SOCKS port')); assert(!s.includes('target port')); assert(!s.includes('target host'));
click('Remote (-R)'); click('Save tunnel');
assert(run('get', 'text', 'body').includes('fixture.invalid:2222'));
assert(run('get', 'text', 'body').includes('127.0.0.1:3000'));
run('screenshot', '--screenshot-dir', resolve('.upstream/ui-evidence'));
click('Edit'); assert(snapshot().includes('Edit tunnel')); click('Cancel');
click('Delete'); assert(snapshot().includes('Delete tunnel?')); click('Cancel');
click('Settings'); s = snapshot(); assert(s.includes('Reconnect after a connection drops'));
click('Save settings'); assert(run('get', 'text', 'body').includes('Settings saved'));
click('About'); assert(run('get', 'text', 'body').includes('16ad2af83b424780ff60b6420a6c20a3b30c7143'));
click('Tunnels'); snapshot(); click('Delete'); click('Delete');
click('SSH Hosts'); snapshot(); click('Delete'); click('Delete');
const errors = run('errors'); assert(!errors.trim(), errors);
console.log('PASS: real Tauri CRUD, mode-specific fields, independent SSH port, edit/delete, settings, attribution, no browser errors.');
