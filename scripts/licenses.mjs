// Collect the resolved dependency licenses verbatim for binary redistribution.
import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync, existsSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
const sections = ['TunnelWarden dependency licenses\nGenerated from Cargo.lock and package-lock.json.\nApplication: AGPL-3.0-or-later; see LICENSE and NOTICE.ORYXIS.'];
function append(name, version, license, root, explicit) {
  const files = new Set(readdirSync(root).filter(f => /^(licen[cs]e|copying|notice|copyright)([._-]|$)/i.test(f)));
  if (explicit && existsSync(join(root, explicit))) files.add(explicit);
  let text = `\n${'='.repeat(72)}\n${name} ${version}\nLicense: ${typeof license === 'string' ? license : JSON.stringify(license) || 'See source'}\n`;
  for (const file of files) {
    try { text += `\n--- ${file} ---\n${readFileSync(join(root, file), 'utf8')}\n`; }
    catch (e) { if (e.code !== 'EISDIR') throw e; }
  }
  sections.push(text);
}
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--locked', '--format-version', '1'], { maxBuffer: 64 * 1024 * 1024 }));
for (const p of metadata.packages) append(p.name, p.version, p.license, dirname(p.manifest_path), p.license_file);
const lock = JSON.parse(readFileSync('package-lock.json', 'utf8'));
for (const path of Object.keys(lock.packages).filter(p => p.startsWith('node_modules/'))) {
  if (!existsSync(join(path, 'package.json'))) continue; // Optional other-platform packages.
  const p = JSON.parse(readFileSync(join(path, 'package.json'), 'utf8'));
  append(p.name, p.version, p.license, path);
}
writeFileSync('THIRD_PARTY_LICENSES.txt', sections.join('\n'));
console.log(`Collected ${sections.length - 1} resolved dependency notices.`);
