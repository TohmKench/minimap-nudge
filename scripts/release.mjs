#!/usr/bin/env node
// Bumps the version everywhere, commits, tags and pushes. GitHub Actions then
// builds, signs and publishes the release, and installed apps offer the update.
//
//   npm run release -- 2.2.0 "Shorter pings; new bell sound"
//   npm run release -- 2.2.0 "notes" --no-push     (commit + tag only)
import { execSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';

const [version, notes, ...flags] = process.argv.slice(2);
const push = !flags.includes('--no-push');
const sh = (c) => execSync(c, { stdio: ['ignore', 'pipe', 'inherit'] }).toString().trim();
const fail = (m) => { console.error(`release: ${m}`); process.exit(1); };

if (!/^\d+\.\d+\.\d+$/.test(version || '')) fail('usage: npm run release -- <x.y.z> "<release notes>"');
if (!notes || !notes.trim()) fail('release notes are required (they show in the app\'s update banner)');

const conf = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8'));
const cmp = (a, b) => { const x = a.split('.').map(Number), y = b.split('.').map(Number); for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] - y[i]; return 0; };
if (cmp(version, conf.version) <= 0) fail(`${version} must be higher than the current ${conf.version}`);
if (sh('git status --porcelain')) fail('working tree has uncommitted changes — commit or stash them first');
if (sh(`git tag -l v${version}`)) fail(`tag v${version} already exists`);

const edit = (file, fn) => writeFileSync(file, fn(readFileSync(file, 'utf8')));
edit('src-tauri/tauri.conf.json', (s) => s.replace(/("version":\s*")[^"]+(")/, `$1${version}$2`));
edit('package.json', (s) => s.replace(/("version":\s*")[^"]+(")/, `$1${version}$2`));
edit('package-lock.json', (s) => {
  const j = JSON.parse(s);
  j.version = version;
  if (j.packages && j.packages['']) j.packages[''].version = version;
  return JSON.stringify(j, null, 2) + '\n';
});
edit('src-tauri/Cargo.toml', (s) => s.replace(/^version = "[^"]+"/m, `version = "${version}"`));
edit('src-tauri/Cargo.lock', (s) => s.replace(/(name = "minimap-nudge"\nversion = ")[^"]+(")/, `$1${version}$2`));

sh('git add src-tauri/tauri.conf.json package.json package-lock.json src-tauri/Cargo.toml src-tauri/Cargo.lock');
sh(`git commit -m "Release v${version}"`);
writeFileSync('.git/RELEASE_NOTES', notes.trim() + '\n');
sh(`git tag -a v${version} -F .git/RELEASE_NOTES`);
console.log(`Committed and tagged v${version}.`);

if (push) {
  execSync('git push origin HEAD --follow-tags', { stdio: 'inherit' });
  console.log(`Pushed. GitHub Actions is building v${version}; installed apps will offer it once the release is published (~10 min).`);
} else {
  console.log('Not pushed. Run: git push origin HEAD --follow-tags');
}
